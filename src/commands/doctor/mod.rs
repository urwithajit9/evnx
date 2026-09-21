//! 🩺 Environment Doctor — diagnose and fix setup issues
//!
//! # Usage
//! ```no_run
//! use anyhow::Result;
//! use evnx::commands::doctor::run;
//!
//! fn main() -> Result<()> {
//!     // path, verbose, fix, strict
//!     run("./my-app".into(), true, false, false)?;
//!     Ok(())
//! }
//! ```
//!
//! # Exit codes
//! - `0` healthy · `1` problems found · `2` doctor could not run
//!
//! # Environment Variables
//! - `EVNX_OUTPUT_JSON=1` — Output results as JSON (for CI/CD)
//! - `EVNX_AUTO_FIX=1` — Same as `--fix`, kept because it shipped first

// Re-export the main entry point so callers use: doctor::run(...)
pub use runner::run;
pub use runner::{EXIT_ERROR, EXIT_HEALTHY, EXIT_PROBLEMS};
// use crate::{docs, utils::ui};

// Internal modules — not public API yet
mod runner;
mod types; // This is your existing doctor.rs content (renamed)

// Re-export types if needed externally (optional)
pub use types::{CheckResult, DiagnosticReport, Severity, Summary};
