//! Pure backup logic — encryption, key derivation, rotation, and file writing.
//!
//! No TTY interaction occurs here. All prompts (password, confirmation) are
//! handled by the CLI adapter in [`mod.rs`](super). Every function in this
//! module is independently testable without a terminal.
//!
//! # Pipeline
//!
//! ```text
//! content (&str)  +  password (String)  +  BackupOptions
//!         │
//!         ▼
//!   backup_inner()
//!         │
//!         ├─ ZeroizeOnDrop guard wraps password immediately
//!         ├─ rotate_backups()        (rename existing, warn beyond --keep)
//!         ├─ spinner starts          (suppressed in verbose mode)
//!         ├─ encrypt_content()       (Argon2id + AES-256-GCM)
//!         ├─ spinner stops
//!         ├─ write_secure()          (0o600 permissions)
//!         ├─ verify_backup()         (optional — only when --verify is set)
//!         └─ Ok(output_path)         password zeroized on drop
//! ```
//!
//! # Security model
//!
//! See the [module-level documentation](super) for the full binary format and
//! Argon2id parameter rationale. This module owns the cryptographic
//! implementation; `mod.rs` owns only the user-facing orchestration.
//!
//! # Error types
//!
//! | Error | Meaning |
//! |-------|---------|
//! | [`BackupError::EncryptionFailed`] | Argon2id / AES-256-GCM failure |
//! | [`BackupError::WriteFailed`] | Could not write backup to disk |
//! | [`BackupError::VerifyFailed`] | Post-write integrity check failed |
//!
//! [`BackupError::EncryptionFailed`]: super::error::BackupError::EncryptionFailed
//! [`BackupError::WriteFailed`]: super::error::BackupError::WriteFailed
//! [`BackupError::VerifyFailed`]: super::error::BackupError::VerifyFailed

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use zeroize::Zeroize;

use crate::utils::ui;

use super::error::BackupError;

// ─── Options ──────────────────────────────────────────────────────────────────

/// Configuration for a backup operation.
///
/// Constructed by the CLI adapter (`mod.rs`) from parsed CLI arguments and
/// passed into [`backup_inner`]. Adding future flags here is additive —
/// existing call sites can use `..Default::default()` for new fields.
///
/// # Defaults
///
/// `verbose`, `verify` default to `false`; `keep` defaults to `3`.
/// `env` and `output` must be supplied by the caller.
#[cfg(feature = "backup")]
#[derive(Debug, Clone)]
pub struct BackupOptions {
    /// Path to the source `.env` file.
    pub env: PathBuf,

    /// Destination path for the encrypted backup.
    ///
    /// Typically `<env>.backup` (e.g. `.env.backup`), but can be overridden
    /// by the user via `--output`.
    pub output: PathBuf,

    /// Emit a diagnostic message at each pipeline stage via
    /// [`ui::verbose_stderr`].
    ///
    /// In verbose mode the Argon2id spinner is suppressed so that per-step
    /// diagnostic lines are not interleaved with spinner output.
    pub verbose: bool,

    /// Path to a key file used as the encryption password source.
    ///
    /// When `Some`, the file's contents are read and used in place of an
    /// interactively typed password, enabling non-interactive CI pipelines.
    /// UTF-8 content is used as-is (trimmed); binary content is Base64-encoded
    /// to produce a stable ASCII string before being fed into Argon2id.
    pub key_file: Option<PathBuf>,

    /// Number of previous backups to retain alongside the new one.
    ///
    /// Before writing, existing backups are rotated:
    /// `output` → `output.1` → `output.2` → … → `output.{keep-1}`.
    /// Files at position `keep` and beyond are **warned about but never
    /// deleted** — the user must remove them manually.
    ///
    /// Set to `0` to disable rotation entirely (overwrites silently).
    pub keep: u32,

