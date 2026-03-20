//! Pure restore logic — decryption, path selection, and file writing.
//!
//! No TTY interaction occurs here. All prompts (password, overwrite
//! confirmation) are handled by the CLI adapter in [`mod.rs`](super).
//! Every function in this module is independently testable without a terminal.
//!
//! # Pipeline
//!
//! ```text
//! encoded blob (&str)  +  password (String)  +  RestoreOptions
//!         │
//!         ▼
//!   prepare_restore()
//!         │
//!         ├─ ZeroizeOnDrop guard wraps password immediately
//!         ├─ spinner starts          (suppressed in verbose mode)
//!         ├─ decrypt_content()       (Argon2id + AES-256-GCM)
//!         ├─ spinner stops
//!         ├─ print_metadata()        (always — dry-run and normal paths)
//!         │
//!         ├─[dry_run = true]──────► validate → print → zeroize → DryRun
//!         │
//!         └─[dry_run = false]─────► choose_write_path()
//!                                          │
//!                                          ▼
//!                                    PrepareResult::Ready(RestoreOutcome)
//!                                          │
//!                             (mod.rs prompts for overwrite if needed)
//!                                          │
//!                                          ▼
//!                                    commit_restore()
//!                                          │
//!                                    write_secure() → zeroize content
//!                                          │
//!                                    Ok(()) — or Err(ValidationFallback)
//!                                    when .restored path was used
//! ```
//!
//! # Error types
//!
//! Failures that deserve a distinct exit code are returned as
//! [`RestoreError`](super::error::RestoreError) variants wrapped in
//! `anyhow::Error`. Generic IO errors propagate as plain `anyhow` errors
//! and map to exit code 1 in `main.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::*;
use zeroize::Zeroize;

use crate::utils::ui;

use super::error::RestoreError;

// ─── Options ──────────────────────────────────────────────────────────────────

/// Configuration for a restore operation.
///
/// Constructed by the CLI adapter (`mod.rs`) from parsed CLI arguments and
/// passed into [`prepare_restore`]. Adding future flags here is additive —
/// existing call sites do not require changes.
#[derive(Debug, Clone)]
pub struct RestoreOptions {
    /// Destination path for the restored `.env` file.
    ///
    /// A `.restored` suffix is appended automatically when the decrypted
    /// content fails `looks_like_dotenv` validation, to protect any
    /// existing live file.
    pub output: PathBuf,

    /// If `true`, decrypt and validate without writing any files.
    pub dry_run: bool,

    /// If `true`, emit a diagnostic message at each pipeline stage via
    /// [`ui::verbose_stderr`].
    ///
    /// In verbose mode the Argon2id spinner is suppressed so that the
    /// per-step diagnostic lines are not interleaved with spinner output.
    pub verbose: bool,

    // ── Planned flags — not yet wired to the CLI ─────────────────────────────
    // pub print_to_stdout: bool,        // --print  (decrypt and emit to stdout)
    // pub select: Option<Vec<String>>,  // --select KEY1,KEY2 (partial restore)
    // pub merge: bool,                  // --merge (add missing keys, prompt on conflict)
}

// ─── Result types ─────────────────────────────────────────────────────────────

/// The two possible outcomes from a successful [`prepare_restore`] call.
#[derive(Debug)]
pub enum PrepareResult {
    /// `--dry-run` was requested; decryption and validation succeeded, but no
    /// file was written.
    DryRun,

    /// Decryption succeeded and the output is ready to be written.
    ///
    /// The caller (`mod.rs`) should check whether `outcome.write_path` already
    /// exists, prompt the user for confirmation if so, and then call
    /// [`commit_restore`].
    Ready(RestoreOutcome),
}

/// Everything needed to write the restored file.
///
/// Returned by [`prepare_restore`] when not in dry-run mode. The `content`
/// field is sensitive and **must** be consumed by [`commit_restore`], which
/// zeroizes it after writing.
#[derive(Debug)]
pub struct RestoreOutcome {
    /// Resolved destination path.
    ///
    /// Equals `options.output` when content passed validation, or
    /// `options.output + ".restored"` when the fallback was triggered.
    pub write_path: PathBuf,

