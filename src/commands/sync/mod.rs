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

/// Public entry point called from main.rs
/// Signature unchanged to maintain compatibility with existing CLI wiring
pub fn run(
    direction: SyncDirection,
    placeholder: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    template_config: Option<PathBuf>,
    naming_policy: NamingPolicy,
) -> Result<()> {
    let ctx = executor::SyncCtx {
        direction,
        placeholder,
        verbose,
        dry_run,
        force,
        template_config,
        naming_policy,
    };
    executor::execute(ctx)
}

// ─────────────────────────────────────────────────────────────
// Tests (Only if you want to test public API here)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // SyncDirection is defined in cli.rs, so test it there instead
    // But if you want to verify the run() function signature compiles:

    #[test]
    fn test_run_signature_compiles() {
        // This test just ensures the public API hasn't changed
        // We can't actually call run() without setting up files,
        // but we can verify the function exists with expected params
        let _func: fn(
            crate::cli::SyncDirection,
            bool,
            bool,
            bool,
            bool,
            Option<std::path::PathBuf>,
            crate::cli::NamingPolicy,
        ) -> anyhow::Result<()> = run;
    }
}