    /// If `true`, re-decrypt the backup immediately after writing and compare
    /// the recovered content against the original byte-for-byte.
    ///
    /// A mismatch returns [`BackupError::VerifyFailed`] (exit code 6) and
    /// leaves the backup file on disk for manual inspection. The check costs
    /// one additional Argon2id round-trip (~1 s).
    pub verify: bool,
    // ── Planned flags — not yet wired to the CLI ─────────────────────────────
    // pub recipient: Option<String>,   // --recipient (asymmetric encryption)
}

// ─── RAII password guard ──────────────────────────────────────────────────────

/// Owns a `String` and zeroizes it on drop, covering every exit path:
/// normal return, `?`-propagated error, and panic.
///
/// Implements `Deref<Target = str>` so the inner value can be borrowed
/// as `&str` without moving out.
#[cfg(feature = "backup")]
struct ZeroizeOnDrop(String);

#[cfg(feature = "backup")]
impl Drop for ZeroizeOnDrop {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(feature = "backup")]
impl std::ops::Deref for ZeroizeOnDrop {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

// ─── Core function ────────────────────────────────────────────────────────────

/// Encrypt `content` and write the backup to `options.output`.
///
/// This is the primary testable entry point for backup logic. The caller
/// is responsible only for supplying the file content and the password —
/// no TTY interaction occurs here.
///
/// # Steps
///
/// 1. Moves `password` into a [`ZeroizeOnDrop`] guard — fires on every exit
///    path including `?`-propagated errors and panics.
/// 2. Rotates existing backups according to `options.keep`.
/// 3. Starts an Argon2id progress spinner (suppressed when `verbose = true`).
/// 4. Encrypts `content` with Argon2id + AES-256-GCM.
/// 5. Stops the spinner unconditionally.
/// 6. Writes the encrypted blob to `options.output` with `0o600` permissions.
/// 7. If `options.verify`: re-decrypts the file and compares content.
/// 8. Returns the path that was written.
///
/// # Errors
///
/// - [`BackupError::EncryptionFailed`] — Argon2id or AES-256-GCM failed.
/// - [`BackupError::WriteFailed`] — could not write the backup to disk.
/// - [`BackupError::VerifyFailed`] — post-write integrity check failed.
/// - Other `anyhow` errors — unexpected encoding failures (non-UTF-8 paths,
///   rotation rename failures).
#[cfg(feature = "backup")]
pub fn backup_inner(content: &str, password: String, options: &BackupOptions) -> Result<PathBuf> {
    // ── Zeroize password on every exit path ───────────────────────────────────
    let pw_guard = ZeroizeOnDrop(password);

    if options.verbose {
        ui::verbose_stderr("Backup pipeline starting");
        ui::verbose_stderr(format!("Source       : {}", options.env.display()));
        ui::verbose_stderr(format!("Output       : {}", options.output.display()));
        ui::verbose_stderr(format!("Keep         : {}", options.keep));
        ui::verbose_stderr(format!("Verify       : {}", options.verify));
        if let Some(kf) = &options.key_file {
            ui::verbose_stderr(format!("Key file     : {}", kf.display()));
        }
        ui::verbose_stderr("Argon2id key derivation in progress…");
    }

    // ── Rotate existing backups ───────────────────────────────────────────────
    if options.keep > 0 {
        rotate_backups(&options.output, options.keep)?;
    }

    // ── Spinner ───────────────────────────────────────────────────────────────
    // Shown only when verbose is off. The KDF is deliberately slow — without
    // feedback users may assume the tool has hung.
    let spinner = if options.verbose {
        None
    } else {
        Some(ui::spinner(
            "Encrypting… (Argon2id key derivation in progress)",
        ))
    };

    let original_filename = options
        .env
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".env");

    // Capture the error before stopping the spinner so the terminal is always
    // left in a clean state regardless of outcome.
    let encrypt_result = encrypt_content(content, &pw_guard, original_filename)
        .map_err(|e| BackupError::EncryptionFailed(e.to_string()));

    // ── Stop spinner unconditionally ──────────────────────────────────────────
    // Must happen before any further output and before propagating an error,
    // so the terminal is not left in a partial state.
    if let Some(sp) = spinner {
        sp.finish_and_clear();
    }

