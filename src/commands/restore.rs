//! Restore command — decrypt a backup created by the `backup` command.
//!
//! # Overview
//!
//! Reads a Base64-encoded AES-256-GCM backup file, derives the decryption key
//! from the user-supplied password using Argon2id, decrypts the payload, and
//! writes the recovered `.env` content to disk.
//!
//! # Safety behaviours
//!
//! ## Password handling
//!
//! Unlike the `backup` command, `restore` uses a **single password prompt**
//! without confirmation. This is intentional: a typo during backup creation
//! could make data permanently unrecoverable, while a typo during restore
//! simply fails decryption (a safe, reversible error). Requiring confirmation
//! on restore would add friction without meaningful safety benefits.
//!
//!
//! ## Overwrite protection
//!
//! If the output file already exists the user is **always prompted** before it
//! is overwritten. There is no `--force` flag — this is intentional. The `.env`
//! file is sensitive; an accidental overwrite is difficult to undo and could
//! destroy credentials that are not backed up anywhere else.
//!
//! ## Validation failure fallback
//!
//! If the decrypted content does not look like a valid `.env` file (checked
//! heuristically — see [`looks_like_dotenv`]) the command writes the content
//! to `<output>.restored` instead of `<output>`. This lets the user inspect
//! the raw content without risking damage to a live `.env` file.
//!
//! # Example
//!
//! ```bash
//! evnx restore .env.backup
//! evnx restore .env.backup --output .env.production
//! evnx restore .env.backup --dry-run  # validate without writing
//! ```
//!
//! # Future work
//!
//! - `--print` flag: decrypt and emit to stdout (useful for piping).
//! - `--diff` flag: show a unified diff between backup content and current file.
//! - Support for asymmetric (public-key) backups once `--recipient` is added
//!   to the `backup` command.

// use crate::utils::{looks_like_dotenv, count_dotenv_vars};

use anyhow::{anyhow, Context, Result};
use colored::*;

