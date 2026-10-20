//! Restore command — decrypt a backup created by `evnx backup`.
//!
//! # Overview
//!
//! Reads a Base64-encoded AES-256-GCM backup file, derives the decryption key
//! from the user-supplied password using Argon2id, decrypts the payload, and
//! writes the recovered `.env` content to disk.
//!
//! This file is the **CLI adapter**: it owns all TTY interaction — the
//! decorative header, the password prompt, the overwrite question and the
//! `.gitignore` question. Pure restore logic (decryption, path selection,
//! writing) lives in [`core`] and is independently testable without a terminal.
//!
//! # Module layout
//!
//! | Module   | Responsibility                                       |
//! |----------|------------------------------------------------------|
//! | `mod.rs` | CLI adapter — header, prompts, orchestration         |
//! | `core`   | Pure logic — decrypt, validate, write                |
//! | `source` | Reading the backup file                              |
//! | `error`  | Typed error variants and process exit code mapping   |
//!
//! # Safety behaviours
//!
//! ## Password resolution order
//!
//! [`crate::utils::secret_source`], the same rule `backup` uses:
//!
//! 1. `--password-file <path>` (also `--key-file`) — read from a file (CI/CD friendly).
//! 2. `EVNX_PASSWORD` environment variable — read and immediately unset.
//! 3. Interactive prompt — default; most secure.
//!
//! There is no length floor here (DEC-B2): a backup made before `backup`
//! required 8 characters still opens.
//!
//! ## Replacing an existing output
//!
//! Never silently. When the output already holds exactly what the backup
//! holds, nothing is asked or written ("already up to date", DEC-B12).
//! Otherwise, with a terminal, evnx asks (default No); without one, `--yes`
//! (alias `--force`, as in `cloud pull`) answers, and without it the run
//! stops with exit 2 naming `--yes` and `--output` (DEC-B1, I-148). Either way
//! the replaced file is kept as `<output>.bak` (DEC-B11). The output may not be
//! the backup itself (DEC-B3).
//!
//! ⚠️ The module header once said "there is intentionally no `--force`
//! flag". Without one a script had no way past an existing file but
//! `--output <new>` and `mv`, and with no terminal the question failed as
//! "IO error: not a terminal" — exit 2, not the documented 4.
//!
//! ## Validation failure fallback
//!
//! When the decrypted content is not a `.env` evnx can read, the file is
//! written to `<o>.restored` instead of `<o>`. The typed error
//! [`RestoreError::ValidationFallback`] is returned so `main.rs` can use exit
//! code 5 without printing a redundant error message.
//!
//! ## Memory safety
//!
//! The password and the decrypted content are [`zeroize::Zeroizing`] strings,
//! cleared on every exit path, including `?`-propagated errors and panics.
//! The encoded ciphertext blob is explicitly zeroized in this file after
//! `prepare_restore` returns.
//!
//! # Exit codes
//!
//! | Code | Meaning                                              |
//! |------|------------------------------------------------------|
//! | 0    | Success (or inspected, dry run, already up to date)  |
//! | 2    | Could not run or could not decrypt — "No verdict"    |
//! | 3    | Backup file not found                                |
//! | 4    | User cancelled overwrite prompt                      |
//! | 5    | Restored to `.restored` fallback (bad content)       |
//!
//! Exit 1 is never produced (DEC-B9). Exit 6 was "evnx-cloud restore not yet
//! available" and is retired with `cloud://` (DEC-B4).
//!
//! # A backup is not a vault
//!
//! `restore` reads files `evnx backup` wrote: a passphrase you chose, no
//! account, no network. A vault in evnx cloud is restored with
//! `evnx cloud pull --version N` (one version) or `evnx cloud export` (every
//! version). Neither reads the other's files.
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
//! # Non-interactive via env var (CI/CD), replacing an existing .env
//! EVNX_PASSWORD=mypass evnx restore .env.backup --yes
//!
//! # Non-interactive via password file
//! evnx restore .env.backup --password-file /run/secrets/evnx-pass --output .env.production
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

/// Everything `restore` takes besides the backup path.
#[derive(Debug, Clone)]
pub struct Options {
    /// Destination path (default `.env`).
    pub output: String,
    /// Emit a diagnostic line at each pipeline stage.
    pub verbose: bool,
    /// Decrypt and validate, but do not write any files.
    pub dry_run: bool,
    /// List variable names (never values), do not write.
    pub inspect: bool,
    /// Read the decryption password from this file instead of prompting
    /// (`EVNX_PASSWORD` is checked when this is `None`).
    pub password_file: Option<String>,
    /// Replace an existing output without asking (`--yes`, `--force`).
    pub yes: bool,
    /// The `.gitignore` step for the written file.
    pub gitignore: crate::utils::protect::GitignoreMode,
}

