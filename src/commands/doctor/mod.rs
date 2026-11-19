//! 🩺 Environment Doctor — diagnose and fix setup issues
//!
//! # Usage
//! ```no_run
//! use anyhow::Result;
//! use evnx::commands::doctor::run;
//!
//! fn main() -> Result<()> {
//!     // path, verbose, fix, strict, json (None: EVNX_OUTPUT_JSON decides), skip_without_env
//!     run("./my-app".into(), true, false, false, None, false)?;
//!     // ...or ask for machine-readable output:
//!     run("./my-app".into(), false, false, true, Some(true), false)?;
//!     Ok(())
//! }
//! ```
//!
//! # Exit codes
//! - `0` healthy · `1` problems found · `2` doctor could not run, or a check
//!   could not finish
//!
//! # Environment Variables
//! - `EVNX_OUTPUT_JSON=1` — Same as `--format json`, kept because it shipped first
//!
//! `EVNX_AUTO_FIX` is no longer read: it prints one warning and repairs
//! nothing. Pass `--fix`. An environment variable is not a discoverable
//! surface, which is how `--fix` came to be documented into existence years
//! before it existed.

// Re-export the main entry point so callers use: doctor::run(...)
pub use runner::{run, run_all};
// use crate::{docs, utils::ui};

// Internal modules — not public API yet
mod all;
mod runner;
mod types; // This is your existing doctor.rs content (renamed)

// Re-export types if needed externally (optional)
pub use types::{CheckResult, DiagnosticReport, Severity, Summary};
