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
//! evnx backup --env .env.production --output prod.backup
//! ```
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0 | Success |
//! | 1 | Generic error (IO, unexpected failure) |
//! | 2 | Source file not found or not a regular file |
//! | 3 | Password confirmation did not match |
//! | 4 | Encryption failed |
//! | 5 | Failed to write backup file |
//!
//! # Module layout
//!
//! - **`mod.rs`** (this file) — CLI adapter: header, password prompts,
//!   validation, orchestration. No pure logic lives here.
//! - **`core.rs`** — Pure logic: [`BackupOptions`], [`backup_inner`],
//!   [`encrypt_content`], [`decrypt_content`], [`BackupMetadata`].
//! - **`error.rs`** — [`BackupError`] enum with exit codes.
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

pub mod core;
pub mod error;

pub use error::BackupError;

// Re-export the items that `restore/core.rs` imports by path so those import
// paths remain stable after the module split. Changing these paths would
// require touching restore code that has no other reason to change.
//
//   crate::commands::backup::decrypt_content   ← pub fn
//   crate::commands::backup::BackupMetadata    ← pub struct
//   crate::commands::backup::encrypt_content   ← pub(crate) fn (tests only)
#[cfg(feature = "backup")]
pub use self::core::decrypt_content;
#[cfg(all(feature = "backup", test))]
pub(crate) use self::core::encrypt_content;
#[cfg(feature = "backup")]
pub use self::core::BackupMetadata;

// ─── Entry point ──────────────────────────────────────────────────────────────

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
pub fn run(env: String, output: Option<String>, verbose: bool) -> anyhow::Result<()> {
    // ── Feature-disabled stub ─────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        // Reference parameters explicitly to silence unused-variable warnings
        // without renaming them, keeping the signature consistent with main.rs.
        let _ = (&env, &output, verbose);
        use colored::Colorize;
        println!("{} Backup feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        return Ok(());
    }

    // ── Full implementation (feature = "backup") ──────────────────────────────
    #[cfg(feature = "backup")]
    {
        use std::fs;
        use std::path::Path;

        use dialoguer::Password;
        use zeroize::Zeroize;

        use crate::docs;
        use crate::utils::looks_like_dotenv;
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

        if verbose {
            ui::verbose_stderr(format!("Source path  : {}", env));
        }

        let content = fs::read_to_string(&env)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", env, e))?;

        if verbose {
            ui::verbose_stderr(format!("Read {} bytes from {}", content.len(), env));
        }

        // ── Sanity check the source ───────────────────────────────────────────
        // Warn — but do not abort — if the file does not look like a .env file.
        // The user might intentionally be backing up a non-standard file.
        if !looks_like_dotenv(&content) {
            ui::warning("File does not look like a standard .env file — backing up anyway");
        } else {
            ui::success(format!("Read {} bytes from {}", content.len(), env));
        }

        // ── Password prompts ──────────────────────────────────────────────────
        // Backup always requires interactive confirmation; non-interactive
        // backup is a future `--key-file` concern.
        let mut password = Password::new()
            .with_prompt("Enter encryption password")
            .interact()?;

        if password.is_empty() {
            password.zeroize();
            return Err(anyhow::anyhow!("Password must not be empty"));
        }

        // Minimum length check — Argon2id makes short passwords expensive to
        // brute-force, but we still enforce a floor as a sanity guard.
        if password.len() < 8 {
            let len = password.len();
            password.zeroize();
            return Err(anyhow::anyhow!(
                "Password must be at least 8 characters (got {})",
                len
            ));
        }

        if verbose {
            ui::verbose_stderr("Password accepted — awaiting confirmation");
        }

        let mut password_confirm = Password::new().with_prompt("Confirm password").interact()?;

        if password != password_confirm {
            password.zeroize();
            password_confirm.zeroize();
            return Err(BackupError::PasswordMismatch.into());
        }
        password_confirm.zeroize();

        if verbose {
            ui::verbose_stderr("Passwords match");
        }

        // ── Resolve paths ─────────────────────────────────────────────────────
        let output_path_str = output.unwrap_or_else(|| format!("{}.backup", env));
        let output_path = std::path::PathBuf::from(&output_path_str);

        let options = BackupOptions {
            env: std::path::PathBuf::from(&env),
            output: output_path.clone(),
            verbose,
        };

        // ── Encrypt and write ─────────────────────────────────────────────────
        // `backup_inner` owns the spinner, ZeroizeOnDrop guard, encrypt, and
        // write steps. All that remains here is UI output on success.
        core::backup_inner(&content, password, &options)?;

        // ── Success summary ───────────────────────────────────────────────────
        let backup_size = std::fs::metadata(&output_path)
            .map(|m| m.len().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let size_display = format!("{} bytes (encrypted + Base64)", backup_size);

        println!();
        ui::success("Backup created successfully");
        ui::print_key_value(&[
            ("Source", env.as_str()),
            ("Backup", output_path.to_str().unwrap_or(&output_path_str)),
            ("Size", &size_display),
        ]);

        print_next_steps(&output_path, &env);
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

    println!("\n{}", "⚠️  Important:".yellow().bold());
    println!("  • Keep your password safe — it cannot be recovered");
    println!("  • Store the backup in a secure, separate location");
    println!(
        "  • To restore: evnx restore {} --output {}",
        backup_path.display(),
        env_path,
    );
    println!("  • Test the restore before deleting the original .env");
}
