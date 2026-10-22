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
//!         ├─ password arrives as Zeroizing<String>: cleared on every exit path
//!         ├─ spinner starts          (suppressed in verbose mode)
//!         ├─ encrypt_content()       (Argon2id + AES-256-GCM)
//!         ├─ spinner stops
//!         ├─ safe_write::stage_private()  (temp file beside the output, 0o600)
//!         ├─ rotate_backups()        (only now: rename existing, warn beyond --keep;
//!         │                            through a symlink, the target's copies rotate)
//!         ├─ Staged::commit()        (rename into place — DEC-B7, I-150)
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
//! | [`DecryptError`] | Why a backup could not be opened (I-149), for `restore` |
//!
//! [`check_output`] runs before any of this, in the CLI adapter: it refuses an
//! output the write or the rotation would destroy (I-146, DEC-B3).
//!
//! [`BackupError::EncryptionFailed`]: super::error::BackupError::EncryptionFailed
//! [`BackupError::WriteFailed`]: super::error::BackupError::WriteFailed
//! [`BackupError::VerifyFailed`]: super::error::BackupError::VerifyFailed

#[cfg(feature = "backup")]
use std::path::{Path, PathBuf};

#[cfg(feature = "backup")]
use anyhow::{Context, Result};
#[cfg(feature = "backup")]
use zeroize::Zeroizing;

#[cfg(feature = "backup")]
use crate::utils::ui;

#[cfg(feature = "backup")]
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
    /// one additional Argon2id round: a fraction of a second to about a second,
    /// depending on the machine (D-103).
    pub verify: bool,
    // ── Planned flags — not yet wired to the CLI ─────────────────────────────
    // pub recipient: Option<String>,   // --recipient (asymmetric encryption)
}

// ─── Core function ────────────────────────────────────────────────────────────

/// Encrypt `content` and write the backup to `options.output`.
///
/// This is the primary testable entry point for backup logic. The caller
/// is responsible only for supplying the file content and the password —
/// no TTY interaction occurs here. The caller has already run
/// [`check_output`].
///
/// # Steps
///
/// 1. Starts an Argon2id progress spinner (suppressed when `verbose = true`).
/// 2. Encrypts `content` with Argon2id + AES-256-GCM.
/// 3. Stops the spinner unconditionally.
/// 4. Stages the encrypted blob beside `options.output` (0o600).
/// 5. Rotates existing backups according to `options.keep` — only now, once
///    the new one is safely on disk (I-150).
/// 6. Renames the staged file into place.
/// 7. If `options.verify`: re-decrypts the file and compares content.
/// 8. Returns the path that was written.
///
/// `password` is a [`Zeroizing`] string, so it is cleared on every exit path,
/// `?`-propagated errors and panics included.
///
/// # Errors
///
/// - [`BackupError::EncryptionFailed`] — Argon2id or AES-256-GCM failed.
/// - [`BackupError::WriteFailed`] — could not write the backup to disk.
/// - [`BackupError::VerifyFailed`] — post-write integrity check failed.
/// - Other `anyhow` errors — unexpected encoding failures (non-UTF-8 paths,
///   rotation rename failures).
#[cfg(feature = "backup")]
pub fn backup_inner(
    content: &str,
    password: Zeroizing<String>,
    options: &BackupOptions,
) -> Result<PathBuf> {
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
    let encrypt_result = encrypt_content(content, &password, original_filename)
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

    // ── Write backup, then rotate, then make it visible (DEC-B7, I-150) ──────
    // ⚠️ This order is the fix. Rotation used to run first, before encryption
    // and writing, so any failure after it left the previous backup at `.1`
    // and no backup at the name the user knows. Now the new backup is fully
    // on disk (a temp file beside the output) before anything is renamed.
    //
    // ⚠️ Rotation works on the file the output names: a symlink's target. The
    // write goes through a link (DEC-B7), and rotating the link itself would
    // move it to `.1` and leave the new backup in a file nothing points at by
    // the name the user gave.
    let base = crate::core::safe_write::preview(&options.output)
        .map_err(|e| BackupError::WriteFailed(format!("{e:#}")))?
        .target;
    let staged = crate::core::safe_write::stage_private(&options.output, encrypted.as_bytes())
        .map_err(|e| BackupError::WriteFailed(format!("{e:#}")))?;
    if options.keep > 0 {
        rotate_backups(&base, options.keep)?;
    }
    let written = staged
        .commit()
        .map_err(|e| BackupError::WriteFailed(format!("{e:#}")))?;
    crate::core::safe_write::report(&written);

    if options.verbose {
        ui::verbose_stderr(format!("Backup written to {}", options.output.display()));
    }

    // ── Post-write integrity check ────────────────────────────────────────────
    // Re-decrypts the file just written and compares it byte-for-byte with the
    // original content. Costs one more Argon2id round, but proves the backup
    // is readable before the user discards the source.
    if options.verify {
        if options.verbose {
            ui::verbose_stderr("Verifying backup integrity…");
        }
        verify_backup(&options.output, content, &password)?;
        if options.verbose {
            ui::verbose_stderr("Integrity check passed");
        }
    }

    Ok(options.output.clone())
}