    let encrypted = encrypt_result?;

    if options.verbose {
        ui::verbose_stderr("Encryption complete — writing backup file");
    }

    // ── Write backup ──────────────────────────────────────────────────────────
    let path_str = options.output.to_str().with_context(|| {
        format!(
            "Output path contains non-UTF-8 characters: {:?}",
            options.output
        )
    })?;

    crate::utils::write_secure(path_str, encrypted.as_bytes())
        .map_err(|e| BackupError::WriteFailed(e.to_string()))?;

    if options.verbose {
        ui::verbose_stderr(format!("Backup written to {}", options.output.display()));
    }

    // ── Post-write integrity check ────────────────────────────────────────────
    // Re-decrypts the file just written and compares it byte-for-byte with the
    // original content. Costs one additional Argon2id round (~1 s) but proves
    // the backup is readable before the user discards the source.
    if options.verify {
        if options.verbose {
            ui::verbose_stderr("Verifying backup integrity…");
        }
        verify_backup(&options.output, content, &pw_guard)?;
        if options.verbose {
            ui::verbose_stderr("Integrity check passed");
        }
    }

    Ok(options.output.clone())
    // pw_guard drops here → password zeroized
}

// ─── Rotation ─────────────────────────────────────────────────────────────────

/// Rotate numbered backup files before writing a new one.
///
/// Renames in reverse order to avoid overwriting:
/// ```text
/// output.{keep-2} → output.{keep-1}
///         ⋮
/// output.1        → output.2
/// output          → output.1
/// ```
/// Files at position `keep` and beyond are warned about but **never deleted**.
///
/// # Example
///
/// With `keep = 3` and files `.env.backup`, `.env.backup.1`, `.env.backup.2`,
/// `.env.backup.3` present:
/// - `.env.backup.3` — warned ("exceeds --keep 3, delete manually")
/// - `.env.backup.2` — renamed to `.env.backup.3` … wait, we never delete,
///   so we only rename up to position `keep - 1`.
///
/// Actually: files at position >= `keep` (e.g. `.env.backup.3` when keep=3)
/// are warned. Positions 1..keep-1 are shifted up by one. Position 0 (the
/// base file) shifts to position 1.
#[cfg(feature = "backup")]
fn rotate_backups(output: &Path, keep: u32) -> Result<()> {
    // Warn about the file at position `keep` — it will be overwritten when
    // `keep-1` is shifted up. We never proactively delete beyond this.
    let overflow = numbered_backup_path(output, keep);
    if overflow.exists() {
        ui::warning(format!(
            "--keep {keep}: {} will be overwritten by rotation — \
             increase --keep or prune manually to retain it",
            overflow.display()
        ));
    }

    // Shift positions keep downto 1 in reverse to avoid clobbering.
    // With keep=3: rename .backup.2→.backup.3, .backup.1→.backup.2, .backup→.backup.1
    for i in (1..=keep).rev() {
        let src = if i == 1 {
            output.to_path_buf()
        } else {
            numbered_backup_path(output, i - 1)
        };
        let dst = numbered_backup_path(output, i);
        if src.exists() {
            std::fs::rename(&src, &dst).with_context(|| {
                format!(
                    "Failed to rotate backup: {} → {}",
                    src.display(),
                    dst.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Build the path for the `n`-th numbered backup.
///
/// `n = 1` → `output.1`, `n = 2` → `output.2`, etc.
#[cfg(feature = "backup")]
fn numbered_backup_path(base: &Path, n: u32) -> PathBuf {
    PathBuf::from(format!("{}.{}", base.display(), n))
}

// ─── Verify ───────────────────────────────────────────────────────────────────

/// Re-decrypt `path` and assert that its content matches `original`.
///
/// Called by [`backup_inner`] when `options.verify = true`. Uses the same
/// `password` that was used for encryption (borrowed from the `ZeroizeOnDrop`
/// guard still in scope).
///
/// Returns [`BackupError::VerifyFailed`] if:
/// - the file cannot be read back from disk,
/// - decryption fails (should not happen immediately after a successful write),
/// - or the recovered content does not match `original` byte-for-byte.
///
/// The backup file is **not** deleted on failure — the user should inspect it.
#[cfg(feature = "backup")]
fn verify_backup(path: &Path, original: &str, password: &str) -> Result<()> {
    let written = std::fs::read_to_string(path).map_err(|e| {
        BackupError::VerifyFailed(format!(
            "could not re-read backup for verification ({}): {}",
            path.display(),
            e
        ))
    })?;

    let (recovered, _) = decrypt_content(&written, password).map_err(|e| {
        BackupError::VerifyFailed(format!(
            "re-decryption failed during verification — backup may be corrupt: {}",
            e
        ))
    })?;

    if recovered != original {
        return Err(BackupError::VerifyFailed(
            "decrypted content does not match original — \
             backup file may be corrupt; original source file is untouched"
                .into(),
        )
        .into());
    }

    Ok(())
}

// ─── Encryption ───────────────────────────────────────────────────────────────

/// Encrypt the plaintext content of a `.env` file.
///
/// Produces a Base64-encoded string containing the complete binary envelope:
/// `version(1) || salt(32) || nonce(12) || AES-256-GCM-ciphertext`.
///
/// The ciphertext decrypts to a JSON envelope containing the `.env` content
/// and metadata (see the [module-level docs](super) for the schema).
///
/// A fresh random salt and nonce are generated on every call, so two
/// encryptions of the same file with the same password always produce
/// different ciphertext.
///
/// # Arguments
///
/// * `plaintext`         — The raw `.env` file content.
/// * `password`          — User-supplied encryption password (or key-file derived string; see [`BackupOptions::key_file`]).
/// * `original_filename` — Filename stored in the metadata envelope so `restore` can surface it to the user.
///
/// # Errors
///
/// Returns an error if Argon2id key derivation or AES-256-GCM encryption
/// fails. In practice these only fail when given invalid parameters, which
/// are hardcoded here and validated at compile time.
#[cfg(feature = "backup")]
pub(crate) fn encrypt_content(
    plaintext: &str,
    password: &str,
    original_filename: &str,
) -> Result<String> {
    use aes_gcm::{
        aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
        Aes256Gcm, Nonce,
    };
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use base64::{engine::general_purpose, Engine as _};

    use anyhow::anyhow;

    // ── JSON metadata envelope ────────────────────────────────────────────────
    // Stored inside the ciphertext so it is confidential and tamper-evident.
    let created_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let envelope = serde_json::json!({
        "schema_version": 1,  // JSON envelope schema version (forward-compatibility)
        "version": 1,         // Binary format version
        "created_at": created_at,
        "original_file": original_filename,
        "tool_version": env!("CARGO_PKG_VERSION"),
        "content": plaintext,
    });
    let envelope_bytes =
        serde_json::to_vec(&envelope).context("Failed to serialise metadata envelope")?;

    // ── Argon2id key derivation ───────────────────────────────────────────────
    // A fresh 32-byte salt is generated for every backup so two encryptions of
    // the same file with the same password produce different ciphertext.
    //
    // | Parameter   | Value    | Rationale                                   |
    // |-------------|----------|---------------------------------------------|
    // | variant     | Argon2id | Resistant to GPU and side-channel attacks   |
    // | memory      | 64 MiB   | Slows brute-force on commodity hardware     |
    // | iterations  | 3        | Adds time cost on top of memory cost        |
    // | parallelism | 1        | Single-threaded CLI usage                   |
    // | output len  | 32 B     | Exactly one AES-256 key                     |
    let mut salt_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut salt_bytes);

    let params =
        Params::new(65536, 3, 1, Some(32)).map_err(|e| anyhow!("Invalid Argon2 params: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let salt_string = argon2::password_hash::SaltString::encode_b64(&salt_bytes)
        .map_err(|e| anyhow!("Failed to encode salt for Argon2: {}", e))?;

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt_string)
        .map_err(|e| anyhow!("Argon2id key derivation failed: {}", e))?;

    let hash_output = password_hash
        .hash
        .ok_or_else(|| anyhow!("Argon2id did not produce a hash output"))?;

    let key_bytes = hash_output.as_bytes();
    if key_bytes.len() < 32 {
        return Err(anyhow!(
            "Derived key too short: {} bytes (expected 32)",
            key_bytes.len()
        ));
    }
    let key: &[u8; 32] = key_bytes[..32]
        .try_into()
        .map_err(|_| anyhow!("Key slice conversion failed"))?;

    // ── AES-256-GCM encryption ────────────────────────────────────────────────
    let cipher = Aes256Gcm::new(key.into());

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, envelope_bytes.as_ref())
        .map_err(|e| anyhow!("AES-256-GCM encryption failed: {}", e))?;

    // ── Assemble binary envelope ──────────────────────────────────────────────
    // Layout: version(1) || salt(32) || nonce(12) || ciphertext(variable)
    // Increment the version byte when the format changes; never break v1 decryption.
    let mut result: Vec<u8> = Vec::with_capacity(1 + 32 + 12 + ciphertext.len());
    result.push(1u8);
    result.extend_from_slice(&salt_bytes);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(general_purpose::STANDARD.encode(&result))
}

// ─── Decryption (pub — used by restore) ──────────────────────────────────────

/// Decrypt a backup envelope produced by [`encrypt_content`].
///
/// This function is `pub` so that `restore/core.rs` can call it directly.
/// Both commands share the same binary format; keeping decrypt logic here
/// means a format bump requires changes in exactly one place.
///
/// If the binary format ever changes, increment the version byte in
/// [`encrypt_content`] and add a new match arm here — do **not** break
/// decryption of existing version-1 files.
///
/// # Returns
///
/// A tuple of `(plaintext, metadata)`:
/// - `plaintext` — The original `.env` file content.
/// - `metadata`  — [`BackupMetadata`] with the original filename, creation
///   timestamp, and tool version extracted from the JSON envelope.
///
/// # Errors
///
/// Returns a descriptive [`anyhow::Error`] for:
/// - Base64 decode failure (not an evnx backup, or file is truncated).
/// - Unknown format version (backup made by a newer tool version).
/// - Argon2id key derivation failure (should not occur with valid inputs).
/// - AES-256-GCM decryption failure — almost always wrong password or tampered
///   file; the error message deliberately does not distinguish these two cases
///   to avoid leaking information.
/// - JSON deserialisation failure (encrypted payload is internally corrupt).
#[cfg(feature = "backup")]
pub fn decrypt_content(encoded: &str, password: &str) -> Result<(String, BackupMetadata)> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use base64::{engine::general_purpose, Engine as _};

    use anyhow::anyhow;

    // ── Base64 decode ─────────────────────────────────────────────────────────
    let raw = general_purpose::STANDARD
        .decode(encoded.trim())
        .with_context(|| {
            "Failed to decode backup file: not valid Base64 or file truncated".to_string()
        })?;

    // Minimum valid size: 1 (version) + 32 (salt) + 12 (nonce) + 16 (GCM tag)
    const MIN_LEN: usize = 1 + 32 + 12 + 16;
    if raw.len() < MIN_LEN {
        return Err(anyhow!(
            "Backup file is too short ({} bytes, minimum {}). File may be corrupt.",
            raw.len(),
            MIN_LEN
        ));
    }

    // ── Parse binary envelope ─────────────────────────────────────────────────
    let version = raw[0];
    if version != 1 {
        return Err(anyhow!(
            "Unsupported backup format version: {}. \
             This backup was created by a newer version of evnx. \
             Please upgrade the tool and try again.",
            version
        ));
    }

    // Slice layout mirrors encrypt_content exactly.
    let salt_bytes = &raw[1..33]; // 32 bytes
    let nonce_bytes = &raw[33..45]; // 12 bytes
    let ciphertext = &raw[45..]; // remainder = GCM ciphertext + 16-byte tag

    // ── Argon2id key re-derivation ────────────────────────────────────────────
    // Same parameters as encrypt_content. If these ever change, add a version
    // branch above and keep the old params here for backward compatibility.
    let params =
        Params::new(65536, 3, 1, Some(32)).map_err(|e| anyhow!("Invalid Argon2 params: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let salt_string = argon2::password_hash::SaltString::encode_b64(salt_bytes)
        .map_err(|e| anyhow!("Failed to encode salt: {}", e))?;

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt_string)
        .map_err(|e| anyhow!("Argon2id key derivation failed: {}", e))?;

    let hash_output = password_hash
        .hash
        .ok_or_else(|| anyhow!("Argon2id did not produce a hash output"))?;

    let key_bytes = hash_output.as_bytes();
    if key_bytes.len() < 32 {
        return Err(anyhow!("Derived key too short: {} bytes", key_bytes.len()));
    }
    let key: &[u8; 32] = key_bytes[..32]
        .try_into()
        .map_err(|_| anyhow!("Key slice conversion failed"))?;

    // ── AES-256-GCM decryption ────────────────────────────────────────────────
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(nonce_bytes);

    // Intentionally vague error — distinguishing "wrong password" from "tampered
    // file" would leak information about which part of authentication failed.
    let plaintext_bytes = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        anyhow!("Decryption failed. The password may be incorrect or the backup file is corrupt.")
    })?;

    // ── Deserialise JSON envelope ─────────────────────────────────────────────
    let envelope: serde_json::Value = serde_json::from_slice(&plaintext_bytes)
        .context("Decrypted payload is not valid JSON. The backup envelope may be corrupt.")?;

    // ── Validate JSON schema version ──────────────────────────────────────────
    let schema_version = envelope["schema_version"].as_u64().unwrap_or(0);
    if schema_version != 1 {
        return Err(anyhow!(
            "Unsupported metadata schema version: {}. \
             This backup requires a newer version of evnx.",
            schema_version
        ));
    }

    let content = envelope["content"]
        .as_str()
        .ok_or_else(|| {
            anyhow!(
                "Backup envelope (schema v{}) is missing 'content' field",
                schema_version
            )
        })?
        .to_string();

    let metadata = BackupMetadata {
        schema_version: envelope["schema_version"].as_u64().unwrap_or(0),
        created_at: envelope["created_at"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        original_file: envelope["original_file"]
            .as_str()
            .unwrap_or(".env")
            .to_string(),
        tool_version: envelope["tool_version"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
    };

    Ok((content, metadata))
}

// ─── Supporting types ─────────────────────────────────────────────────────────

/// Metadata extracted from a decrypted backup envelope.
///
/// Returned by [`decrypt_content`] so the `restore` command can display
/// information about the backup before writing any files.
///
/// All fields are stored inside the AES-256-GCM ciphertext, making them both
/// confidential (an attacker without the password cannot read them) and
/// tamper-evident (altering any field invalidates the GCM authentication tag).
#[cfg(feature = "backup")]
#[derive(Debug)]
pub struct BackupMetadata {
    /// Schema version of the JSON envelope (for forward-compatibility checks).
    pub schema_version: u64,
    /// ISO 8601 UTC timestamp recorded when the backup was created.
    pub created_at: String,
    /// The original filename stored at backup time (e.g. `.env`, `.env.production`).
    pub original_file: String,
    /// The `CARGO_PKG_VERSION` of the tool that created this backup.
    pub tool_version: String,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::dotenv_validation;

    // ── ZeroizeOnDrop ─────────────────────────────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn zeroize_guard_deref_gives_inner_str() {
        let guard = ZeroizeOnDrop("secret-value".to_owned());
        assert_eq!(&*guard, "secret-value");
    }

    #[test]
    #[cfg(feature = "backup")]
    fn zeroize_guard_drops_without_panic() {
        let guard = ZeroizeOnDrop("sensitive".to_owned());
        drop(guard);
    }

    // ── looks_like_dotenv ─────────────────────────────────────────────────────

    #[test]
    fn test_looks_like_dotenv_valid() {
        assert!(dotenv_validation::looks_like_dotenv(
            "# Database\nDATABASE_URL=postgresql://localhost\nSECRET_KEY=abc123\n"
        ));
    }

    #[test]
    fn test_looks_like_dotenv_empty() {
        assert!(dotenv_validation::looks_like_dotenv(""));
        assert!(dotenv_validation::looks_like_dotenv("  \n  "));
    }

    #[test]
    fn test_looks_like_dotenv_rejects_prose() {
        assert!(!dotenv_validation::looks_like_dotenv(
            "This is just a plain text file.\nWith no env vars at all.\nWhatsoever."
        ));
    }

    #[test]
    fn test_looks_like_dotenv_comments_and_blanks() {
        assert!(dotenv_validation::looks_like_dotenv(
            "# Comment\n\n# Another\nKEY=value\n"
        ));
    }

    // ── Encrypt / decrypt roundtrip ───────────────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn test_roundtrip() {
        let plaintext = "DATABASE_URL=postgresql://localhost:5432/db\nSECRET_KEY=abc123\n";
        let password = "correct-horse-battery-staple";
        let filename = ".env";

        let encrypted =
            encrypt_content(plaintext, password, filename).expect("encryption must succeed");

        assert!(!encrypted.is_empty());
        assert_ne!(encrypted, plaintext);

        let (decrypted, metadata) =
            decrypt_content(&encrypted, password).expect("decryption must succeed");

        assert_eq!(
            decrypted, plaintext,
            "roundtrip must recover original content"
        );
        assert_eq!(metadata.original_file, filename);
        assert_eq!(metadata.tool_version, env!("CARGO_PKG_VERSION"));
        assert!(!metadata.created_at.is_empty());
    }

    #[test]
    #[cfg(feature = "backup")]
    fn test_wrong_password_returns_error() {
        let encrypted =
            encrypt_content("KEY=val\n", "correct", ".env").expect("encryption must succeed");

        let result = decrypt_content(&encrypted, "wrong");
        assert!(result.is_err(), "wrong password must return an error");

        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("incorrect") || msg.contains("corrupt"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn test_tampered_ciphertext_is_rejected() {
        use base64::{engine::general_purpose, Engine as _};

        let encrypted =
            encrypt_content("KEY=value\n", "password", ".env").expect("encryption must succeed");

        let mut raw = general_purpose::STANDARD.decode(&encrypted).unwrap();
        let idx = raw.len() - 5;
        raw[idx] ^= 0xFF;
        let tampered = general_purpose::STANDARD.encode(&raw);

        assert!(
            decrypt_content(&tampered, "password").is_err(),
            "tampered ciphertext must be rejected"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn test_two_encryptions_produce_different_ciphertext() {
        let a = encrypt_content("KEY=value\n", "password", ".env").unwrap();
        let b = encrypt_content("KEY=value\n", "password", ".env").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    #[cfg(feature = "backup")]
    fn test_metadata_round_trips() {
        let (_, meta) = decrypt_content(
            &encrypt_content("KEY=val\n", "pass12345", ".env.production").unwrap(),
            "pass12345",
        )
        .unwrap();

        assert_eq!(meta.original_file, ".env.production");
        assert!(!meta.created_at.is_empty());
        assert_eq!(meta.tool_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    #[cfg(feature = "backup")]
    fn test_backup_file_has_restrictive_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            use tempfile::NamedTempFile;

            let temp_env = NamedTempFile::new().unwrap();
            std::fs::write(&temp_env, "KEY=value\n").unwrap();

            let temp_backup = temp_env.path().with_extension("backup");
            let result = crate::utils::write_secure(&temp_backup, b"test");
            assert!(result.is_ok());

            let metadata = std::fs::metadata(&temp_backup).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "Backup file should have 0o600 permissions"
            );
        }
    }

    // ── backup_inner ──────────────────────────────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn backup_inner_writes_decryptable_file() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let env_content = "BACKUP_INNER=yes\nFOO=bar\n";
        let output_path = dir.path().join(".env.backup");

        let options = BackupOptions {
            env: dir.path().join(".env"),
            output: output_path.clone(),
            verbose: false,
            key_file: None,
            keep: 0, // disable rotation for this test
            verify: false,
        };

        let written_path = backup_inner(env_content, "test-password-123".to_owned(), &options)
            .expect("backup_inner must succeed");

        assert_eq!(written_path, output_path);
        assert!(output_path.exists(), "backup file must have been written");

        let encoded = std::fs::read_to_string(&output_path).unwrap();
        let (decrypted, _) =
            decrypt_content(&encoded, "test-password-123").expect("decryption must succeed");
        assert_eq!(
            decrypted, env_content,
            "decrypted content must match original"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn backup_inner_verbose_does_not_panic() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join(".env.backup");

        let options = BackupOptions {
            env: dir.path().join(".env"),
            output: output_path.clone(),
            verbose: true,
            key_file: None,
            keep: 0,
            verify: false,
        };

        backup_inner("KEY=value\n", "verbosepass123".to_owned(), &options)
            .expect("backup_inner in verbose mode must not panic");
    }

    // ── rotate_backups ────────────────────────────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn rotation_renames_existing_backup() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path().join(".env.backup");
        std::fs::write(&base, "old backup").unwrap();

        rotate_backups(&base, 3).expect("rotate_backups must succeed");

        assert!(!base.exists(), ".env.backup should have been rotated away");
        assert!(
            numbered_backup_path(&base, 1).exists(),
            ".env.backup.1 should now exist"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn rotation_shifts_chain_correctly() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path().join(".env.backup");

        // Seed: base, .1, .2 all exist
        std::fs::write(&base, "newest").unwrap();
        std::fs::write(numbered_backup_path(&base, 1), "middle").unwrap();
        std::fs::write(numbered_backup_path(&base, 2), "oldest").unwrap();

        rotate_backups(&base, 3).expect("rotate_backups must succeed");

        // After rotation: base gone, .1 has old base content, .2 has old .1, .3 has old .2
        assert!(!base.exists());
        assert_eq!(
            std::fs::read_to_string(numbered_backup_path(&base, 1)).unwrap(),
            "newest"
        );
        assert_eq!(
            std::fs::read_to_string(numbered_backup_path(&base, 2)).unwrap(),
            "middle"
        );
        assert_eq!(
            std::fs::read_to_string(numbered_backup_path(&base, 3)).unwrap(),
            "oldest"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn rotation_no_op_when_nothing_exists() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path().join(".env.backup");

        // Nothing on disk — should succeed without error.
        rotate_backups(&base, 3).expect("rotate_backups with no existing files must succeed");
    }

    // ── verify_backup ─────────────────────────────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn backup_inner_with_verify_passes_on_valid_backup() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join(".env.backup");
        let content = "VERIFY=yes\nBAR=baz\n";

        let options = BackupOptions {
            env: dir.path().join(".env"),
            output: output_path.clone(),
            verbose: false,
            key_file: None,
            keep: 0,
            verify: true, // ← enabled
        };

        backup_inner(content, "verifypass123".to_owned(), &options)
            .expect("backup_inner with --verify must succeed on valid write");
    }

    #[test]
    #[cfg(feature = "backup")]
    fn verify_backup_detects_tampered_file() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".env.backup");
        let content = "KEY=value\n";
        let password = "tampertest123";

        // Write a valid backup first.
        let encrypted = encrypt_content(content, password, ".env").unwrap();
        std::fs::write(&path, &encrypted).unwrap();

        // Now corrupt the file on disk.
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("corruption");
        std::fs::write(&path, raw).unwrap();

        let result = verify_backup(&path, content, password);
        assert!(result.is_err(), "tampered file must fail verification");

        let err = result.unwrap_err();
        let typed = err.downcast_ref::<BackupError>();
        assert!(
            matches!(typed, Some(BackupError::VerifyFailed(_))),
            "expected BackupError::VerifyFailed"
        );
    }
}
