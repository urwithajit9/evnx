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
    models::{Confidence, Finding, ScanResults},
    output::{render, OutputFormat},
};
use crate::utils::ui;
use anyhow::Result;
// use colored::*;
use std::path::Path;

/// Characters shown at each end of a masked secret.
const VISIBLE_EDGE: usize = 4;

/// Below this length no characters are shown at all — four of eight would be
/// half the secret.
///
/// ⚠️ There used to be a `MAX_VISIBLE = 20` here, under which a value was shown
/// whole. An AWS access key ID is exactly 20 characters, so the threshold
/// perfectly exempted the format the scanner detects most often. Length is not a
/// safety property; a short secret is not a less secret one.
const MIN_MASKABLE: usize = 8;

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
    /// Lowest confidence worth reporting. `Low` reports everything.
    min_confidence: Confidence,
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
        Self::with_min_confidence(exclude, ignore_placeholders, verbose, Confidence::Low)
    }

    /// As [`ScanRunner::new`], but reporting only findings at or above
    /// `min_confidence` — what `--severity` sets.
    ///
    /// Filtering happens at the point a finding is recorded rather than at render
    /// time, so every count, the JSON `summary`, and the process exit code all
    /// describe the same filtered set. A scanner whose exit code disagreed with
    /// its own output would be worse than one without the flag.
    pub fn with_min_confidence(
        exclude: &[String],
        ignore_placeholders: bool,
        verbose: bool,
        min_confidence: Confidence,
    ) -> Self {
        Self::with_spec(
            exclude,
            ignore_placeholders,
            verbose,
            min_confidence,
            Default::default(),
        )
    }

    /// As [`ScanRunner::with_min_confidence`], plus the project's declared
    /// contract — `secret = true` reports a credential the heuristics cannot
    /// name, and `secret = false` retracts a guess the project has looked at.
    pub fn with_spec(
        exclude: &[String],
        ignore_placeholders: bool,
        verbose: bool,
        min_confidence: Confidence,
        spec: crate::core::spec::Spec,
    ) -> Self {
        Self {
            registry: DetectorRegistry::new().with_spec(spec),
            filter: FileFilter::new(exclude),
            ignore_placeholders,
            verbose,
            min_confidence,
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
                    if let Some(detection) =
                        Self::best(self.registry.scan_kv(key, value, &location))
                    {
                        self.add_finding(results, path, line_num, Some(key.to_string()), detection);
                    }
                }
            } else {
                // Scan tokens for general text files
                let location = format!("{}:{}", path.display(), line_num);
                for token in Self::extract_tokens(line) {
                    if let Some(detection) = Self::best(self.registry.scan_token(&token, &location))
                    {
                        self.add_finding(results, path, line_num, None, detection);
                    }
                }
            }
        }

        Ok(())
    }

    /// One value, one finding.
    ///
    /// Every registered detector sees each key/value pair, and more than one can
    /// match — `STRIPE_SECRET_KEY` fires both the pattern matcher and the
    /// sensitive-key heuristic. Recording both inflated `secrets_found` (one
    /// Stripe key reported as two secrets), printed the same line twice, and
    /// emitted two GitHub annotations for it.
    ///
    /// Ranking, in order:
    ///
    /// 1. **Carries a remediation URL.** A finding that can say *where to revoke
    ///    this* is more useful than one that cannot, and only the named provider
    ///    patterns can. Confidence alone gets this wrong: the sensitive-key
    ///    heuristic scores `High` on any long value, so it beat the real
    ///    `AWS Secret Access Key` match — `Medium` on purpose, since that pattern
    ///    is just "40 base64-ish characters" — and a genuine AWS key was reported
    ///    as "Sensitive config key" with no link to IAM.
    /// 2. **Higher confidence.** So `MY_PASSWORD=<long value>`, which matches only
    ///    the key-aware heuristic and the shapeless high-entropy fallback, keeps
    ///    the key-aware answer.
    /// 3. **Earliest detector**, which is `PatternDetector`.
    ///
    /// The winner keeps its label and URL but takes the **highest confidence any
    /// detector reported**. The two are answering different questions: the
    /// pattern's `Medium` on an AWS secret key expresses doubt about *which
    /// provider* a shapeless 40-character blob belongs to, not doubt about
    /// whether it is a secret — and `AWS_SECRET_ACCESS_KEY` as a key name settles
    /// the second question. Two independent detectors agreeing is stronger
    /// evidence than either alone.
    ///
    /// Without this, deduplication would have quietly narrowed `--severity high`:
    /// a real AWS secret key would drop to `Medium` and slip under a gate that
    /// used to catch it through the heuristic's separate `High` finding.
    fn best(detections: Vec<super::detector::Detection>) -> Option<super::detector::Detection> {
        fn rank(d: &super::detector::Detection) -> (bool, super::models::Confidence) {
            (d.action_url.is_some(), d.confidence)
        }

        let strongest = detections.iter().map(|d| d.confidence).max()?;
        let mut winner = detections.into_iter().reduce(|best, next| {
            if rank(&next) > rank(&best) {
                next
            } else {
                best
            }
        })?;
        winner.confidence = strongest;
        Some(winner)
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

        // Below the --severity threshold: not recorded at all.
        if detection.confidence < self.min_confidence {
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
/// ⚠️ **Masks by policy, never by length.** This used to return the value
/// unchanged when it was `<= MAX_VISIBLE` (20) characters — and an AWS access key
/// ID is `AKIA` plus sixteen characters, **exactly 20**. So the single
/// most-detected credential format was printed in full, into terminal scrollback
/// and, through `scan --format json`, into CI artefacts.
///
/// # What is shown, and why so little
///
/// At most four leading and four trailing characters, and never more than half
/// the value. The preview exists so you can *recognise* which secret was found —
/// `AKIA…LKEY` is plainly an AWS key — not so you can verify it. The finding
/// already prints the variable name and the file and line, which is what actually
/// identifies it; the value adds recognition, not identity.
///
/// A value under [`MIN_MASKABLE`] characters shows no characters at all. Four of
/// eight is half a secret; there is no prefix short enough to be safe on a short
/// value, so it reports the length instead.
///
/// # Example
///
/// ```
/// # use evnx::commands::scan::runner::truncate_value;
/// assert_eq!(truncate_value("AKIA4OZRMFJ3VREALKEY"), "AKIA…LKEY");
/// assert_eq!(truncate_value("tiny"), "<4 chars>");
/// ```
pub fn truncate_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();

    if n < MIN_MASKABLE {
        // Nothing can be revealed safely, so reveal nothing.
        return format!("<{n} chars>");
    }

    // Never more than a quarter from each end, so at most half the value.
    let keep = VISIBLE_EDGE.min(n / 4);
    if keep == 0 {
        return format!("<{n} chars>");
    }

    let prefix: String = chars[..keep].iter().collect();
    let suffix: String = chars[n - keep..].iter().collect();
    format!("{prefix}…{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ These two tests asserted the bug. `truncate_value("short") == "short"`
    /// and `truncate_value("exactly20characters!")` returning its input whole
    /// were written as the specification, so the suite passed for as long as the
    /// scanner printed AWS keys in full.
    #[test]
    fn no_detected_value_is_ever_shown_whole() {
        for secret in [
            "AKIA4OZRMFJ3VREALKEY",                          // 20 — the bug
            "exactly20characters!",                          // 20 — the old test's own case
            "short",                                         // 5
            "sk_live_51Habc123def456",                       // 24
            "this_is_a_very_long_secret_key_value_12345678", // 45
        ] {
            let masked = truncate_value(secret);
            assert!(
                !masked.contains(secret),
                "{secret} was shown whole as {masked}"
            );
        }
    }

    /// Enough to recognise the provider, never enough to use.
    #[test]
    fn a_mask_keeps_the_shape_and_drops_the_secret() {
        assert_eq!(truncate_value("AKIA4OZRMFJ3VREALKEY"), "AKIA…LKEY");
        assert_eq!(
            truncate_value("this_is_a_very_long_secret_key_value_12345678"),
            "this…5678"
        );
    }

    /// Below `MIN_MASKABLE` there is no prefix short enough to be safe, so the
    /// length is reported instead of any characters.
    #[test]
    fn a_short_value_reveals_nothing_at_all() {
        assert_eq!(truncate_value("short"), "<5 chars>");
        assert_eq!(truncate_value("tiny"), "<4 chars>");
        assert_eq!(truncate_value(""), "<0 chars>");
    }

    /// Never more than half the value, whatever its length.
    #[test]
    fn at_most_half_the_characters_survive() {
        for n in 8..64 {
            let secret: String = std::iter::repeat_n('x', n).collect();
            let shown = truncate_value(&secret)
                .chars()
                .filter(|c| *c == 'x')
                .count();
            assert!(shown * 2 <= n, "{n}-char value revealed {shown} characters");
        }
    }

    /// Multi-byte input must not panic on a character boundary.
    #[test]
    fn masking_is_utf8_safe() {
        let masked = truncate_value("🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑");
        assert!(masked.contains('…'), "{masked}");
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
