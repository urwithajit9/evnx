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
//! The entire structure is Base64-encoded (standard alphabet) before being
//! written to disk. The ciphertext is the AES-256-GCM encryption of a JSON
//! envelope:
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "version": 1,
//!   "created_at": "2025-02-24T10:00:00Z",
//!   "original_file": ".env",
//!   "tool_version": "0.1.0",
//!   "content": "DATABASE_URL=...\nSECRET_KEY=..."
//! }
//! ```
//!
//! Embedding metadata *inside* the encrypted payload means it is both
//! confidential (an attacker without the password cannot learn the filename or
//! timestamp) and tamper-evident (altering metadata invalidates the GCM tag).
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
//! evnx backup
//! evnx backup .env.production --output prod.backup
//! evnx backup --env-name production
//! evnx backup --key-file /run/secrets/backup-key --keep 5 --verify
//! ```
//!
//! # Passphrase
//!
//! `--key-file` (also `--password-file`), then `EVNX_PASSWORD`, then a prompt
//! asked twice — [`crate::utils::secret_source`], the same rule `restore` reads
//! with. A new backup needs at least 8 characters from any of them (DEC-B2).
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0 | Success |
//! | 2 | Could not run: source not found or not a file, key file or passphrase, an output that is the source or a directory, no terminal to ask on — with the "No verdict" line |
//! | 3 | Password confirmation did not match |
//! | 4 | Encryption failed |
//! | 5 | Failed to write backup file |
//! | 6 | Post-write integrity check failed |
//!
//! Exit 1 is never produced (DEC-B9): it means "findings" for the commands
//! that have them, and a backup has none.
//!
//! # Module layout
//!
//! - **`mod.rs`** (this file) — CLI adapter: header, password prompts,
//!   key-file resolution, validation, orchestration. No pure logic lives here.
//! - **`core.rs`** — Pure logic: [`BackupOptions`](core::BackupOptions),
//!   [`backup_inner`](core::backup_inner), [`check_output`](core::check_output),
//!   `rotate_backups`, `verify_backup`, `encrypt_content`, [`decrypt_content`],
//!   [`KdfParams`](core::KdfParams), [`DecryptError`], [`BackupMetadata`].
//! - **`error.rs`** — [`BackupError`] enum with exit codes.
//!
//! # Future work
//!
//! - `--recipient` flag: asymmetric encryption (age / NaCl sealed boxes) so a
//!   backup can be decrypted by a public-key holder without knowing the password.
//! - Backup rotation hard-delete: opt-in `--prune` flag to actually remove files
//!   beyond `--keep` rather than just warning.
//! - Integrity manifest: store a SHA-256 hash of the plaintext separately so
//!   integrity can be checked without a full decryption round-trip.

pub mod core;
pub mod error;

pub use error::BackupError;

// Re-export the items that `restore/core.rs` imports by path so those import
// paths remain stable after the module split. Changing these paths would
// require touching restore code that has no other reason to change.
//
//   crate::commands::backup::decrypt_content   ← pub fn
//   crate::commands::backup::DecryptError      ← pub enum (why it failed, I-149)
//   crate::commands::backup::BackupMetadata    ← pub struct
//   crate::commands::backup::encrypt_content   ← pub(crate) fn (tests only)
#[cfg(feature = "backup")]
pub use self::core::decrypt_content;
#[cfg(all(feature = "backup", test))]
pub(crate) use self::core::encrypt_content;
#[cfg(feature = "backup")]
pub use self::core::BackupMetadata;
#[cfg(feature = "backup")]
pub use self::core::DecryptError;

// ─── Entry point ──────────────────────────────────────────────────────────────

