//! Restore command — decrypt a backup created by `evnx backup`.
//!
//! # Overview
//!
//! Reads a Base64-encoded AES-256-GCM backup (local file or, once available,
//! an evnx-cloud reference), derives the decryption key from the
//! user-supplied password using Argon2id, decrypts the payload, and writes
//! the recovered `.env` content to disk.
//!
//! This file is the **CLI adapter**: it owns all TTY interaction — the
//! decorative header, the password prompt, and the overwrite confirmation.
//! Pure restore logic (decryption, path selection, writing) lives in
//! [`core`] and is independently testable without a terminal.
//!
//! # Module layout
//!
//! | Module   | Responsibility                                     |
//! |----------|----------------------------------------------------|
//! | `mod.rs` | CLI adapter — header, prompts, orchestration       |
//! | `core`   | Pure logic — decrypt, validate, write              |
//! | `source` | Backup source — local path or cloud reference      |
//!
//! # Safety behaviours
//!
//! ## Password handling
//!
//! A **single password prompt** is used without confirmation. A typo during
//! backup creation could make data permanently unrecoverable; a typo during
//! restore simply fails decryption (safe and reversible). Confirmation on
//! restore adds friction with no safety benefit.
//!
//! ## Overwrite protection
//!
//! If the output file already exists the user is **always prompted** before
//! it is overwritten. There is intentionally no `--force` flag. An accidental
//! overwrite of a live `.env` is difficult to undo and could destroy
//! credentials that are not backed up anywhere else.
//!
//! ## Validation failure fallback
//!
//! When the decrypted content does not pass the `.env` validation heuristic,
//! the file is written to `<output>.restored` instead of `<output>`. The
//! user is guided through inspection and manual rename. See
//! [`core::choose_write_path`](core) for details.
//!
//! ## Memory safety
//!
//! The password is moved into a `ZeroizeOnDrop` guard in `core::prepare_restore`
//! the moment it is passed over — it is zeroized on every exit path, including
//! `?`-propagated errors and panics. The encoded ciphertext blob is explicitly
//! zeroized in this file after `prepare_restore` returns.
//!
//! # Examples
//!
//! ```bash
//! evnx restore .env.backup
//! evnx restore .env.backup --output .env.production
//! evnx restore .env.backup --dry-run          # validate without writing
//! evnx restore cloud://myproject              # latest cloud backup (planned)
//! evnx restore cloud://myproject/backup-abc123
//! ```

// Sub-modules are conditionally compiled with the `backup` feature so that
// the types they depend on (e.g. `BackupMetadata`, `decrypt_content`) are
// only referenced when the feature is active.
#[cfg(feature = "backup")]
pub mod core;

#[cfg(feature = "backup")]
pub mod source;

use anyhow::Result;

/// Entry point for the `restore` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly without returning an error.
///
/// # Arguments
///
/// * `backup`   — Local path or `cloud://project[/id]` reference to the
///   backup file. Pass `cloud://project` to restore the latest snapshot, or
///   `cloud://project/backup-id` to pin to a specific one.
/// * `output`   — Destination path for the restored file (default `.env`).
///   A `.restored` suffix is appended automatically when decrypted content
///   fails `.env` validation, to protect any existing live file.
/// * `verbose`  — Emit a diagnostic message at each pipeline stage.
/// * `dry_run`  — Decrypt and validate, but do not write any files.
pub fn run(backup: String, output: String, verbose: bool, dry_run: bool) -> Result<()> {
    // ── Feature-disabled stub ─────────────────────────────────────────────────
    // Both cfg blocks use expression syntax (`Ok(())`) rather than
    // `return Ok(())` so that whichever block is compiled becomes the tail
    // expression of the function body — avoiding a type-mismatch from the
    // implicit `()` that would follow a `return` statement.
    #[cfg(not(feature = "backup"))]
    {
        let _ = (&backup, &output, verbose, dry_run);
        println!("{} Backup/restore feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        Ok(())
    }

    // ── Full implementation ───────────────────────────────────────────────────
    #[cfg(feature = "backup")]
    {
        use anyhow::anyhow;
        use dialoguer::{Confirm, Password};
        use std::path::PathBuf;
        use zeroize::Zeroize;

        use self::core::{commit_restore, prepare_restore, PrepareResult, RestoreOptions};
        use self::source::BackupSource;
        use crate::utils::ui;

        // ── Parse source ──────────────────────────────────────────────────────
        let src = BackupSource::parse(&backup);

        // ── Header ────────────────────────────────────────────────────────────
        ui::print_header(
            "evnx restore",
            Some("Decrypt and restore from an encrypted backup"),
        );

        if verbose {
            ui::verbose_stderr(format!("Source: {}", src.display_path()));
        }

        // ── Fetch encoded blob ────────────────────────────────────────────────
        let mut encoded = src.fetch()?;
        ui::success(format!("Read backup from {}", src.display_path()));

        // ── Password prompt ───────────────────────────────────────────────────
        // No echo — the password is never displayed or logged.
        // We do NOT confirm the password here: a typo simply fails decryption
        // (safe and reversible), so confirmation would add friction for no gain.
        let password = Password::new()
            .with_prompt("Enter decryption password")
            .interact()?;

        if password.is_empty() {
            // Zeroize the encoded blob before returning the error so sensitive
            // ciphertext does not linger in memory longer than necessary.
            encoded.zeroize();
            return Err(anyhow!("Password must not be empty"));
        }

        // ── Core restore logic ────────────────────────────────────────────────
        // `prepare_restore` takes ownership of `password` and zeroizes it via
        // a RAII guard — it is cleared on every exit path, including errors
        // and panics.
        let options = RestoreOptions {
            output: PathBuf::from(&output),
            dry_run,
            verbose,
        };

        let result = prepare_restore(&encoded, password, &options);

        // Zeroize the ciphertext blob now that decryption is complete,
        // regardless of whether it succeeded or failed.
        encoded.zeroize();

        match result? {
            // ── Dry-run: nothing to write ─────────────────────────────────────
            PrepareResult::DryRun => Ok(()),

            // ── Normal path: check for overwrite then write ───────────────────
            PrepareResult::Ready(outcome) => {
                // Overwrite protection — always prompt; there is intentionally
                // no --force flag.  Overwriting a live .env without confirmation
                // is too easy to do by accident, and the consequences (lost
                // credentials) are hard to undo.
                if outcome.write_path.exists() {
                    ui::warning(format!(
                        "Output file already exists: {}",
                        outcome.write_path.display()
                    ));

                    let overwrite = Confirm::new()
                        .with_prompt(format!("Overwrite {}?", outcome.write_path.display()))
                        .default(false)
                        .interact()?;

                    if !overwrite {
                        ui::info("Restore cancelled — no files were modified.");
                        ui::info("Tip: use --output <path> to restore to a different location.");
                        return Ok(());
                    }
                }

                commit_restore(outcome, &options)
            }
        }
    }
}
