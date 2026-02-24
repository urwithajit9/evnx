//! Backup command — create an encrypted backup of a `.env` file.
//!
//! # Security model
//!
//! Encryption uses **AES-256-GCM** (authenticated encryption) with a key derived
//! from a user-supplied password via **Argon2id** (memory-hard KDF). Every backup
//! uses a freshly generated random salt and nonce, so two backups of the same file
//! with the same password always produce different ciphertext.
//!
//! ## Binary format (version 1)
//!
//! ```text
//! ┌─────────┬────────────┬──────────┬────────────────────────────────┐
//! │ version │    salt    │  nonce   │  AES-256-GCM ciphertext        │
//! │  1 byte │  32 bytes  │ 12 bytes │  variable (JSON envelope)      │
//! └─────────┴────────────┴──────────┴────────────────────────────────┘
//! ```
//!
//! The entire structure is Base64-encoded (standard alphabet) before being written
//! to disk. The ciphertext is the AES-256-GCM encryption of a JSON envelope:
//!
//! ```json
//! {
//!   "version": 1,
//!   "created_at": "2025-02-24T10:00:00Z",
//!   "original_file": ".env",
//!   "tool_version": "0.1.0",
//!   "content": "DATABASE_URL=...\nSECRET_KEY=..."
//! }
//! ```
//!
//! Embedding metadata *inside* the encrypted payload means it is both confidential
//! (an attacker without the password cannot learn the filename or timestamp) and
//! tamper-evident (altering metadata invalidates the GCM authentication tag).
//!
//! ## Argon2id parameters
//!
//! | Parameter   | Value    | Rationale                                 |
//! |-------------|----------|-------------------------------------------|
//! | variant     | Argon2id | Resistant to GPU and side-channel attacks |
//! | memory      | 64 MiB   | Slows brute-force on commodity hardware   |
//! | iterations  | 3        | Adds time cost on top of memory cost      |
//! | parallelism | 1        | Single-threaded CLI usage                 |
//! | output len  | 32 bytes | Exactly one AES-256 key                   |
//!
//! # Example
//!
//! ```bash
//! dotenv-space backup
//! dotenv-space backup --env .env.production --output prod.backup
//! ```
//!
//! # Future work
//!
//! - `--key-file` flag: derive the key from a file instead of a password
//!   (useful for automated/CI backup pipelines).
//! - `--recipient` flag: asymmetric encryption (age / NaCl sealed boxes) so a
//!   backup can be decrypted by a public-key holder without knowing the password.
//! - Backup rotation: keep N most recent backups, auto-prune older ones.
//! - Integrity manifest: store a SHA-256 hash of the plaintext so the restore
//!   command can verify it was not silently corrupted after writing.

use anyhow::{anyhow, Context, Result};
use colored::*;