/// Entry point for the `restore` subcommand.
///
/// When the `backup` feature is **not** enabled this prints a helpful message
/// and exits cleanly — it does **not** panic or return an error.
///
/// # Arguments
///
/// * `backup`  — Path to the `.backup` file created by `evnx backup`.
/// * `output`  — Desired destination path for the restored file (default `.env`).
///   If decrypted content fails `.env` validation this path receives
///   a `.restored` suffix to avoid overwriting a live file with
///   potentially corrupt content.
/// * `verbose` — Print extra diagnostic information during the run.
pub fn run(backup: String, output: String, verbose: bool, dry_run: bool) -> Result<()> {
    // ── Feature-disabled stub ────────────────────────────────────────────────
    #[cfg(not(feature = "backup"))]
    {
        // Reference all parameters to silence unused-variable warnings in
        // the non-feature build without renaming them.
        let _ = (&backup, &output, verbose);
        println!("{} Backup/restore feature not enabled", "✗".red());
        println!("Rebuild with: cargo build --features backup");
        return Ok(());
    }

    // ── Full implementation (feature = "backup") ─────────────────────────────
    #[cfg(feature = "backup")]
    {
        use crate::commands::backup::decrypt_content;
        use crate::docs;
        use crate::utils::ui;
        use dialoguer::{Confirm, Password};
        use std::fs;
        use std::path::Path;
        use zeroize::Zeroize;

        if verbose {
            println!("{}", "Running restore in verbose mode".dimmed());
        }

        println!(
            "\n{}",
            "┌─ Restore from encrypted backup ─────────────────────┐".cyan()
        );
        println!(
            "{}",
            "│ Your backup will be decrypted with AES-256-GCM      │".cyan()
        );
        println!(
            "{}\n",
            "└──────────────────────────────────────────────────────┘".cyan()
        );

        // ── Validate backup file exists ──────────────────────────────────────
        if !Path::new(&backup).exists() {
            return Err(anyhow!("Backup file not found: {}", backup));
        }

        let encoded = fs::read_to_string(&backup)
            .with_context(|| format!("Failed to read backup file: {}", backup))?;

        println!("{} Read backup from {}", "✓".green(), backup);

        // ── Password prompt ──────────────────────────────────────────────────
        // No echo — the password is never displayed or logged.
        let mut password = Password::new()
            .with_prompt("Enter decryption password")
            .interact()?;

        if password.is_empty() {
            password.zeroize();
            return Err(anyhow!("Password must not be empty"));
        }

        // ── Decrypt ──────────────────────────────────────────────────────────
        println!("Decrypting… (Argon2id key derivation in progress)");

        let (content, metadata) = decrypt_content(&encoded, &password)
            .with_context(|| format!("Failed to decrypt backup: {}", backup))?;

        // ── Dry-run mode: validate and exit early ────────────────────────────
        if dry_run {
            println!(
                "\n{}",
                "✓ Dry-run successful — no files were written"
                    .green()
                    .bold()
            );
            println!("\n{}", "Backup information:".bold());
            println!("  Schema version: v{}", metadata.schema_version);
            println!("  Original file : {}", metadata.original_file);
            println!("  Created at    : {}", metadata.created_at);
            println!("  Tool version  : evnx v{}", metadata.tool_version);
            println!(
                "  Variables     : {}",
                crate::utils::count_dotenv_vars(&content)
            );

            if crate::utils::looks_like_dotenv(&content) {
                println!(
                    "\n{} Decrypted content appears to be a valid .env file",
                    "✓".green()
                );
            } else {
                println!(
                    "\n{} Warning: Decrypted content does not look like a valid .env file",
                    "⚠️".yellow()
                );
            }

            password.zeroize();
            return Ok(());
        }

        println!("{} Decryption successful", "✓".green());
        password.zeroize();
        println!(
            "{} Decryption key cleared from memory",
            "✓".green().dimmed()
        );

        // ── Display metadata ─────────────────────────────────────────────────
        // Always show metadata so the user can confirm this is the right backup
        // before any file is written.
        println!("\n{}", "Backup information:".bold());
        println!("  Schema version: v{}", metadata.schema_version);
        println!("  Original file : {}", metadata.original_file);
        println!("  Created at    : {}", metadata.created_at);
        println!("  Tool version  : evnx v{}", metadata.tool_version);
        println!(
            "  Variables     : {}",
            crate::utils::count_dotenv_vars(&content)
        );

        // ── Validation — choose write path ───────────────────────────────────
        // If the decrypted content does not look like a .env file we redirect
        // to a `.restored` fallback rather than aborting. This allows the user
        // to inspect the content and decide what to do with it, while keeping
        // the live .env file untouched.
        let write_path: String = if crate::utils::looks_like_dotenv(&content) {
            output.clone()
        } else {
            let fallback = format!("{}.restored", output);
            println!(
                "\n{} Decrypted content does not look like a valid .env file.",
                "⚠️".yellow()
            );
            println!(
                "  Writing to {} instead of {} to protect your current file.",
                fallback, output
            );
            println!("  Inspect the file manually, then rename it if the content is correct:");
            println!("    mv {} {}", fallback, output);
            fallback
        };

        // ── Overwrite protection — always prompt ─────────────────────────────
        // There is intentionally no --force flag. Overwriting a live .env
        // without confirmation is far too easy to do accidentally, and the
        // consequences (lost credentials) are hard to undo.
        if Path::new(&write_path).exists() {
            println!(
                "\n{} Output file already exists: {}",
                "⚠️".yellow(),
                write_path
            );

            let overwrite = Confirm::new()
                .with_prompt(format!("Overwrite {}?", write_path))
                .default(false)
                .interact()?;

            if !overwrite {
                println!("{} Restore cancelled. No files were modified.", "ℹ️".cyan());
                println!("  Tip: use --output <path> to restore to a different location.");
                return Ok(());
            }
        }

        // ── Write ─────────────────────────────────────────────────────────────
        // fs::write(&write_path, &content)
        //     .with_context(|| format!("Failed to write restored file to {}", write_path))?;
        crate::utils::write_secure(&write_path, content.as_bytes())
            .with_context(|| format!("Failed to write restored file to {}", write_path))?;

        println!(
            "\n{} Restored {} variables to {}",
            "✓".green(),
            crate::utils::count_dotenv_vars(&content),
            write_path
        );

        // Context-aware next steps depending on whether we used the fallback.
        println!("\n{}", "Next steps:".bold());
        if write_path == output {
            println!("  1. Run: evnx validate --env {}", write_path);
            println!("  2. Verify your application starts correctly");
            println!("  3. Delete the backup file once you have confirmed the restore");
        } else {
            // Fallback path was used — guide the user through inspection.
            println!("  1. Inspect the restored file: cat {}", write_path);
            println!(
                "  2. If the content is correct, rename it: mv {} {}",
                write_path, output
            );
            println!("  3. Run: evnx validate --env {}", output);
        }
        ui::print_docs_hint(&docs::RESTORE);
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::utils::dotenv_validation;

    #[test]
    fn test_count_dotenv_vars_normal() {
        let content = "# Comment\nKEY=value\nOTHER=123\n\n# Another\nFOO=bar\n";
        assert_eq!(dotenv_validation::count_dotenv_vars(content), 3);
    }

    #[test]
    fn test_count_dotenv_vars_empty() {
        assert_eq!(dotenv_validation::count_dotenv_vars(""), 0);
        assert_eq!(
            dotenv_validation::count_dotenv_vars("# Only comments\n\n"),
            0
        );
    }

    #[test]
    fn test_count_dotenv_vars_only_assignments() {
        assert_eq!(dotenv_validation::count_dotenv_vars("A=1\nB=2\nC=3\n"), 3);
    }

    #[test]
    fn test_looks_like_dotenv_valid() {
        assert!(dotenv_validation::looks_like_dotenv(
            "KEY=value\nOTHER=123\n# comment\n"
        ));
    }

    #[test]
    fn test_looks_like_dotenv_empty() {
        assert!(dotenv_validation::looks_like_dotenv(""));
        assert!(dotenv_validation::looks_like_dotenv("   \n  "));
    }

    #[test]
    fn test_looks_like_dotenv_rejects_prose() {
        // Zero KEY=VALUE lines — well below the 80% threshold.
        assert!(!dotenv_validation::looks_like_dotenv(
            "This is a plain text file.\nIt contains no env vars.\nNone at all."
        ));
    }

    #[test]
    fn test_looks_like_dotenv_tolerates_some_noise() {
        // 4 valid lines, 1 invalid → 80% valid → should pass.
        let content = "KEY=value\nOTHER=123\n# comment\nFOO=bar\nnot-a-valid-line\n";
        assert!(dotenv_validation::looks_like_dotenv(content));
    }

    #[test]
    fn test_looks_like_dotenv_rejects_too_much_noise() {
        // 2 valid, 4 invalid → 33% valid → below threshold.
        let content = "KEY=value\nFOO=bar\nnot valid\nalso invalid\nstill wrong\nnope\n";
        assert!(!dotenv_validation::looks_like_dotenv(content));
    }

    //     #[test]
    // #[cfg(feature = "backup")]
    // fn test_dry_run_does_not_write_file() {
    //     use crate::commands::backup::encrypt_content;
    //     use tempfile::TempDir;

    //     // Create a valid backup first
    //     let dir = TempDir::new().unwrap();
    //     let env_path = dir.path().join(".env");
    //     let backup_path = dir.path().join("test.backup");

    //     std::fs::write(&env_path, "TEST_KEY=test_value\n").unwrap();

    //     // Encrypt (simplified - in real tests, call the actual backup flow)
    //     let encrypted = encrypt_content(
    //         "TEST_KEY=test_value\n",
    //         "testpass123",
    //         ".env"
    //     ).unwrap();
    //     std::fs::write(&backup_path, &encrypted).unwrap();

    //     // Run restore in dry-run mode
    //     let output_path = dir.path().join(".env.restored");
    //     let result = crate::commands::restore::run(
    //         backup_path.to_string_lossy().to_string(),
    //         output_path.to_string_lossy().to_string(),
    //         false,  // verbose
    //         true,   // dry_run ← key parameter
    //     );

    //     assert!(result.is_ok());
    //     // Verify file was NOT written
    //     assert!(!output_path.exists(), "dry-run should not create output file");
    // }
}