    /// `true` when `write_path` differs from `options.output` because the
    /// decrypted content did not pass the `.env` validation heuristic.
    pub used_fallback: bool,

    /// Decrypted `.env` content — consumed and zeroized by [`commit_restore`].
    pub(crate) content: String,

    /// Metadata embedded in the backup by `evnx backup`.
    pub metadata: crate::commands::backup::BackupMetadata,
}

// ─── RAII password guard ──────────────────────────────────────────────────────

/// Owns a `String` and zeroizes it on drop, covering every exit path:
/// normal return, `?`-propagated error, and panic.
///
/// Implements `Deref<Target = str>` so the inner value can be borrowed
/// as `&str` without moving out.
struct ZeroizeOnDrop(String);

impl Drop for ZeroizeOnDrop {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::ops::Deref for ZeroizeOnDrop {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

// ─── Core functions ───────────────────────────────────────────────────────────

/// Decrypt the backup blob and prepare for writing.
///
/// This is the primary testable entry point for restore logic. The caller
/// is responsible only for fetching `encoded` from [`source`](super::source)
/// and prompting for the `password` — no other state is needed.
///
/// # Steps
///
/// 1. Moves `password` into a [`ZeroizeOnDrop`] guard that fires on every
///    exit path, including `?`-propagated errors and panics.
/// 2. Starts an Argon2id progress spinner (suppressed when `verbose = true`
///    to avoid interleaving with diagnostic output).
/// 3. Decrypts `encoded` with Argon2id key derivation + AES-256-GCM.
/// 4. Stops the spinner unconditionally, whether decryption succeeded or failed.
/// 5. Prints backup metadata via [`ui::print_key_value`].
/// 6. If `options.dry_run`: validates content, prints result, zeroizes content,
///    returns [`PrepareResult::DryRun`].
/// 7. Otherwise: resolves the write path (with `.restored` fallback on
///    validation failure) and returns [`PrepareResult::Ready`].
///
/// # Security notes
///
/// - `password` is zeroized via `ZeroizeOnDrop` regardless of outcome.
/// - A decryption failure returns [`RestoreError::WrongPassword`]; the
///   underlying AES-GCM error is logged in verbose mode but not exposed to
///   the user (avoids leaking implementation details).
/// - GCM authentication failures return immediately with no artificial delay.
///   Timing side-channels are not a practical threat for a local CLI tool.
///
/// # Errors
///
/// - [`RestoreError::WrongPassword`] — decryption failed (wrong password or
///   corrupt backup).
/// - Other `anyhow` errors — unexpected IO or encoding failures.
pub fn prepare_restore(
    encoded: &str,
    password: String,
    options: &RestoreOptions,
) -> Result<PrepareResult> {
    // ── Zeroize password on every exit path ───────────────────────────────────
    // ZeroizeOnDrop fires even on panic, which scattered `.zeroize()` calls
    // cannot guarantee.
    let pw_guard = ZeroizeOnDrop(password);

    if options.verbose {
        ui::verbose_stderr("Restore pipeline starting");
        ui::verbose_stderr(format!("Output path  : {}", options.output.display()));
        ui::verbose_stderr(format!("Dry-run      : {}", options.dry_run));
        // In verbose mode a static line replaces the spinner so diagnostic
        // output is not interleaved with spinner control sequences.
        ui::verbose_stderr("Argon2id key derivation in progress…");
    }

    // ── Spinner ───────────────────────────────────────────────────────────────
    // Shown only when verbose is off. The KDF is deliberately slow (designed
    // to take ~1 s on modern hardware) — without feedback users may assume the
    // tool has hung.
    let spinner = if options.verbose {
        None
    } else {
        Some(ui::spinner("Decrypting… (Argon2id key derivation in progress)"))
    };

    // ── Decrypt ───────────────────────────────────────────────────────────────
    let decrypt_result =
        crate::commands::backup::decrypt_content(encoded, &pw_guard).map_err(|e| {
            if options.verbose {
                // Log the underlying cryptographic detail before replacing it
                // with the user-friendly typed error.
                ui::verbose_stderr(format!("Decrypt error detail: {e:#}"));
            }
            anyhow::Error::from(RestoreError::WrongPassword)
        });

    // Stop the spinner unconditionally — before any further output and before
    // propagating a potential error, so it is always cleaned up.
    if let Some(sp) = spinner {
        sp.finish_and_clear();
    }

    // Now propagate a decryption failure with the typed error attached.
    let (mut content, metadata) = decrypt_result?;
    // pw_guard remains in scope until the end of this function.

    if options.verbose {
        ui::verbose_stderr(format!(
            "Decrypted successfully — {} variable(s)",
            crate::utils::count_dotenv_vars(&content),
        ));
        ui::verbose_stderr(format!("Backup schema : v{}", metadata.schema_version));
    }

    // ── Metadata ──────────────────────────────────────────────────────────────
    // Shown on both dry-run and normal paths so the user can confirm this is
    // the correct backup before any file is written.
    print_metadata(&metadata, &content);

    // ── Dry-run ───────────────────────────────────────────────────────────────
    if options.dry_run {
        print_validation_result(crate::utils::looks_like_dotenv(&content));
        // Best-effort: reduce the window where plaintext sits in heap memory.
        content.zeroize();
        println!(
            "\n{}",
            "✓ Dry-run complete — no files were written".green().bold()
        );
        return Ok(PrepareResult::DryRun);
    }

    // ── Success message ───────────────────────────────────────────────────────
    ui::success("Decryption successful");
    println!(
        "{}",
        format!("  {} Decryption key cleared from memory", "✓".green()).dimmed()
    );

    // ── Choose write path ─────────────────────────────────────────────────────
    let (write_path, used_fallback) =
        choose_write_path(&options.output, &content, options.verbose);

    Ok(PrepareResult::Ready(RestoreOutcome {
        write_path,
        used_fallback,
        content,
        metadata,
    }))
    // pw_guard drops here → password zeroized
}

/// Write the decrypted content to `outcome.write_path` and print confirmation.
///
/// Called by `mod.rs` **after** the user has confirmed any overwrite prompt.
/// Zeroizes `outcome.content` after writing to reduce the window where
/// plaintext credentials sit in heap memory.
///
/// # Return value
///
/// Returns `Ok(())` when the content was written to the requested output path.
/// Returns `Err(`[`RestoreError::ValidationFallback`]`)` when the `.restored`
/// fallback path was used — the write still succeeded, but the typed error
/// signals to `main.rs` that exit code 5 should be used. All inline messages
/// (path, next steps, docs hint) are printed before the error is returned.
///
/// # Errors
///
/// - [`RestoreError::ValidationFallback`] — write succeeded but content failed
///   `.env` validation; exit code 5.
/// - Other `anyhow` errors — IO failure writing the file.
pub fn commit_restore(mut outcome: RestoreOutcome, options: &RestoreOptions) -> Result<()> {
    // Capture var count before zeroize so it can appear in the success message.
    let var_count = crate::utils::count_dotenv_vars(&outcome.content);

    let path_str = outcome.write_path.to_str().with_context(|| {
        format!(
            "Output path contains non-UTF-8 characters: {:?}",
            outcome.write_path
        )
    })?;

    crate::utils::write_secure(path_str, outcome.content.as_bytes()).with_context(|| {
        format!(
            "Failed to write restored file to {}",
            outcome.write_path.display()
        )
    })?;

    // Best-effort zeroize: eliminates the most obvious heap reference to the
    // plaintext. Copies made internally by the allocator may persist, but
    // this meaningfully reduces the exposure window.
    outcome.content.zeroize();

    if options.verbose {
        ui::verbose_stderr("Plaintext content zeroized from memory");
    }

    println!(
        "\n{} Restored {} variable(s) to {}",
        "✓".green(),
        var_count,
        outcome.write_path.display(),
    );

    print_next_steps(&outcome.write_path, &options.output, outcome.used_fallback);
    ui::print_docs_hint(&crate::docs::RESTORE);

    // Signal to main.rs that exit code 5 should be used when the fallback
    // path was triggered. All output has already been printed above, so
    // main.rs should suppress the error message for this variant (see
    // RestoreError::is_silent).
    if outcome.used_fallback {
        return Err(RestoreError::ValidationFallback {
            fallback_path: outcome.write_path.display().to_string(),
        }
        .into());
    }

    Ok(())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Determine where to write the restored file.
///
/// Returns `(write_path, used_fallback)`.
///
/// When `content` passes the `looks_like_dotenv` heuristic the requested
/// `output` path is returned unchanged (`used_fallback = false`). When it
/// fails, a `.restored` suffix is appended and a warning is printed, so the
/// user can inspect the raw content without risking damage to a live `.env`.
fn choose_write_path(output: &Path, content: &str, verbose: bool) -> (PathBuf, bool) {
    if crate::utils::looks_like_dotenv(content) {
        if verbose {
            ui::verbose_stderr(format!(
                "Content validated — writing to {}",
                output.display()
            ));
        }
        return (output.to_path_buf(), false);
    }

    let fallback = PathBuf::from(format!("{}.restored", output.display()));

    ui::warning("Decrypted content does not look like a valid .env file.");
    println!(
        "  Writing to {} instead of {} to protect your current file.",
        fallback.display(),
        output.display(),
    );
    println!("  Inspect the file manually, then rename it if the content looks correct:");
    println!("    mv {} {}", fallback.display(), output.display());

    if verbose {
        ui::verbose_stderr(format!("Fallback path: {}", fallback.display()));
    }

    (fallback, true)
}

/// Print backup metadata in an aligned key-value block.
///
/// Uses [`ui::print_section_header`] and [`ui::print_key_value`] for
/// consistent formatting with the rest of the tool. Displayed on both
/// the dry-run and normal paths.
fn print_metadata(metadata: &crate::commands::backup::BackupMetadata, content: &str) {
    let schema = format!("v{}", metadata.schema_version);
    let tool_ver = format!("evnx v{}", metadata.tool_version);
    let var_count = crate::utils::count_dotenv_vars(content).to_string();

    ui::print_section_header("📦", "Backup information");
    ui::print_key_value(&[
        ("Schema version", &schema),
        ("Original file", &metadata.original_file),
        ("Created at", &metadata.created_at),
        ("Tool version", &tool_ver),
        ("Variables", &var_count),
    ]);
}

/// Print a validation result line after the metadata block (dry-run path).
fn print_validation_result(valid: bool) {
    if valid {
        println!(
            "\n{} Decrypted content appears to be a valid .env file",
            "✓".green()
        );
    } else {
        ui::warning("Decrypted content does not look like a valid .env file");
    }
}

/// Print context-aware next steps after a successful write.
///
/// When the fallback path was used the steps guide the user through
/// inspection and manual rename. Otherwise they proceed directly to
/// `evnx validate`.
fn print_next_steps(write_path: &Path, output: &Path, used_fallback: bool) {
    println!("\n{}", "Next steps:".bold());
    if used_fallback {
        println!("  1. Inspect the restored file:  cat {}", write_path.display());
        println!(
            "  2. If the content is correct:  mv {} {}",
            write_path.display(),
            output.display(),
        );
        println!(
            "  3. Validate the result:        evnx validate --env {}",
            output.display()
        );
    } else {
        println!("  1. Run: evnx validate --env {}", write_path.display());
        println!("  2. Verify your application starts correctly");
        println!("  3. Delete the backup file once you have confirmed the restore");
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::dotenv_validation;

    // ── ZeroizeOnDrop ─────────────────────────────────────────────────────────

    #[test]
    fn zeroize_guard_deref_gives_inner_str() {
        let guard = ZeroizeOnDrop("secret-value".to_owned());
        assert_eq!(&*guard, "secret-value");
    }

    #[test]
    fn zeroize_guard_drops_without_panic() {
        let guard = ZeroizeOnDrop("sensitive".to_owned());
        drop(guard);
    }

    // ── choose_write_path ─────────────────────────────────────────────────────

    #[test]
    fn valid_content_uses_requested_output_path() {
        let output = PathBuf::from(".env");
        let (path, used_fallback) =
            choose_write_path(&output, "KEY=value\nOTHER=123\n", false);
        assert_eq!(path, output);
        assert!(!used_fallback);
    }

    #[test]
    fn invalid_content_redirects_to_restored_fallback() {
        let output = PathBuf::from(".env");
        let prose = "This is plain prose.\nNo variables here.\nNot a dotenv file at all.\n";
        let (path, used_fallback) = choose_write_path(&output, prose, false);
        assert_eq!(path, PathBuf::from(".env.restored"));
        assert!(used_fallback);
    }

    #[test]
    fn empty_content_uses_requested_output_path() {
        // Empty files are considered valid (nothing to restore, but not corrupt).
        let output = PathBuf::from(".env");
        let (path, used_fallback) = choose_write_path(&output, "", false);
        assert_eq!(path, output);
        assert!(!used_fallback);
    }

    // ── looks_like_dotenv ─────────────────────────────────────────────────────

    #[test]
    fn looks_like_dotenv_accepts_valid_content() {
        assert!(dotenv_validation::looks_like_dotenv(
            "KEY=value\nOTHER=123\n# comment\n"
        ));
    }

    #[test]
    fn looks_like_dotenv_accepts_empty_string() {
        assert!(dotenv_validation::looks_like_dotenv(""));
        assert!(dotenv_validation::looks_like_dotenv("   \n  "));
    }

    #[test]
    fn looks_like_dotenv_rejects_pure_prose() {
        assert!(!dotenv_validation::looks_like_dotenv(
            "This is a plain text file.\nIt contains no env vars.\nNone at all."
        ));
    }

    #[test]
    fn looks_like_dotenv_tolerates_minority_of_noise_lines() {
        assert!(dotenv_validation::looks_like_dotenv(
            "KEY=value\nOTHER=123\n# comment\nFOO=bar\nnot-a-valid-line\n"
        ));
    }

    #[test]
    fn looks_like_dotenv_rejects_majority_noise() {
        assert!(!dotenv_validation::looks_like_dotenv(
            "KEY=value\nFOO=bar\nnot valid\nalso invalid\nstill wrong\nnope\n"
        ));
    }

    // ── count_dotenv_vars ─────────────────────────────────────────────────────

    #[test]
    fn count_vars_in_normal_file() {
        let content = "# Comment\nKEY=value\nOTHER=123\n\n# Another\nFOO=bar\n";
        assert_eq!(dotenv_validation::count_dotenv_vars(content), 3);
    }

    #[test]
    fn count_vars_empty_and_comments_only() {
        assert_eq!(dotenv_validation::count_dotenv_vars(""), 0);
        assert_eq!(
            dotenv_validation::count_dotenv_vars("# Only comments\n\n"),
            0
        );
    }

    #[test]
    fn count_vars_only_assignments() {
        assert_eq!(dotenv_validation::count_dotenv_vars("A=1\nB=2\nC=3\n"), 3);
    }

    // ── Integration tests (feature = "backup") ────────────────────────────────

    #[test]
    #[cfg(feature = "backup")]
    fn dry_run_decrypts_correctly_and_writes_nothing() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let env_content = "TEST_KEY=test_value\nANOTHER_VAR=123\n";
        let backup_path = dir.path().join("test.backup");

        let encoded =
            crate::commands::backup::encrypt_content(env_content, "testpass123", ".env")
                .expect("encrypt_content should succeed");
        std::fs::write(&backup_path, &encoded).unwrap();

        let encoded_str = std::fs::read_to_string(&backup_path).unwrap();
        let output_path = dir.path().join(".env");
        let options = RestoreOptions {
            output: output_path.clone(),
            dry_run: true,
            verbose: false,
        };

        let result = prepare_restore(&encoded_str, "testpass123".to_owned(), &options);

        assert!(result.is_ok(), "prepare_restore failed: {:?}", result.err());
        assert!(
            !output_path.exists(),
            "dry-run must not create the output file"
        );
        assert!(
            matches!(result.unwrap(), PrepareResult::DryRun),
            "expected PrepareResult::DryRun"
        );
    }

    #[test]
    #[cfg(feature = "backup")]
    fn wrong_password_returns_wrong_password_error_with_exit_code_2() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let backup_path = dir.path().join("test.backup");

        let encoded =
            crate::commands::backup::encrypt_content("KEY=val\n", "correct_password", ".env")
                .expect("encrypt_content should succeed");
        std::fs::write(&backup_path, &encoded).unwrap();

        let encoded_str = std::fs::read_to_string(&backup_path).unwrap();
        let options = RestoreOptions {
            output: dir.path().join(".env"),
            dry_run: false,
            verbose: false,
        };

        let result = prepare_restore(&encoded_str, "wrong_password".to_owned(), &options);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let typed = err.downcast_ref::<RestoreError>();
        assert!(
            matches!(typed, Some(RestoreError::WrongPassword)),
            "expected RestoreError::WrongPassword, got: {err}"
        );
        assert_eq!(typed.unwrap().exit_code(), 2);
    }

    #[test]
    #[cfg(feature = "backup")]
    fn prepare_and_commit_round_trip_writes_correct_content() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let env_content = "ROUND_TRIP=yes\nFOO=bar\n";
        let backup_path = dir.path().join("test.backup");
        let output_path = dir.path().join(".env");

        let encoded =
            crate::commands::backup::encrypt_content(env_content, "roundtrippass", ".env")
                .expect("encrypt_content should succeed");
        std::fs::write(&backup_path, &encoded).unwrap();

        let encoded_str = std::fs::read_to_string(&backup_path).unwrap();
        let options = RestoreOptions {
            output: output_path.clone(),
            dry_run: false,
            verbose: false,
        };

        let result =
            prepare_restore(&encoded_str, "roundtrippass".to_owned(), &options)
                .expect("prepare_restore should succeed");

        let outcome = match result {
            PrepareResult::Ready(o) => o,
            PrepareResult::DryRun => panic!("expected Ready, got DryRun"),
        };

        commit_restore(outcome, &options).expect("commit_restore should succeed");

        let written = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(written, env_content, "written content must match original");
    }

