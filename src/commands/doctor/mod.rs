//! 🩺 Environment Doctor — diagnose and fix setup issues
//!
//! # Usage
//! ```no_run
//! use anyhow::Result;
//! use evnx::commands::doctor::run;
//!
//! fn main() -> Result<()> {
//!     run("./my-app".into(), true)?;
//!     Ok(())
//! }
//! ```
//!
//! # Environment Variables
//! - `EVNX_OUTPUT_JSON=1` — Output results as JSON (for CI/CD)
//! - `EVNX_AUTO_FIX=1` — Attempt to auto-fix detected issues

// Re-export the main entry point so callers use: doctor::run(...)
pub use runner::run;
// use crate::{docs, utils::ui};

// Internal modules — not public API yet
mod runner;
mod types; // This is your existing doctor.rs content (renamed)

// Re-export types if needed externally (optional)
pub use types::{CheckResult, DiagnosticReport, Severity, Summary};