// ─── Output checks (I-146, DEC-B3) ───────────────────────────────────────────

/// Refuse an output that writing or rotating would destroy. Run before the
/// passphrase is asked for, so nothing is typed for a run that cannot finish.
///
/// The files a backup touches are the output (replaced, or renamed to `.1`)
/// and, with `keep > 0`, each `.1` … `.{keep}` slot (renamed one place up;
/// the last is replaced) — all beside the file the output names, a symlink's
/// target included. None of them may be the source: `--output .env` turned the
/// `.env` into ciphertext, and `backup out.1 --output out` moved the source to
/// `out.2` (I-146). None of them may be a directory: rotation renamed one
/// (DEC-B3).
///
/// A missing directory is not refused here; the write reports it (exit 5).
///
/// # Errors
///
/// Plain `anyhow` errors: "could not run", exit 2.
#[cfg(feature = "backup")]
pub fn check_output(source: &Path, output: &Path, keep: u32) -> Result<()> {
    use crate::core::safe_write;

    // The same wording `template` and `convert` use for their own input.
    safe_write::refuse_same_file(
        &output.to_string_lossy(),
        "env file",
        &source.to_string_lossy(),
    )?;
    // A directory (or anything else that is not a file) where the backup goes.
    let base = safe_write::preview(output)?.target;

    for n in 1..=keep {
        let slot = numbered_backup_path(&base, n);
        if safe_write::same_file(source, &slot) {
            anyhow::bail!(
                "--output '{}' would rotate '{}', which is the env file being backed up. \
                 Nothing was written; choose another --output.",
                output.display(),
                slot.display()
            );
        }
        if std::fs::symlink_metadata(&slot).is_ok_and(|m| m.is_dir()) {
            anyhow::bail!(
                "{} is a directory, and rotation would move it. \
                 Nothing was written; move it away or choose another --output.",
                slot.display()
            );
        }
    }
    Ok(())
}

// ─── Rotation ─────────────────────────────────────────────────────────────────

/// Rotate numbered backup files before writing a new one.
///
/// Renames in reverse order to avoid overwriting:
/// ```text
/// output.{keep-1} → output.{keep}
///         ⋮
/// output.1        → output.2
/// output          → output.1
/// ```
/// The file at position `keep` is replaced by the shift, with a warning;
/// files beyond it are never touched.
///
/// With `keep = 3` and `.env.backup` … `.env.backup.3` present: `.env.backup.3`
/// is warned about and replaced by `.env.backup.2`, `.1` moves to `.2`, and the
/// base file to `.1`.
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
/// Called by [`backup_inner`] when `options.verify = true`, with the same
/// `password` that was used for encryption.
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

// ─── Format ───────────────────────────────────────────────────────────────────

/// The binary format version this evnx writes, and the only one it reads.
/// Defined in `core::envfile`, which recognises a backup without this feature.
#[cfg(feature = "backup")]
const FORMAT_V1: u8 = crate::core::envfile::BACKUP_FORMAT_V1;
#[cfg(feature = "backup")]
const SALT_LEN: usize = 32;
#[cfg(feature = "backup")]
const NONCE_LEN: usize = 12;
#[cfg(feature = "backup")]
const KEY_LEN: usize = 32;
/// `version || salt || nonce`, before the ciphertext.
#[cfg(feature = "backup")]
const HEADER_LEN: usize = 1 + SALT_LEN + NONCE_LEN;

/// The smallest file `evnx backup` has ever written, in bytes after Base64.
///
/// The header and the GCM tag (61 bytes), plus the JSON envelope, which is
/// never shorter than 124 bytes: every field is always present, the file name
/// is at least one character and the tool version at least `0.1.0`. 100 leaves
/// room for that; anything shorter was cut off (I-149).
#[cfg(feature = "backup")]
const MIN_LEN: usize = HEADER_LEN + 16 + 100;

