//! 🩺 Environment Doctor — diagnose and fix setup issues
//!
//! # Usage
//! ```no_run
//! use anyhow::Result;
//! use evnx::commands::doctor::run;
//!
//! fn main() -> Result<()> {
//!     // path, verbose, fix, strict, format
//!     run("./my-app".into(), true, false, false, None)?;
//!     // ...or ask for machine-readable output:
//!     run("./my-app".into(), false, false, true, Some("json".into()))?;
//!     Ok(())
//! }
//! ```
//!
//! # Exit codes
//! - `0` healthy · `1` problems found · `2` doctor could not run
//!
//! # Environment Variables
//! - `EVNX_OUTPUT_JSON=1` — Same as `--format json`, kept because it shipped first
//! - `EVNX_AUTO_FIX=1` — Same as `--fix`, kept because it shipped first
//!
//! Both predate the flags that replaced them. An environment variable is not a
//! discoverable surface, which is how `--fix` came to be documented into
//! existence years before it existed; the flag wins when both are given.

// Re-export the main entry point so callers use: doctor::run(...)
pub use runner::run;
pub use runner::{EXIT_ERROR, EXIT_HEALTHY, EXIT_PROBLEMS};
// use crate::{docs, utils::ui};

// Internal modules — not public API yet
mod runner;
mod types; // This is your existing doctor.rs content (renamed)

// Re-export types if needed externally (optional)
pub use types::{CheckResult, DiagnosticReport, Severity, Summary};