    #[test]
    #[cfg(feature = "backup")]
    fn commit_restore_returns_validation_fallback_error_for_non_dotenv_content() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Prose content that will fail the .env heuristic.
        let prose_content =
            "This is plain prose.\nNo variables here.\nNot a dotenv file.\nStill not.\nNope.\nNope.\n";
        let backup_path = dir.path().join("test.backup");
        let output_path = dir.path().join(".env");

        let encoded =
            crate::commands::backup::encrypt_content(prose_content, "pass", ".env")
                .expect("encrypt_content should succeed");
        std::fs::write(&backup_path, &encoded).unwrap();

        let encoded_str = std::fs::read_to_string(&backup_path).unwrap();
        let options = RestoreOptions {
            output: output_path.clone(),
            dry_run: false,
            verbose: false,
        };

        let result = prepare_restore(&encoded_str, "pass".to_owned(), &options)
            .expect("prepare_restore should succeed despite bad content");

        let outcome = match result {
            PrepareResult::Ready(o) => o,
            PrepareResult::DryRun => panic!("expected Ready, got DryRun"),
        };

        // commit_restore should succeed (file written) but return
        // ValidationFallback to signal exit code 5.
        let commit_result = commit_restore(outcome, &options);
        assert!(commit_result.is_err());

        let err = commit_result.unwrap_err();
        let typed = err.downcast_ref::<RestoreError>();
        assert!(
            matches!(typed, Some(RestoreError::ValidationFallback { .. })),
            "expected RestoreError::ValidationFallback, got: {err}"
        );
        assert_eq!(typed.unwrap().exit_code(), 5);
        assert!(typed.unwrap().is_silent(), "ValidationFallback should be silent");

        // The fallback file should exist on disk.
        let fallback = dir.path().join(".env.restored");
        assert!(fallback.exists(), ".env.restored should have been written");

        // The original output path should NOT have been touched.
        assert!(
            !output_path.exists(),
            ".env should not exist when fallback was used"
        );
    }
}