/// Argon2id cost parameters, tied to the format version byte (T-SHARE-28).
///
/// ⚠️ **Format-locked.** Changing [`KdfParams::V1`] makes every existing
/// backup unreadable: the golden files in `tests/golden/backup/` would stop
/// opening, which `tests/backup_format_lock.rs` exists to catch. A different
/// cost is a new format version with its own constant, and `V1` stays.
///
/// ⚠️ **Not evnx-crypto's.** The cloud master key is Argon2id too, with other
/// parameters (4 lanes) and another purpose; neither may follow the other.
///
/// These were written out twice, in encrypt and in decrypt, with a comment
/// asking whoever changed one to change the other.
#[cfg(feature = "backup")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory, in KiB.
    pub memory_kib: u32,
    /// Passes over the memory.
    pub iterations: u32,
    /// Lanes.
    pub lanes: u32,
}

#[cfg(feature = "backup")]
impl KdfParams {
    /// Format 1: 64 MiB, 3 passes, 1 lane. Every backup evnx has written.
    pub const V1: KdfParams = KdfParams {
        memory_kib: 65_536,
        iterations: 3,
        lanes: 1,
    };

    /// The parameters for a format version byte; `None` for a version this
    /// evnx does not know (a newer one).
    pub fn for_format(version: u8) -> Option<KdfParams> {
        (version == FORMAT_V1).then_some(Self::V1)
    }
}

/// The AES-256 key for `password` and `salt`: the one derivation both
/// [`encrypt_content`] and [`decrypt_content`] use.
#[cfg(feature = "backup")]
fn derive_key(password: &str, salt: &[u8], params: KdfParams) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    use anyhow::anyhow;
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};

    let argon2_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.lanes,
        Some(KEY_LEN),
    )
    .map_err(|e| anyhow!("Invalid Argon2 params: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let salt_string = argon2::password_hash::SaltString::encode_b64(salt)
        .map_err(|e| anyhow!("Failed to encode salt for Argon2: {}", e))?;
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt_string)
        .map_err(|e| anyhow!("Argon2id key derivation failed: {}", e))?;
    let hash_output = password_hash
        .hash
        .ok_or_else(|| anyhow!("Argon2id did not produce a hash output"))?;

    let bytes = hash_output.as_bytes();
    if bytes.len() < KEY_LEN {
        return Err(anyhow!(
            "Derived key too short: {} bytes (expected {KEY_LEN})",
            bytes.len()
        ));
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&bytes[..KEY_LEN]);
    Ok(key)
}

/// Why a backup could not be opened, where that is known (I-149, DEC-B6).
///
/// `restore` showed "Wrong password or corrupt backup" for every one of these,
/// so a backup from a newer evnx never said "upgrade". A wrong password and a
/// tampered file stay one variant: telling them apart would say which part of
/// the authentication failed.
#[cfg(feature = "backup")]
#[derive(Debug, thiserror::Error)]
pub enum DecryptError {
    /// Not Base64 text at all.
    #[error("This file is not an evnx backup: it is not the Base64 text evnx backup writes")]
    NotABackup,
    /// Cut off: too short, or Base64 that stops part-way.
    #[error("The backup file is truncated or damaged ({detail})")]
    Truncated {
        /// What was found.
        detail: String,
    },
    /// A format version byte this evnx does not know.
    #[error(
        "This backup uses format version {version}, made by a newer evnx; this version reads \
         format {FORMAT_V1}. Upgrade evnx to restore it."
    )]
    NewerFormat {
        /// The version byte.
        version: u8,
    },
    /// The authentication tag did not match.
    #[error("Decryption failed. The password may be incorrect or the backup file is corrupt.")]
    WrongPasswordOrTampered,
    /// The envelope's schema is newer than this evnx.
    #[error("This backup's contents use schema version {schema}, made by a newer evnx. Upgrade evnx to restore it.")]
    NewerSchema {
        /// The envelope's `schema_version`.
        schema: u64,
    },
    /// It decrypted, so the password was right, but the contents are not
    /// what evnx writes.
    #[error("The backup decrypted, but {0}")]
    Corrupt(String),
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
    use base64::{engine::general_purpose, Engine as _};

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
    let envelope_bytes = Zeroizing::new(
        serde_json::to_vec(&envelope).context("Failed to serialise metadata envelope")?,
    );

    // ── Argon2id key derivation ───────────────────────────────────────────────
    // A fresh 32-byte salt is generated for every backup so two encryptions of
    // the same file with the same password produce different ciphertext.
    let mut salt_bytes = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt_bytes);
    let key = derive_key(password, &salt_bytes, KdfParams::V1)?;

    // ── AES-256-GCM encryption ────────────────────────────────────────────────
    let cipher = Aes256Gcm::new((&*key).into());

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, envelope_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("AES-256-GCM encryption failed: {}", e))?;

    // ── Assemble binary envelope ──────────────────────────────────────────────
    // Layout: version(1) || salt(32) || nonce(12) || ciphertext(variable)
    // Increment the version byte when the format changes; never break v1 decryption.
    let mut result: Vec<u8> = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    result.push(FORMAT_V1);
    result.extend_from_slice(&salt_bytes);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(general_purpose::STANDARD.encode(&result))
}