/// Entry point for the `restore` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly without returning an error.
pub fn run(backup: String, opts: Options) -> Result<()> {
    // ── Feature-disabled stub ─────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        let _ = (&backup, &opts);
        eprintln!("✗ Backup/restore feature not enabled");
        eprintln!("Rebuild with: cargo build --features backup");
        Ok(())
    }

    // ── Full implementation ───────────────────────────────────────────────────
    #[cfg(feature = "backup")]
    {
        use std::path::PathBuf;
        use zeroize::Zeroize;

        use self::core::{
            commit_restore, existing_output, finish, keep_previous, prepare_restore, Existing,
            PrepareResult, RestoreOptions,
        };
        use self::error::RestoreError;
        use crate::utils::secret_source::{self, Purpose};
        use crate::utils::ui;

        let Options {
            output,
            verbose,
            dry_run,
            inspect,
            password_file,
            yes,
            gitignore,
        } = opts;

        // ── Header ────────────────────────────────────────────────────────────
        ui::print_header(
            "evnx restore",
            Some("Decrypt and restore from an encrypted backup"),
        );

        if verbose {
            ui::verbose_stderr(format!("Source : {backup}"));
            ui::verbose_stderr(format!("Inspect: {inspect}"));
            ui::verbose_stderr(format!("Dry-run: {dry_run}"));
        }

        // ── Fetch encoded blob ────────────────────────────────────────────────
        let mut encoded = source::fetch(&backup)?;
        ui::success(format!("Read backup from {backup}"));

        // ── The output may not be the backup itself (DEC-B3) ──────────────────
        // Restoring a backup over itself replaces the only copy of the
        // ciphertext with the plaintext. `--inspect` and `--dry-run` write
        // nothing, so where `--output` points does not matter to them.
        if !inspect && !dry_run {
            let check = crate::core::safe_write::refuse_same_file(&output, "backup", &backup)
                .and_then(|()| crate::core::safe_write::preview(&output).map(drop));
            if let Err(e) = check {
                encoded.zeroize();
                return Err(e);
            }
        }

        // ── Password resolution ───────────────────────────────────────────────
        // Key file, then EVNX_PASSWORD, then the prompt — `backup`'s rule.
        let password = match secret_source::resolve(
            password_file.as_deref(),
            Purpose::Decrypt,
            &format!("evnx restore {backup}"),
            verbose,
        ) {
            Ok(pw) => pw,
            Err(e) => {
                encoded.zeroize();
                return Err(e);
            }
        };

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

        let outcome = match result? {
            // ── Inspect: key names printed, nothing written ───────────────────
            PrepareResult::Inspect => return Ok(()),

            // ── Dry-run: nothing to write ─────────────────────────────────────
            PrepareResult::DryRun => return Ok(()),

            PrepareResult::Ready(outcome) => outcome,
        };
        let path = outcome.write_path.display().to_string();

        // ── An existing output: identical, or replaced only on a yes ─────────
        match existing_output(&outcome)? {
            Existing::Absent => {}
            Existing::Same => {
                // DEC-B12 (S-B4): nothing to ask and nothing to write — and no
                // "next steps", which are about a file that was just written.
                ui::success(format!(
                    "{path} is already up to date — nothing was written"
                ));
                if !outcome.used_fallback {
                    ui::print_docs_hint(&crate::docs::RESTORE);
                    return Ok(());
                }
                // Still not a `.env` evnx can read: exit 5, with its steps.
                let restored = core::Restored {
                    path: outcome.write_path.clone(),
                    used_fallback: true,
                };
                return finish(&restored, &options);
            }
            Existing::Differs(previous) => {
                if !yes {
                    ui::warning(format!("Output file already exists: {path}"));
                    // I-148: dialoguer's "IO error: not a terminal" named
                    // neither way out.
                    crate::utils::prompt::require_terminal_or(&format!(
                        "Add --yes to replace it (the current file is kept as a .bak copy), \
                         or restore somewhere else with --output <path>:\n  \
                         evnx restore {backup} --yes"
                    ))?;
                    let overwrite = crate::utils::prompt::yes_no()
                        .with_prompt(format!(
                            "Overwrite {path}? (the current file is kept as a .bak copy)"
                        ))
                        .default(false)
                        .interact()?;

                    if !overwrite {
                        ui::info("Restore cancelled — no files were modified.");
                        ui::info("Tip: use --output <path> to restore to a different location.");
                        return Err(RestoreError::Cancelled.into());
                    }
                }
                // DEC-B11: what was there is kept, whichever way the yes came.
                let kept = keep_previous(&outcome, &previous)?;
                ui::info(format!("Kept the previous {path} as {}", kept.display()));
            }
        }

        let restored = commit_restore(outcome, &options)?;
        // The restored file holds plaintext: keep it out of git, the step
        // `template` and `convert -o` take (R-template).
        crate::utils::protect::protect_output(&restored.path.to_string_lossy(), &gitignore)?;
        finish(&restored, &options)
    }
}
