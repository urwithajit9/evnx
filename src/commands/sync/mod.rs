//! Sync command: keep `.env` and `.env.example` in sync.
//!
//! This module provides the public API for the sync command.
//! Internal implementation is organized into submodules for maintainability.

use anyhow::Result;
use std::path::PathBuf;

use crate::cli::{NamingPolicy, SyncDirection};

mod executor;
mod models;
mod placeholder;
mod security;

// Re-export public types for external use (if needed)
pub use models::{PlaceholderConfig, SyncAction, SyncPreview, VarChange};

/// Public entry point called from main.rs.
///
/// # `--check`
///
/// `check` implies `dry_run` and turns "the files are out of step" into a
/// **non-zero exit**, the way `prettier --check`, `cargo fmt --check` and
/// `git diff --exit-code` do. Without it `sync --dry-run` previews and exits 0
/// whatever it finds, which is correct for a human looking at the output and
/// useless as a CI gate — `evnx.dev`'s pipeline recipe is written as
/// `if ! evnx sync --direction forward --dry-run --force; then …`, a condition
/// that could never fire.
///
/// `--dry-run`'s own exit code is deliberately left alone: it is a preview, and
/// scripts that already call it should not start failing.
// 8 flags. `SyncArgs` in `cli.rs` already holds exactly this set, so the tidier
// signature is to pass that struct — but it would rewrite every call site for no
// behavioural gain, so it is queued for the v0.5.0 command review instead.
// `validate::run` carries the same allow for the same reason.
#[allow(clippy::too_many_arguments)]
pub fn run(
    direction: SyncDirection,
    placeholder: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    check: bool,
    template_config: Option<PathBuf>,
    naming_policy: NamingPolicy,
) -> Result<()> {
    let ctx = executor::SyncCtx {
        direction,
        placeholder,
        verbose,
        // `--check` is a dry run that reports its verdict through the exit code.
        dry_run: dry_run || check,
        force,
        template_config,
        naming_policy,
    };

    let out_of_step = executor::execute(ctx)?;

    if check && out_of_step {
        eprintln!();
        eprintln!("✗ Out of sync. Run `evnx sync --direction {direction}` and commit the result.");
        std::process::exit(1);
    }

    Ok(())
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
    /// ⚠️ Updated 2026-09-21: `check: bool` was inserted after `force`. `evnx` is
    /// published as a library as well as a binary, so this is a breaking change
    /// for any direct caller — acceptable in 0.x, and the whole point of this
    /// pin is that it could not happen by accident. Six `bool`s in a row is also
    /// why the argument list should become `SyncArgs`; see the note on `run`.
    #[test]
    fn test_run_signature_compiles() {
        let _func: fn(
            crate::cli::SyncDirection,
            bool, // placeholder
            bool, // verbose
            bool, // dry_run
            bool, // force
            bool, // check
            Option<std::path::PathBuf>,
            crate::cli::NamingPolicy,
        ) -> anyhow::Result<()> = run;
    }
}