/// Entry point for the `backup` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly — it does **not** panic or return an error.
///
/// # Arguments
///
/// * `env`     — Path to the `.env` file to back up (default: `.env`).
/// * `output`  — Destination path for the encrypted backup (default: `<env>.backup`).
/// * `verbose` — Print extra diagnostic information during the run.
pub fn run(env: String, output: Option<String>, verbose: bool) -> Result<()> {
    // ── Feature-disabled stub ────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        // Reference parameters explicitly to silence unused-variable warnings
        // without renaming them, keeping the signature consistent with main.rs.
        let _ = (&env, &output, verbose);
        println!("{} Backup feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        return Ok(());
    }

    // ── Full implementation (feature = "backup") ─────────────────────────────
    #[cfg(feature = "backup")]
    {
        use dialoguer::Password;
        use std::fs;
        use std::path::Path;

        if verbose {
            println!("{}", "Running backup in verbose mode".dimmed());
        }

        println!(
            "\n{}",
            "┌─ Create encrypted backup ───────────────────────────┐".cyan()
        );
        println!(
            "{}",
            "│ Your .env will be encrypted with AES-256-GCM        │".cyan()
        );
        println!(
            "{}",
            "│ Key derived via Argon2id (64 MiB, 3 iterations)     │".cyan()
        );
        println!(
            "{}\n",
            "└──────────────────────────────────────────────────────┘".cyan()
        );

        // ── Validate source file ─────────────────────────────────────────────
        if !Path::new(&env).exists() {
            return Err(anyhow!("File not found: {}", env));
        }

        let content =
            fs::read_to_string(&env).with_context(|| format!("Failed to read {}", env))?;

        // Warn — but do not abort — if the file does not look like a .env file.
        // The user might intentionally be backing up a non-standard file.
        if !looks_like_dotenv(&content) {
            println!(
                "{} File does not look like a standard .env file — backing up anyway",
                "⚠️".yellow()
            );
        }

        println!("{} Read {} bytes from {}", "✓".green(), content.len(), env);

        // ── Password prompt ──────────────────────────────────────────────────
        let password = Password::new()
            .with_prompt("Enter encryption password")
            .interact()?;

        if password.is_empty() {
            return Err(anyhow!("Password must not be empty"));
        }

        // Minimum length check — Argon2id makes short passwords expensive to
        // brute-force, but we still enforce a floor as a sanity guard.
        if password.len() < 8 {
            return Err(anyhow!(
                "Password must be at least 8 characters (got {})",
                password.len()
            ));
        }

        let password_confirm = Password::new().with_prompt("Confirm password").interact()?;

        if password != password_confirm {
            return Err(anyhow!("Passwords do not match"));
        }

        println!("{} Password accepted", "✓".green());

        // ── Encrypt ──────────────────────────────────────────────────────────
        println!("Encrypting… (this may take a moment due to Argon2id key derivation)");

        let original_filename = Path::new(&env)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(".env")
            .to_string();

        let encrypted = encrypt_content(&content, &password, &original_filename)
            .context("Encryption failed")?;

        // ── Write backup ─────────────────────────────────────────────────────
        let output_path = output.unwrap_or_else(|| format!("{}.backup", env));

        fs::write(&output_path, &encrypted)
            .with_context(|| format!("Failed to write backup to {}", output_path))?;

        let backup_size = std::fs::metadata(&output_path)
            .map(|m| m.len().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        println!("{} Backup created at {}", "✓".green(), output_path);
        println!("    Size: {} bytes (encrypted + Base64)", backup_size);

        println!("\n{}", "⚠️  Important:".yellow().bold());
        println!("  • Keep your password safe — it cannot be recovered");
        println!("  • Store the backup in a secure, separate location");
        println!(
            "  • To restore: dotenv-space restore {} --output {}",
            output_path, env
        );
        println!("  • Test the restore before deleting the original .env");

        Ok(())
    }
}

// ── Encryption ────────────────────────────────────────────────────────────────

/// Encrypt the plaintext content of a `.env` file.
///
/// Produces a Base64-encoded string containing the complete binary envelope:
/// `version(1) || salt(32) || nonce(12) || AES-256-GCM-ciphertext`.
///
/// The ciphertext decrypts to a JSON envelope containing the `.env` content
/// and metadata (see module-level documentation for the schema).
///
/// # Arguments
///
/// * `plaintext`          — The raw `.env` file content.
/// * `password`           — User-supplied encryption password.
/// * `original_filename`  — The filename (e.g. `.env`) stored in the metadata envelope so restore can surface it to the user.
///
/// # Errors
///
/// Returns an error if Argon2id key derivation or AES-256-GCM encryption fails.
/// In practice these only fail with invalid parameters, which are hardcoded here.
#[cfg(feature = "backup")]
fn encrypt_content(plaintext: &str, password: &str, original_filename: &str) -> Result<String> {
    use aes_gcm::{
        aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
        Aes256Gcm, Nonce,
    };
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use base64::{engine::general_purpose, Engine as _};

    // ── JSON metadata envelope ────────────────────────────────────────────────
    // Stored inside the ciphertext so it is confidential and tamper-evident.
    let created_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let envelope = serde_json::json!({
        "version": 1,
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
    let mut salt_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut salt_bytes);

    // Explicit parameters pin the KDF behaviour regardless of library defaults.
    // output_len = 32 guarantees exactly one AES-256 key's worth of material.
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
    let mut result: Vec<u8> = Vec::with_capacity(1 + 32 + 12 + ciphertext.len());
    result.push(1u8); // Version byte — increment when format changes
    result.extend_from_slice(&salt_bytes);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(general_purpose::STANDARD.encode(&result))
}

// ── Decryption (pub — used by restore.rs) ─────────────────────────────────────

/// Decrypt a backup envelope produced by [`encrypt_content`].
///
/// This function is `pub` so that `restore.rs` can call it directly. Both
/// commands live in the same `commands` module, keeping format logic in one
/// place. If the binary format ever changes, bump the version byte here and
/// add a new match arm — do **not** break decryption of existing version-1 files.
///
/// # Returns
///
/// A tuple of `(plaintext, metadata)`:
/// - `plaintext` — The original `.env` file content.
/// - `metadata`  — [`BackupMetadata`] with the original filename, creation timestamp, and tool version extracted from the JSON envelope.
///
/// # Errors
///
/// Returns a descriptive [`anyhow::Error`] for:
/// - Base64 decode failure (not a dotenv-space backup, or file is truncated).
/// - Unknown format version (backup was made by a newer tool version).
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

    // ── Base64 decode ─────────────────────────────────────────────────────────
    let raw = general_purpose::STANDARD.decode(encoded.trim()).context(
        "Failed to Base64-decode the backup file. \
             Is this a valid dotenv-space backup?",
    )?;

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
             This backup was created by a newer version of dotenv-space. \
             Please upgrade the tool and try again.",
            version
        ));
    }

    // Slice layout mirrors encrypt_content exactly.
    let salt_bytes = &raw[1..33]; // 32 bytes
    let nonce_bytes = &raw[33..45]; // 12 bytes
    let ciphertext = &raw[45..]; // remainder = GCM ciphertext + 16-byte tag

    // ── Argon2id key re-derivation ────────────────────────────────────────────
    // Same parameters as encrypt_content — if these ever change, add a version
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

    let content = envelope["content"]
        .as_str()
        .ok_or_else(|| anyhow!("Backup envelope is missing the 'content' field."))?
        .to_string();

    let metadata = BackupMetadata {
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

// ── Supporting types ──────────────────────────────────────────────────────────

/// Metadata extracted from a decrypted backup envelope.
///
/// Returned by [`decrypt_content`] so the `restore` command can display
/// information about the backup before writing any files.
#[cfg(feature = "backup")]
#[derive(Debug)]
pub struct BackupMetadata {
    /// ISO 8601 UTC timestamp recorded when the backup was created.
    pub created_at: String,
    /// The original filename (e.g. `.env`, `.env.production`).
    pub original_file: String,
    /// The `CARGO_PKG_VERSION` of the tool that created this backup.
    pub tool_version: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Heuristically check whether `content` resembles a `.env` file.
///
/// A file is considered valid if at least **80%** of its non-empty lines are:
/// - blank lines,
/// - comments beginning with `#`, or
/// - `KEY=VALUE` assignments where KEY contains only alphanumerics and `_`.
///
/// This check is intentionally lenient — it exists to warn the user if they
/// accidentally pass the wrong file, not to enforce strict `.env` grammar.
fn looks_like_dotenv(content: &str) -> bool {
    if content.trim().is_empty() {
        return true;
    }

    let valid_line = |line: &str| -> bool {
        let line = line.trim();
        line.is_empty()
            || line.starts_with('#')
            || line
                .split_once('=')
                .map(|(key, _)| {
                    !key.trim().is_empty()
                        && key.trim().chars().all(|c| c.is_alphanumeric() || c == '_')
                })
                .unwrap_or(false)
    };

    let non_empty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return true;
    }
    let valid_count = non_empty.iter().filter(|&&l| valid_line(l)).count();
    // Integer equivalent of valid_count / total >= 0.8
    valid_count * 10 >= non_empty.len() * 8
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── looks_like_dotenv ─────────────────────────────────────────────────────

    #[test]
    fn test_looks_like_dotenv_valid() {
        assert!(looks_like_dotenv(
            "# Database\nDATABASE_URL=postgresql://localhost\nSECRET_KEY=abc123\n"
        ));
    }

    #[test]
    fn test_looks_like_dotenv_empty() {
        assert!(looks_like_dotenv(""));
        assert!(looks_like_dotenv("  \n  "));
    }

    #[test]
    fn test_looks_like_dotenv_rejects_prose() {
        assert!(!looks_like_dotenv(
            "This is just a plain text file.\nWith no env vars at all.\nWhatsoever."
        ));
    }

    #[test]
    fn test_looks_like_dotenv_comments_and_blanks() {
        assert!(looks_like_dotenv("# Comment\n\n# Another\nKEY=value\n"));
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

        // Error message must be user-friendly and not expose internal details.
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
        // Flip a byte well into the ciphertext region (after the 45-byte header).
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
        // Random salt + nonce mean identical inputs → different outputs.
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
}
