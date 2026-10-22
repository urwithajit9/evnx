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
//! patternset.rs   → Custom patterns: --pattern and [[scan.patterns]]
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
//! scan::run(scan::ScanOptions {
//!     paths: vec!["./src".to_string()],
//!     severity: "low".to_string(),   // lowest confidence reported
//!     ..Default::default()           // pretty output, no patterns, no spec
//! }).unwrap();
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

pub mod filters;
pub mod models;
pub mod output;
pub mod runner;
// The rule engine lives in `core::secretscan` so `sync` can ask it without
// importing a command (A-09). Its old paths keep working.
pub use crate::core::secretscan::{detector, patternset};

use crate::docs;
use crate::utils::ui;
use colored::Colorize;
pub use detector::{Detection, DetectorRegistry, SecretDetector};
pub use filters::FileFilter;
pub use models::{Confidence, Finding, ScanResults};
pub use output::{render, OutputFormat};
pub use patternset::{PatternMatch, PatternSet};
pub use runner::{truncate_value, ScanRunner};

// The codes are `core::verdict`'s, as in `sync`; these names stay for callers.
/// Nothing at or above `--severity`.
pub const EXIT_CLEAN: i32 = crate::core::verdict::EXIT_OK;
/// Secrets found.
pub const EXIT_FOUND: i32 = crate::core::verdict::EXIT_FINDINGS;
/// The scan could not be completed, so there is no verdict.
pub const EXIT_ERROR: i32 = crate::core::verdict::EXIT_ERROR;

/// Everything `evnx scan` was asked to do, resolved from flags and `.evnx.toml`.
///
/// `Default` is the bare command: the current folder, every confidence,
/// pretty output, built-ins only through `patterns` (empty here), no spec.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Files or folders to scan.
    pub paths: Vec<String>,
    /// Exclusion patterns for folder walks (`--exclude` plus `[scan] exclude`).
    pub exclude: Vec<String>,
    /// Secret formats in force: `--pattern` merged with `[[scan.patterns]]`
    /// and the built-ins. See [`patternset::merge`](self::patternset::merge).
    pub patterns: Vec<crate::core::config::PatternRule>,
    /// Skip values that look like filler.
    pub ignore_placeholders: bool,
    /// Lowest confidence reported: `high`, `medium` or `low`.
    pub severity: String,
    /// `pretty`, `json`, `sarif` or `github`.
    pub format: String,
    /// Exit 0 even with findings (never hides exit 2).
    pub exit_zero: bool,
    /// Progress lines on stderr.
    pub verbose: bool,
    /// `[vars]` from `.evnx.toml`, if any.
    pub spec: crate::core::spec::Spec,
    /// An extension list for folder walks and where it came from; `None`
    /// reads every file (I-047).
    pub extensions: Option<(Vec<String>, &'static str)>,
    /// Report findings in files git ignores and does not track (I-285).
    pub include_ignored: bool,
    /// Skip walked files larger than this many bytes; `None` is 1 MiB (I-061).
    pub max_file_size: Option<u64>,
    /// `[scan] disable`, as written: built-in rule names and
    /// [`detector::HEURISTIC_NAMES`] (I-056). The rules themselves are already
    /// out of `patterns`; this also silences the heuristics for their values.
    pub disable: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            paths: vec![".".to_string()],
            exclude: Vec::new(),
            patterns: Vec::new(),
            ignore_placeholders: false,
            severity: "low".to_string(),
            format: "pretty".to_string(),
            exit_zero: false,
            verbose: false,
            spec: Default::default(),
            extensions: None,
            include_ignored: false,
            max_file_size: None,
            disable: Vec::new(),
        }
    }
}

/// Run the scan command (CLI entry point).
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
/// ⚠️ **A custom pattern that does not compile is trouble, not a clean scan.**
/// `--pattern '('` exits 2 and says so. A scanner that shrugged off a broken
/// rule and printed "no secrets found" would be reporting a result for a search
/// it never ran — and in CI that reads as a pass.
///
/// # Example
///
/// ```no_run
/// use evnx::commands::scan;
///
/// scan::run(scan::ScanOptions {
///     paths: vec!["./src".to_string()],
///     exclude: vec!["node_modules".to_string()],
///     ..Default::default()
/// }).unwrap();
/// ```
pub fn run(opts: ScanOptions) -> Result<(), anyhow::Error> {
    // Trouble prints here and exits 2 rather than propagating to `main`, which
    // would format it as a generic error and collapse it onto 1 — the code that
    // already means "secrets found".
    let scan = || -> Result<bool, anyhow::Error> {
        let min_confidence: Confidence = opts.severity.parse()?;
        let output_format: OutputFormat = opts.format.parse()?;
        // Compiled before anything is printed, so a broken expression cannot be
        // mistaken for a finished scan.
        let compiled = PatternSet::compile(&opts.patterns)?;
        let off: Vec<String> = opts.disable.iter().map(|n| n.to_lowercase()).collect();
        let disabled = PatternSet::compile(
            &crate::utils::patterns::builtin_rules()
                .into_iter()
                .filter(|r| off.contains(&r.name.to_lowercase()))
                .collect::<Vec<_>>(),
        )?;
        let runner = ScanRunner::with_spec_and_patterns(
            &opts.exclude,
            opts.ignore_placeholders,
            opts.verbose,
            min_confidence,
            opts.spec.clone(),
            compiled,
        )
        .with_extensions(opts.extensions.clone())
        .with_include_ignored(opts.include_ignored)
        .with_max_file_size(opts.max_file_size)
        .with_disabled(disabled, &opts.disable);
        runner.run(opts.paths.clone(), output_format)
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

    if secrets_found && !opts.exit_zero {
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
