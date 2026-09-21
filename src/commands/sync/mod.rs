//! Sync command: keep `.env` and `.env.example` in sync.
//!
//! This module provides the public API for the sync command.
//! Internal implementation is organized into submodules for maintainability.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::{NamingPolicy, SyncDirection};

mod executor;
mod models;
mod placeholder;
mod security;

// Re-export public types for external use (if needed)
pub use models::{PlaceholderConfig, SyncAction, SyncPreview, VarChange};

/// In sync — nothing to do.
pub const EXIT_IN_SYNC: i32 = 0;
/// `--check` only: the files are out of step.
pub const EXIT_OUT_OF_SYNC: i32 = 1;
/// `--check` only: the command could not reach a verdict.
pub const EXIT_ERROR: i32 = 2;

/// Public entry point called from main.rs.
///
/// # `--check` and its three exit codes
///
/// | code | meaning |
/// |------|---------|
/// | `0`  | in sync |
/// | `1`  | **out of sync** — the assertion failed |
/// | `2`  | **error** — missing file, parse failure, bad config |
///
/// The three states exist because two of them were previously indistinguishable.
/// `--check` returned `1` for "your template is stale" *and* for "`.env` is
/// missing" *and* for "the template will not parse", so a CI gate could fail the
/// build without being able to say which had happened — and
/// `evnx sync --check || true`, the documented advisory idiom, swallowed genuine
/// breakage along with the signal it meant to ignore.
///
/// `0 / 1 / 2` is the POSIX shape, and the reason `diff` and `grep` both use it:
///
/// ```text
/// diff   same → 0    different → 1    trouble → 2
/// grep   match → 0   no match  → 1    trouble → 2
/// ```
///
/// ⚠️ **The contract is opt-in.** Without `--check` nothing changes: errors still
/// propagate to `main` and exit `1` exactly as before, and `--dry-run` still
/// previews and exits `0` whatever it finds. Terraform draws the same line with
/// `-detailed-exitcode`, and for the same reason — silently re-numbering the exit
/// codes of a command people already script would be a breaking change dressed up
/// as a fix.
///
/// # Why `--dry-run` is kept
///
/// `--check` and `--dry-run` currently print byte-identical output and differ only
/// in the exit code, so the case for dropping one is real. Two things keep it:
/// `--dry-run` has shipped since before v0.4.0 and appears throughout the guides;
/// and `evnx sync --check || true` cannot distinguish `1` from `2`, so a user who
/// only wants the preview needs a flag that never reports state through the exit
/// code at all.
// 8 flags. `SyncArgs` in `cli.rs` already holds exactly this set, so the tidier
// signature is to pass that struct — but it would rewrite every call site for no
// behavioural gain, so it is queued for the v0.5.0 command review instead.
// `validate::run` carries the same allow for the same reason.
#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    example: String,
    direction: SyncDirection,
    placeholder: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    check: bool,
    format: String,
    template_config: Option<PathBuf>,
    naming_policy: NamingPolicy,
) -> Result<()> {
    let paths = executor::SyncPaths {
        env: PathBuf::from(env),
        example: PathBuf::from(example),
    };

    // ⚠️ `--format json` is accepted only with `--check`. Without it `sync`
    // edits files, and a machine-readable report of an edit already made is
    // worth less than the edit — clap enforces the pairing, this is the
    // behaviour behind it.
    if format == "json" {
        // ⚠️ The same three codes the pretty path uses, and for the same reason:
        // under `--check` an error is a *different answer*, not a louder "out of
        // sync". Propagating the `Err` would exit 1, and a pipeline branching on
        // the code would read an unparseable template as drift.
        return match check_as_json(&paths, direction) {
            Ok(false) => Ok(()),
            Ok(true) => std::process::exit(EXIT_OUT_OF_SYNC),
            Err(e) => {
                eprintln!("evnx could not check: {e:#}");
                std::process::exit(EXIT_ERROR);
            }
        };
    }

    let ctx = executor::SyncCtx {
        paths,
        direction,
        placeholder,
        verbose,
        // `--check` is a dry run that reports its verdict through the exit code.
        dry_run: dry_run || check,
        force,
        template_config,
        naming_policy,
    };

    match executor::execute(ctx) {
        Ok(out_of_step) => {
            if check && out_of_step {
                eprintln!();
                eprintln!(
                    "✗ Out of sync. Run `evnx sync --direction {direction}` and commit the result."
                );
                std::process::exit(EXIT_OUT_OF_SYNC);
            }
            Ok(())
        }
        // Under `--check` an error is a *different answer*, not a louder version
        // of "out of sync" — so it gets its own code and prints here rather than
        // propagating to `main`, which would collapse it back onto 1.
        Err(e) => {
            if check {
                eprintln!();
                eprintln!("✗ evnx could not check: {e:#}");
                std::process::exit(EXIT_ERROR);
            }
            Err(e)
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Tests (Only if you want to test public API here)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // SyncDirection is defined in cli.rs, so test it there instead
    // But if you want to verify the run() function signature compiles:

    /// Pins the public signature so a change to it has to be deliberate.
    ///
    /// ⚠️ Updated 2026-09-21 again: `env` and `example` lead the list. `sync` was
    /// the only command that could not be pointed at a file, so the two paths it
    /// operates on are now arguments rather than 37 literals in the executor.
    ///
    /// ⚠️ Updated 2026-09-21 twice. First `check: bool` was inserted after
    /// `force`; then `format: String` after `check`. `evnx` is published as a
    /// library as well as a binary, so each is a breaking change for a direct
    /// caller — acceptable in 0.x, and the whole point of this pin is that it
    /// cannot happen by accident. Six `bool`s in a row is also why the argument
    /// list should become `SyncArgs`; see the note on `run`.
    #[test]
    fn test_run_signature_compiles() {
        let _func: fn(
            String, // env
            String, // example
            crate::cli::SyncDirection,
            bool,   // placeholder
            bool,   // verbose
            bool,   // dry_run
            bool,   // force
            bool,   // check
            String, // format
            Option<std::path::PathBuf>,
            crate::cli::NamingPolicy,
        ) -> anyhow::Result<()> = run;
    }
}

// ─────────────────────────────────────────────────────────────
// Machine-readable --check
// ─────────────────────────────────────────────────────────────

/// What `--check --format json` reports.
///
/// ⚠️ Read-only, and deliberately separate from the write paths. `sync` without
/// `--check` edits files; nothing pipes a file-writing command into `jq`, and
/// threading a preview back out of `sync_forward` / `sync_reverse` would mean
/// restructuring both for output nobody consumes.
///
/// The shape answers the question a CI gate asks — *what* drifted, not just that
/// something did. Until now the only answer available was the exit code.
#[derive(serde::Serialize)]
struct CheckReport {
    in_sync: bool,
    direction: &'static str,
    env: String,
    example: String,
    /// Keys the target is missing, sorted so the output is stable between runs.
    missing: Vec<String>,
    summary: CheckSummary,
}

#[derive(serde::Serialize)]
struct CheckSummary {
    missing: usize,
}

/// Compute the drift and print it as JSON. Returns whether the files are in sync.
///
/// ⚠️ A file that will not parse is an error, not "nothing missing". Reporting
/// `in_sync: true` over a template evnx could not read is the fail-open this
/// release has spent its time removing.
fn check_as_json(paths: &executor::SyncPaths, direction: SyncDirection) -> Result<bool> {
    use crate::core::Parser;

    let parser = Parser::default();
    let env = parser
        .parse_file(&paths.env)
        .with_context(|| format!("Failed to parse {}", paths.env_str()))?;

    // A missing template means everything is missing from it, which is a real
    // answer rather than an error — `sync` itself would create it.
    let example_vars = if paths.example.exists() {
        parser
            .parse_file(&paths.example)
            .with_context(|| format!("Failed to parse {}", paths.example_str()))?
            .vars
    } else {
        Default::default()
    };

    let (source, target) = match direction {
        SyncDirection::Forward => (&env.vars, &example_vars),
        SyncDirection::Reverse => (&example_vars, &env.vars),
    };

    let mut missing: Vec<String> = source
        .keys()
        .filter(|k| !target.contains_key(*k))
        .cloned()
        .collect();
    missing.sort();

    let report = CheckReport {
        in_sync: missing.is_empty(),
        direction: match direction {
            SyncDirection::Forward => "forward",
            SyncDirection::Reverse => "reverse",
        },
        env: paths.env_str(),
        example: paths.example_str(),
        summary: CheckSummary {
            missing: missing.len(),
        },
        missing,
    };

    // stdout, and nothing else on it — the property the golden tests pin.
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(!report.in_sync)
}
