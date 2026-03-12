//! Scan orchestration and execution logic.
//!
//! This module coordinates the scanning process:
//! 1. Collect files using [`FileFilter`](super::filters::FileFilter)
//! 2. Scan each file using [`DetectorRegistry`](super::detector::DetectorRegistry)
//! 3. Aggregate results into [`ScanResults`](super::models::ScanResults)
//! 4. Output results using [`render()`](super::output::render)
//!
//! # Example
//!
//! ```
//! use evnx::commands::scan::ScanRunner;
//! use evnx::commands::scan::OutputFormat;
//!
//! let runner = ScanRunner::new(&[], false, false);
//! runner.run(vec!["./src".to_string()], OutputFormat::Pretty).unwrap();
//! ```

use super::{
    detector::DetectorRegistry,
    filters::FileFilter,
    models::{Finding, ScanResults},
    output::{render, OutputFormat},
};
use crate::utils::ui;
use anyhow::Result;
// use colored::*;
use std::path::Path;

/// Maximum visible characters in secret preview
const PREFIX_LEN: usize = 8;
/// Suffix length for secret preview
const SUFFIX_LEN: usize = 5;
/// Maximum total visible characters
const MAX_VISIBLE: usize = 20;

/// Main orchestrator for secret scanning operations.
///
/// Holds configuration and coordinates all scanning components.
///
/// # Fields
///
/// * `registry` - Detector registry with all active detection strategies
/// * `filter` - File filter for collecting scannable files
/// * `ignore_placeholders` - Whether to skip placeholder values
/// * `verbose` - Whether to print verbose progress information
///
/// # Example
///
/// ```
/// use evnx::commands::scan::ScanRunner;
/// let runner = ScanRunner::new(
///     &["node_modules".to_string()],
///     true,  // ignore placeholders
///     false, // not verbose
/// );
/// ```
pub struct ScanRunner {
    registry: DetectorRegistry,
    filter: FileFilter,
    ignore_placeholders: bool,
    verbose: bool,
}

impl ScanRunner {
    /// Create a new ScanRunner with configuration.
    ///
    /// # Arguments
    ///
    /// * `exclude` - File exclusion patterns
    /// * `ignore_placeholders` - Skip placeholder values (e.g., "changeme", "example")
    /// * `verbose` - Enable verbose progress output
    ///
    /// # Example
    ///
    /// ```
    /// # use evnx::commands::scan::ScanRunner;
    /// let runner = ScanRunner::new(&["*.log".to_string()], false, true);
    /// ```
    pub fn new(exclude: &[String], ignore_placeholders: bool, verbose: bool) -> Self {
        Self {
            registry: DetectorRegistry::new(),
            filter: FileFilter::new(exclude),
            ignore_placeholders,
            verbose,
        }
    }

    // In commands/scan/runner.rs

    /// Run the scan on the given paths.
    ///
    /// # Arguments
    ///
    /// * `paths` - Vector of file or directory paths to scan
    /// * `format` - Output format for results
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Scan completed, secrets were found
    /// * `Ok(false)` - Scan completed, no secrets found
    /// * `Err(e)` - Scan failed with error
    ///
    /// # Note
    ///
    /// This method does NOT call `std::process::exit()`. The caller
    /// is responsible for exit code handling based on configuration.
    ///
    /// # Example
    ///
    /// ```ignore
    /// # use evnx::commands::scan::ScanRunner;
    /// # use evnx::commands::scan::OutputFormat;
    /// let runner = ScanRunner::new(&[], false, false);
    /// let found_secrets = runner.run(vec!["./src".to_string()], OutputFormat::Pretty)?;
    /// if found_secrets {
    ///     println!("⚠️  Secrets detected!");
    /// }
    /// ```
    pub fn run(&self, paths: Vec<String>, format: OutputFormat) -> Result<bool> {
        // Only show header for human-readable output
        // Machine formats (JSON/SARIF) skip the UI header to keep stdout clean
        if matches!(format, OutputFormat::Pretty) {
            ui::print_header_stderr("evnx scan", Some("Checking for exposed secrets"));
        } else if self.verbose {
            // For machine formats, at least acknowledge start in verbose mode
            ui::verbose_stderr("Starting scan...");
        }

        if self.verbose {
            ui::verbose_stderr(format!("Scanning {} files...", paths.len()));
        }

        let files = self.filter.collect_files(&paths)?;

        if self.verbose {
            ui::verbose_stderr(format!("Found {} files to scan", files.len()));
        }

        let mut results = ScanResults::new(files.len());

        for file in &files {
            if self.verbose {
                ui::scanning_file_stderr(file);
            }
            self.scan_file(file, &mut results)?;
        }

        // Render output (goes to stdout)
        render(&results, format, &files)?;

        // Return status for caller to handle exit code
        Ok(results.secrets_found > 0)
    }

