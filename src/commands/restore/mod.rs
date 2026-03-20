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
//! | Module   | Responsibility                                       |
//! |----------|------------------------------------------------------|
//! | `mod.rs` | CLI adapter — header, prompts, orchestration         |
//! | `core`   | Pure logic — decrypt, validate, write                |
//! | `source` | Backup source — local path or cloud reference        |
//! | `error`  | Typed error variants and process exit code mapping   |
//!
//! # Safety behaviours
//!
//! ## Password resolution order
//!
//! Passwords are resolved in this priority order:
//!
//! 1. `--password-file <path>` — read from a file (CI/CD friendly).
//! 2. `EVNX_PASSWORD` environment variable — read and immediately unset.
//! 3. Interactive prompt — default; most secure.
//!
//! Options 1 and 2 are clearly less secure than the interactive prompt
//! (the password may appear in process listings, shell history, or CI logs)
//! and are documented as opt-in for non-interactive environments.
//!
//! ## Overwrite protection
//!
//! If the output file already exists the user is **always prompted** before
//! it is overwritten. There is intentionally no `--force` flag.
//!
//! ## Validation failure fallback
//!
//! When the decrypted content does not pass the `.env` validation heuristic,
//! the file is written to `<o>.restored` instead of `<o>`. The typed error
//! [`RestoreError::ValidationFallback`] is returned so `main.rs` can use exit
//! code 5 without printing a redundant error message.
//!
//! ## Memory safety
//!
//! The password is moved into a `ZeroizeOnDrop` guard in `core::prepare_restore`
//! the moment it is passed over — cleared on every exit path, including
//! `?`-propagated errors and panics. The encoded ciphertext blob is explicitly
//! zeroized in this file after `prepare_restore` returns.
//!
//! # Exit codes
//!
//! | Code | Meaning                                              |
//! |------|------------------------------------------------------|
//! | 0    | Success                                              |
//! | 1    | Generic error (IO, encoding, etc.)                   |
//! | 2    | Wrong password or corrupt backup                     |
//! | 3    | Backup file not found                                |
//! | 4    | User cancelled overwrite prompt                      |
//! | 5    | Restored to `.restored` fallback (bad content)       |
//! | 6    | evnx-cloud restore not yet available                 |
//!
//! # Examples
//!
//! ```bash
//! # Interactive (default)
//! evnx restore .env.backup
//!
//! # Inspect key names without restoring
//! evnx restore .env.backup --inspect
//!
//! # Non-interactive via env var (CI/CD)
//! EVNX_PASSWORD=mypass evnx restore .env.backup --output .env.production
//!
//! # Non-interactive via password file
//! evnx restore .env.backup --password-file /run/secrets/evnx-pass
//!
//! # Validate without writing
//! evnx restore .env.backup --dry-run
//! ```

#[cfg(feature = "backup")]
pub mod core;

#[cfg(feature = "backup")]
pub mod error;

#[cfg(feature = "backup")]
pub mod source;

#[cfg(feature = "backup")]
pub use error::RestoreError;

use anyhow::Result;

