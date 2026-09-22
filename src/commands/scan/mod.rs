//! Secret scanning command module.
//!
//! Scans files for accidentally committed secrets using pattern matching
//! and entropy analysis. Outputs findings with confidence levels and
//! remediation steps.
//!
//! # Architecture
//!
//! ```text
//! mod.rs          → Public API (backward compatible run() function)
//! runner.rs       → Orchestration logic
//! detector.rs     → Detection strategies (trait + implementations)
//! filters.rs      → File collection and filtering
//! output.rs       → Output formatters (pretty/json/sarif)
//! models.rs       → Data structures (Finding, ScanResults)
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use evnx::commands::scan;
//!
//! // CLI-compatible entry point
//! scan::run(
//!     vec!["./src".to_string()],  // paths
//!     vec![],                      // exclude
//!     vec![],                      // pattern (unused)
//!     false,                       // ignore_placeholders
//!     "low".to_string(),           // severity — lowest confidence reported
//!     "pretty".to_string(),        // format
//!     false,                       // exit_zero
//!     false,                       // verbose
//!     Default::default(),          // spec — empty means heuristics only
//! ).unwrap();
//! ```
//!
//! # Adding New Detectors
//!
//! See [`detector`](self::detector) module documentation for detailed steps.
//!
//! Quick example:
//!
//! ```no_run
//! # use evnx::commands::scan::detector::{SecretDetector, Detection};
//! # use evnx::commands::scan::models::Confidence;
//! # use std::path::Path;
//! pub struct MyDetector;
//!
//! impl SecretDetector for MyDetector {
//!     fn name(&self) -> &str { "my-detector" }
//!     fn scan_kv(&self, key: &str, value: &str, _loc: &str) -> Option<Detection> {
//!         // Your logic here
//!         None
//!     }
//!     fn scan_token(&self, token: &str, _loc: &str) -> Option<Detection> {
//!         self.scan_kv("", token, "")
//!     }
//! }
//! ```

pub mod detector;
pub mod filters;
pub mod models;
pub mod output;
pub mod runner;

use crate::docs;
use crate::utils::ui;
use colored::Colorize;
pub use detector::{Detection, DetectorRegistry, SecretDetector};
pub use filters::FileFilter;
pub use models::{Confidence, Finding, ScanResults};
pub use output::{render, OutputFormat};
pub use runner::{truncate_value, ScanRunner};

/// Nothing at or above `--severity`.
pub const EXIT_CLEAN: i32 = 0;
/// Secrets found.
pub const EXIT_FOUND: i32 = 1;
/// The scan could not be completed, so there is no verdict.
pub const EXIT_ERROR: i32 = 2;

/// Run the scan command (CLI entry point).
///
/// # Arguments
///
/// * `paths` - Paths to scan (files or directories)
/// * `exclude` - Exclusion patterns (glob or substring)
/// * `_pattern` - Custom patterns (currently unused, reserved for future)
/// * `ignore_placeholders` - Skip placeholder values
/// * `format` - Output format (pretty/json/sarif)
/// * `exit_zero` - Always exit 0 (for CI pipelines)
/// * `verbose` - Enable verbose output
///
/// # Exit codes
///
/// | code | meaning |
/// |------|---------|
/// | 0 | clean — nothing at or above `--severity` |
/// | 1 | secrets found (suppressed by `--exit-zero`) |
/// | 2 | the scan could not be completed |
///
/// ⚠️ **2 is not a louder 1.** "I found nothing" and "I could not look" are
/// different answers, and collapsing them is what let `evnx scan ./typo` report
/// success. A CI gate written as `evnx scan .` still fails on either, but one
/// written to branch on the code can now tell a finding from a broken setup.
///
/// `--exit-zero` suppresses 1 only. Trouble still exits 2: the flag means "do
/// not fail my build over findings", not "never tell me the scan was impossible".
///
/// # Example
///
/// ```no_run
/// use evnx::commands::scan;
///
/// scan::run(
///     vec!["./src".to_string()],
///     vec!["node_modules".to_string()],
///     vec![],
///     false,                   // ignore_placeholders
///     "low".to_string(),       // severity
///     "pretty".to_string(),    // format
///     false,                   // exit_zero
///     false,                   // verbose
///     Default::default(),      // spec — `[vars]` from .evnx.toml, if any
/// ).unwrap();
/// ```
#[allow(clippy::too_many_arguments)]
pub fn run(
    paths: Vec<String>,
    exclude: Vec<String>,
    _pattern: Vec<String>,
    ignore_placeholders: bool,
    severity: String,
    format: String,
    exit_zero: bool,
    verbose: bool,
    spec: crate::core::spec::Spec,
) -> Result<(), anyhow::Error> {
    // Trouble prints here and exits 2 rather than propagating to `main`, which
    // would format it as a generic error and collapse it onto 1 — the code that
    // already means "secrets found".
    let scan = || -> Result<bool, anyhow::Error> {
        let min_confidence: Confidence = severity.parse()?;
        let output_format: OutputFormat = format.parse()?;
        let runner = ScanRunner::with_spec(
            &exclude,
            ignore_placeholders,
            verbose,
            min_confidence,
            spec.clone(),
        );
        runner.run(paths, output_format)
    };

    let secrets_found = match scan() {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{} {:#}", "Error:".on_red().bold(), e);
            eprintln!();
            eprintln!("No verdict: evnx did not scan anything. This is not a clean result.");
            std::process::exit(EXIT_ERROR);
        }
    };

    // Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::SCAN);

    if secrets_found && !exit_zero {
        std::process::exit(EXIT_FOUND);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Verify all public exports are accessible
        let _ = Confidence::High;
        let _ = Finding::new("test", Confidence::Low, "val", "loc", None, None);
        let _ = ScanResults::new(10);
        let _ = OutputFormat::Pretty;
        let _ = ScanRunner::new(&[], false, false);
    }
}