// ─── Decryption (pub — used by restore) ──────────────────────────────────────

/// Decrypt a backup envelope produced by `encrypt_content`.
///
/// This function is `pub` so that `restore/core.rs` can call it directly.
/// Both commands share the same binary format; keeping decrypt logic here
/// means a format bump requires changes in exactly one place.
///
/// If the binary format ever changes, add a version to [`KdfParams::for_format`]
/// and a branch here — do **not** break decryption of existing version-1 files.
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
/// A [`DecryptError`] inside the `anyhow::Error` for everything a person can
/// act on: not Base64, truncated, a newer format or schema, a wrong password
/// or tampered file (one variant, deliberately), damaged contents. An Argon2id
/// failure, which valid inputs never produce, is a plain error.
#[cfg(feature = "backup")]
pub fn decrypt_content(encoded: &str, password: &str) -> Result<(String, BackupMetadata)> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use base64::{engine::general_purpose, DecodeError, Engine as _};

    // ── Base64 decode ─────────────────────────────────────────────────────────
    let text = encoded.trim();
    let raw = general_purpose::STANDARD
        .decode(text)
        .map_err(|e| match e {
            // A character Base64 does not have: this was never a backup.
            DecodeError::InvalidByte(..) => DecryptError::NotABackup,
            // Valid characters that stop short of a whole group: cut off.
            _ => DecryptError::Truncated {
                detail: format!(
                    "{} characters of Base64, which does not end where a backup does",
                    text.chars().count()
                ),
            },
        })?;

    // ── Parse binary envelope ─────────────────────────────────────────────────
    // The version byte first: a later format may lay out the rest differently,
    // so its length is not judged by version 1's (I-149).
    let Some(&version) = raw.first() else {
        return Err(DecryptError::Truncated {
            detail: "the file is empty".into(),
        }
        .into());
    };
    let Some(params) = KdfParams::for_format(version) else {
        return Err(DecryptError::NewerFormat { version }.into());
    };
    if raw.len() < MIN_LEN {
        return Err(DecryptError::Truncated {
            detail: format!(
                "{} bytes after decoding; an evnx backup is at least {MIN_LEN}",
                raw.len()
            ),
        }
        .into());
    }

    // Slice layout mirrors encrypt_content exactly.
    let salt_bytes = &raw[1..1 + SALT_LEN];
    let nonce_bytes = &raw[1 + SALT_LEN..HEADER_LEN];
    let ciphertext = &raw[HEADER_LEN..]; // GCM ciphertext + 16-byte tag

    // ── Argon2id key re-derivation ────────────────────────────────────────────
    let key = derive_key(password, salt_bytes, params)?;

    // ── AES-256-GCM decryption ────────────────────────────────────────────────
    let cipher = Aes256Gcm::new((&*key).into());
    let nonce = Nonce::from_slice(nonce_bytes);

    // Intentionally vague error — distinguishing "wrong password" from "tampered
    // file" would leak information about which part of authentication failed.
    let plaintext_bytes = Zeroizing::new(
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| DecryptError::WrongPasswordOrTampered)?,
    );

    // ── Deserialise JSON envelope ─────────────────────────────────────────────
    let envelope: serde_json::Value = serde_json::from_slice(&plaintext_bytes)
        .map_err(|_| DecryptError::Corrupt("its contents are not the JSON evnx writes".into()))?;

    // ── Validate JSON schema version ──────────────────────────────────────────
    let schema_version = envelope["schema_version"].as_u64().unwrap_or(0);
    if schema_version != 1 {
        return Err(DecryptError::NewerSchema {
            schema: schema_version,
        }
        .into());
    }

    let content = envelope["content"]
        .as_str()
        .ok_or_else(|| {
            DecryptError::Corrupt(format!(
                "its contents (schema v{schema_version}) have no 'content' field"
            ))
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

#[cfg(all(test, feature = "backup"))]
mod tests {
    use super::*;

    fn pw(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_owned())
    }

    /// I-160: `core::envfile::is_evnx_backup` knows what this module writes,
    /// so `doctor` can skip a backup and still read a plaintext copy.
    #[test]
    fn t_i160_core_recognises_a_backup() {
        use crate::core::envfile::is_evnx_backup;
        let blob = encrypt_content("A=1\n", "correct horse", ".env").unwrap();
        assert!(is_evnx_backup(&blob));
        assert!(is_evnx_backup(&format!("{blob}\n")), "a trailing newline");
        // Cut short: still ciphertext.
        assert!(is_evnx_backup(&blob[..100]));
    }

    // ── Format (T-SHARE-28, I-149) ────────────────────────────────────────────

    /// The one set of Argon2id parameters, as every backup since 0.1 used
    /// them. The golden files (`tests/backup_format_lock.rs`) prove the
    /// derivation; this pins the numbers so a change reads as one.
    #[test]
    fn kdf_params_v1_are_the_format_1_parameters() {
        assert_eq!(
            KdfParams::V1,
            KdfParams {
                memory_kib: 65_536,
                iterations: 3,
                lanes: 1
            }
        );
        assert_eq!(KdfParams::for_format(1), Some(KdfParams::V1));
        assert_eq!(KdfParams::for_format(2), None);
    }

    /// The smallest backup evnx can write is not called truncated.
    #[test]
    fn t_i149_the_smallest_backup_is_not_too_short() {
        use base64::{engine::general_purpose, Engine as _};
        let encoded = encrypt_content("", "password-123", "a").unwrap();
        let raw = general_purpose::STANDARD.decode(&encoded).unwrap();
        assert!(raw.len() >= MIN_LEN, "{} < {MIN_LEN}", raw.len());
        assert_eq!(decrypt_content(&encoded, "password-123").unwrap().0, "");
    }

    #[test]
    fn t_i149_each_damage_has_its_own_error() {
        let kind = |encoded: &str| {
            let e = decrypt_content(encoded, "password-123").unwrap_err();
            match e.downcast_ref::<DecryptError>() {
                Some(DecryptError::NotABackup) => "not a backup",
                Some(DecryptError::Truncated { .. }) => "truncated",
                Some(DecryptError::NewerFormat { version: 2 }) => "newer",
                Some(DecryptError::WrongPasswordOrTampered) => "wrong password",
                other => panic!("unexpected {other:?}"),
            }
        };
        use base64::{engine::general_purpose, Engine as _};
        let good = encrypt_content("A=1\n", "password-123", ".env").unwrap();
        let mut raw = general_purpose::STANDARD.decode(&good).unwrap();

        assert_eq!(kind("not base64 at all!"), "not a backup");
        assert_eq!(kind(""), "truncated");
        assert_eq!(kind(&good[..100]), "truncated");
        assert_eq!(kind(&good[..101]), "truncated");
        raw[0] = 2;
        assert_eq!(kind(&general_purpose::STANDARD.encode(&raw)), "newer");
        raw[0] = 1;
        raw[60] ^= 1;
        assert_eq!(
            kind(&general_purpose::STANDARD.encode(&raw)),
            "wrong password"
        );
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

        let written_path = backup_inner(env_content, pw("test-password-123"), &options)
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

        backup_inner("KEY=value\n", pw("verbosepass123"), &options)
            .expect("backup_inner in verbose mode must not panic");
    }

    /// I-150, DEC-B7: when the new backup cannot be written, the previous one
    /// stays where it was. Rotation used to run first, so a failed write left
    /// `.env.backup.1` and no `.env.backup`.
    #[test]
    #[cfg(all(feature = "backup", unix))]
    fn t_i150_a_failed_write_does_not_rotate() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let output_path = dir.path().join(".env.backup");
        std::fs::write(&output_path, "previous backup").unwrap();
        // A directory nobody can create files in: the temp file fails.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(dir.path().join("probe"), "").is_ok() {
            // Running as root: permissions do not stop it, so the scenario
            // cannot be built here.
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let options = BackupOptions {
            env: dir.path().join(".env"),
            output: output_path.clone(),
            verbose: false,
            key_file: None,
            keep: 3,
            verify: false,
        };
        let result = backup_inner("A=1\n", pw("test-password-123"), &options);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&output_path).unwrap(),
            "previous backup"
        );
        assert!(
            !numbered_backup_path(&output_path, 1).exists(),
            "nothing rotated"
        );
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

        backup_inner(content, pw("verifypass123"), &options)
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