/// Entry point for the `restore` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly without returning an error.
///
/// # Arguments
///
/// * `backup`        — Local path or `cloud://project[/id]` reference.
/// * `output`        — Destination path (default `.env`).
/// * `verbose`       — Emit a diagnostic line at each pipeline stage.
/// * `dry_run`       — Decrypt and validate, but do not write any files.
/// * `inspect`       — List variable names (never values), do not write.
/// * `password_file` — Read the decryption password from this file instead
///   of prompting interactively. Less secure than the interactive prompt —
///   use only in non-interactive environments (CI/CD).
///   `EVNX_PASSWORD` env var is checked first if this is `None`.
pub fn run(
    backup: String,
    output: String,
    verbose: bool,
    dry_run: bool,
    inspect: bool,
    password_file: Option<String>,
) -> Result<()> {
    // ── Feature-disabled stub ─────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        let _ = (&backup, &output, verbose, dry_run, inspect, &password_file);
        eprintln!("✗ Backup/restore feature not enabled");
        eprintln!("Rebuild with: cargo build --features backup");
        Ok(())
    }

    // ── Full implementation ───────────────────────────────────────────────────
    #[cfg(feature = "backup")]
    {
        use anyhow::anyhow;
        use dialoguer::Confirm;
        use std::path::PathBuf;
        use zeroize::Zeroize;

        use self::core::{commit_restore, prepare_restore, PrepareResult, RestoreOptions};
        use self::error::RestoreError;
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
            ui::verbose_stderr(format!("Source : {}", src.display_path()));
            ui::verbose_stderr(format!("Inspect: {inspect}"));
            ui::verbose_stderr(format!("Dry-run: {dry_run}"));
        }

        // ── Fetch encoded blob ────────────────────────────────────────────────
        let mut encoded = src.fetch()?;
        ui::success(format!("Read backup from {}", src.display_path()));

        // ── Password resolution ───────────────────────────────────────────────
        // Priority: --password-file > EVNX_PASSWORD > interactive prompt.
        let password = resolve_password(password_file.as_deref(), verbose)?;

        if password.is_empty() {
            encoded.zeroize();
            return Err(anyhow!("Password must not be empty"));
        }

        // ── Core restore logic ────────────────────────────────────────────────
        let options = RestoreOptions {
            output: PathBuf::from(&output),
            inspect,
            dry_run,
            verbose,
        };

        let result = prepare_restore(&encoded, password, &options);

        // Zeroize the ciphertext blob unconditionally.
        encoded.zeroize();

        match result? {
            // ── Inspect: key names printed, nothing written ───────────────────
            PrepareResult::Inspect => Ok(()),

            // ── Dry-run: nothing to write ─────────────────────────────────────
            PrepareResult::DryRun => Ok(()),

            // ── Normal path: check for overwrite then write ───────────────────
            PrepareResult::Ready(outcome) => {
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
                        return Err(RestoreError::Cancelled.into());
                    }
                }

                commit_restore(outcome, &options)
            }
        }
    }
}

// ─── Password resolution ──────────────────────────────────────────────────────

/// Resolve the decryption password from, in priority order:
///
/// 1. `--password-file <path>` — file contents, trailing newline stripped.
/// 2. `EVNX_PASSWORD` env var — read and immediately removed from the
///    process environment to reduce the exposure window.
/// 3. Interactive prompt (default) — no echo, most secure.
///
/// # Security notes
///
/// - `--password-file` and `EVNX_PASSWORD` are less secure than the
///   interactive prompt: the password may appear in process listings, shell
///   history, or CI log output. Both options print a warning to stderr.
/// - `EVNX_PASSWORD` is removed via [`std::env::remove_var`] immediately
///   after reading. This is best-effort: the value was already in the
///   process environment and may have been captured by monitoring tools.
///
/// # Errors
///
/// Returns an error if the password file cannot be read.
/// An empty password is returned as-is and rejected by the caller.
#[cfg(feature = "backup")]
fn resolve_password(password_file: Option<&str>, verbose: bool) -> anyhow::Result<String> {
    use crate::utils::ui;
    use dialoguer::Password;

    // ── 1. --password-file ────────────────────────────────────────────────────
    if let Some(path) = password_file {
        ui::warning(format!(
            "Reading password from file: {path}\n  \
             Less secure than interactive prompt — avoid in shared environments."
        ));
        if verbose {
            ui::verbose_stderr(format!("Password source: --password-file {path}"));
        }

        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read password file '{}': {}", path, e))?;

        // Strip a single trailing newline — editors commonly add one.
        let pw = raw.trim_end_matches('\n').trim_end_matches('\r').to_owned();
        return Ok(pw);
    }

    // ── 2. EVNX_PASSWORD env var ──────────────────────────────────────────────
    if let Ok(pw) = std::env::var("EVNX_PASSWORD") {
        ui::warning(
            "Reading password from EVNX_PASSWORD environment variable.\n  \
             Less secure than interactive prompt — avoid in shared environments.",
        );
        if verbose {
            ui::verbose_stderr("Password source: EVNX_PASSWORD env var");
        }
        // Best-effort removal: reduces window but cannot guarantee it was
        // not already captured by the shell, a process monitor, or CI logs.
        std::env::remove_var("EVNX_PASSWORD");
        return Ok(pw);
    }

    // ── 3. Interactive prompt ─────────────────────────────────────────────────
    if verbose {
        ui::verbose_stderr("Password source: interactive prompt");
    }
    let pw = Password::new()
        .with_prompt("Enter decryption password")
        .interact()?;
    Ok(pw)
}