/// Entry point for the `backup` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly — it does **not** panic or return an error.
///
/// # Arguments
///
/// * `env`      — Path to the `.env` file to back up (default: `.env`).
/// * `output`   — Destination path for the encrypted backup (default: `<env>.backup`).
/// * `verbose`  — Print extra diagnostic information during the run.
/// * `key_file` — Path to a key file to use instead of an interactive password.
/// * `keep`     — Number of previous backups to retain (default: 3, `0` = overwrite).
/// * `verify`   — Re-decrypt and integrity-check after writing.
pub fn run(
    env: String,
    output: Option<String>,
    verbose: bool,
    key_file: Option<String>,
    keep: u32,
    verify: bool,
) -> anyhow::Result<()> {
    // ── Feature-disabled stub ─────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        let _ = (&env, &output, verbose, &key_file, keep, verify);
        use colored::Colorize;
        println!("{} Backup feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        Ok(())
    }

    // ── Full implementation (feature = "backup") ──────────────────────────────
    #[cfg(feature = "backup")]
    {
        use std::fs;
        use std::path::Path;

        use crate::docs;
        use crate::utils::looks_like_dotenv;
        use crate::utils::secret_source::{self, Purpose};
        use crate::utils::ui;

        use self::core::BackupOptions;

        // ── Header ────────────────────────────────────────────────────────────
        ui::print_header("evnx backup", Some("Encrypt and back up your .env file"));

        // ── Validate source file ──────────────────────────────────────────────
        if !Path::new(&env).exists() {
            return Err(BackupError::FileNotFound(env.clone()).into());
        }

        if !Path::new(&env).is_file() {
            return Err(BackupError::NotAFile(env.clone()).into());
        }

        // ── Refuse an output the write or rotation would destroy ─────────────
        // Before the passphrase, so nothing is typed for a run that cannot
        // finish (I-146, DEC-B3).
        let output_path_str = output.unwrap_or_else(|| format!("{}.backup", env));
        let output_path = std::path::PathBuf::from(&output_path_str);
        core::check_output(Path::new(&env), &output_path, keep)?;

        if verbose {
            ui::verbose_stderr(format!("Source path  : {}", env));
        }

        let content = fs::read_to_string(&env)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", env, e))?;

        if verbose {
            ui::verbose_stderr(format!("Read {} bytes from {}", content.len(), env));
        }

        // Warn — but do not abort — if the file does not look like a .env file.
        if !looks_like_dotenv(&content) {
            ui::warning("File does not look like a standard .env file — backing up anyway");
        } else {
            ui::success(format!("Read {} bytes from {}", content.len(), env));
        }

        // ── Resolve password source ───────────────────────────────────────────
        // Key file, then EVNX_PASSWORD, then the prompt (asked twice): one rule
        // with `restore`, in `utils::secret_source` (DEC-B13).
        let password = secret_source::resolve(
            key_file.as_deref(),
            Purpose::Encrypt,
            &format!("evnx backup {env}"),
            verbose,
        )
        .map_err(|e| {
            if e.is::<secret_source::Mismatch>() {
                BackupError::PasswordMismatch.into()
            } else {
                e
            }
        })?;

        let options = BackupOptions {
            env: std::path::PathBuf::from(&env),
            output: output_path.clone(),
            verbose,
            key_file: key_file.as_deref().map(std::path::PathBuf::from),
            keep,
            verify,
        };

        // ── Encrypt, rotate, write (and optionally verify) ────────────────────
        core::backup_inner(&content, password, &options)?;

        // ── Success summary ───────────────────────────────────────────────────
        let backup_size = std::fs::metadata(&output_path)
            .map(|m| m.len().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let size_display = format!("{} bytes (encrypted + Base64)", backup_size);
        let verify_display = if verify { "yes" } else { "no" };

        // `-q`: nothing on success, the reminder included (DEC-G4, I-153).
        if !ui::is_quiet() {
            println!();
            ui::success(if verify {
                "Backup created and verified successfully"
            } else {
                "Backup created successfully"
            });
            ui::print_key_value(&[
                ("Source", env.as_str()),
                ("Backup", output_path.to_str().unwrap_or(&output_path_str)),
                ("Size", &size_display),
                ("Verified", verify_display),
            ]);

            print_next_steps(&output_path, &env);
        }
        ui::print_docs_hint(&docs::BACKUP);

        Ok(())
    }
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Print context-aware next steps after a successful backup.
///
/// Mirrors the `print_next_steps` pattern from `restore/core.rs`: a private
/// function that owns the "what to do next" prose, keeping `run()` focused on
/// orchestration.
#[cfg(feature = "backup")]
fn print_next_steps(backup_path: &std::path::Path, env_path: &str) {
    use colored::Colorize;

    println!("\n{}", "! Important:".yellow().bold());
    println!("  • Keep your password (or key file) safe — it cannot be recovered");
    println!("  • Store the backup in a secure, separate location");
    println!(
        "  • To restore: evnx restore {} --output {}",
        backup_path.display(),
        env_path,
    );
    println!("  • Test the restore before deleting the original .env");
}