    /// Scan a single file for secrets.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to scan
    /// * `results` - Mutable results to append findings to
    ///
    /// # Returns
    ///
    /// Ok(()) on success. Skips files that can't be read as text.
    fn scan_file(&self, path: &Path, results: &mut ScanResults) -> Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(()), // Skip binary/unreadable files
        };

        let is_env = path.to_string_lossy().contains(".env");

        for (line_num, line) in content.lines().enumerate() {
            let line_num = line_num + 1;

            // Skip comments and empty lines
            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }

            if is_env {
                // Parse as key=value for .env files
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().trim_start_matches("export").trim();
                    let value = value.trim();

                    let location = format!("{}:{} ({})", path.display(), line_num, key);
                    for detection in self.registry.scan_kv(key, value, &location) {
                        self.add_finding(results, path, line_num, Some(key.to_string()), detection);
                    }
                }
            } else {
                // Scan tokens for general text files
                let location = format!("{}:{}", path.display(), line_num);
                for token in Self::extract_tokens(line) {
                    for detection in self.registry.scan_token(&token, &location) {
                        self.add_finding(results, path, line_num, None, detection);
                    }
                }
            }
        }

        Ok(())
    }

    /// Add a finding to results with proper truncation and filtering.
    ///
    /// # Arguments
    ///
    /// * `results` - Mutable results to append to
    /// * `path` - File path for location context
    /// * `line` - Line number
    /// * `variable` - Optional variable name
    /// * `detection` - Detection from a detector
    fn add_finding(
        &self,
        results: &mut ScanResults,
        path: &Path,
        line: usize,
        variable: Option<String>,
        detection: super::detector::Detection,
    ) {
        // Skip placeholders if configured
        if self.ignore_placeholders
            && crate::utils::patterns::is_placeholder(&detection.matched_value)
        {
            return;
        }

        let finding = Finding::new(
            detection.pattern,
            detection.confidence,
            truncate_value(&detection.matched_value),
            if let Some(ref var) = variable {
                format!("{}:{} ({})", path.display(), line, var)
            } else {
                format!("{}:{}", path.display(), line)
            },
            variable,
            detection.action_url,
        );

        results.add_finding(finding);
    }

    /// Extract tokens from a line for scanning.
    ///
    /// Splits on common separators and filters by minimum length.
    ///
    /// # Arguments
    ///
    /// * `line` - Line of text to extract tokens from
    ///
    /// # Returns
    ///
    /// Vector of token strings (min 20 chars by default)
    fn extract_tokens(line: &str) -> Vec<String> {
        line.split(|c: char| c.is_whitespace() || c == '=' || c == ':' || c == '"' || c == '\'')
            .filter(|t| t.len() > 20)
            .map(|s| s.to_string())
            .collect()
    }

    // /// Print the scan header UI.
    // fn print_header(&self) {
    //     println!(
    //         "\n{}",
    //         "┌─ Scanning for exposed secrets ──────────────────────┐".cyan()
    //     );
    //     println!(
    //         "{}",
    //         "│ Checking for real-looking credentials               │".cyan()
    //     );
    //     println!(
    //         "{}\n",
    //         "└──────────────────────────────────────────────────────┘".cyan()
    //     );
    // }
}

/// Truncate a value for safe display.
///
/// Shows first 8 and last 5 characters with ellipsis in between.
/// Never displays the full secret value.
///
/// # Arguments
///
/// * `value` - The secret value to truncate
///
/// # Returns
///
/// Truncated string safe for display.
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::scan::runner::truncate_value;
/// let truncated = truncate_value("AKIA1234567890EXAMPLE");
/// assert_eq!(truncated, "AKIA1234...MPLE");
/// ```
pub fn truncate_value(value: &str) -> String {
    if value.len() <= MAX_VISIBLE {
        value.to_string()
    } else {
        format!(
            "{}...{}",
            &value[..PREFIX_LEN],
            &value[value.len() - SUFFIX_LEN..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_value_short() {
        assert_eq!(truncate_value("short"), "short");
        assert_eq!(
            truncate_value("exactly20characters!"),
            "exactly20characters!"
        );
    }

    #[test]
    fn test_truncate_value_long() {
        let result = truncate_value("this_is_a_very_long_secret_key_value_12345678");
        assert_eq!(result, "this_is_...45678");
        assert!(result.len() <= MAX_VISIBLE + 3); // +3 for "..."
    }

    #[test]
    fn test_extract_tokens() {
        let line = "API_KEY=sk_live_1234567890abcdefghijklmnop short";
        let tokens = ScanRunner::extract_tokens(line);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].len() > 20);
    }

    #[test]
    fn test_scan_runner_new() {
        let runner = ScanRunner::new(&["test".to_string()], true, true);
        assert!(runner.ignore_placeholders);
        assert!(runner.verbose);
    }
    #[test]
    fn test_scan_runner_returns_secrets_found() {
        // This test would need a fixture with a known secret
        // For now, just verify the method signature works
        let _runner = ScanRunner::new(&[], false, false);
        // Note: Full integration test requires test fixtures
    }
}
