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
//!     "pretty".to_string(),        // format
//!     false,                       // exit_zero
//!     false,                       // verbose
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

pub use detector::{Detection, DetectorRegistry, SecretDetector};
pub use filters::FileFilter;
pub use models::{Confidence, Finding, ScanResults};
pub use output::{render, OutputFormat};
pub use runner::{truncate_value, ScanRunner};

// In commands/scan/mod.rs

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
/// # Returns
///
/// Ok(()) on success. May exit with code 1 if secrets found (unless exit_zero=true).
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
///     false,
///     "pretty".to_string(),
///     false,
///     false,
/// ).unwrap();
/// ```
pub fn run(
    paths: Vec<String>,
    exclude: Vec<String>,
    _pattern: Vec<String>, // Reserved for future custom pattern support
    ignore_placeholders: bool,
    format: String,
    exit_zero: bool,
    verbose: bool,
) -> Result<(), anyhow::Error> {
    // let output_format = OutputFormat::from_str(&format);
    let output_format = format.parse().unwrap_or(OutputFormat::Pretty);
    let runner = ScanRunner::new(&exclude, ignore_placeholders, verbose);

    // Run scan and check if secrets were found
    let secrets_found = runner.run(paths, output_format)?;

    // Handle exit code based on configuration
    if secrets_found && !exit_zero {
        std::process::exit(1);
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
