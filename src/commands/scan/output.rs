//! Output formatters for scan results.
//!
//! This module provides multiple output formats:
//! - **Pretty**: Human-readable terminal output with colors and icons
//! - **JSON**: Machine-readable JSON for tooling integration
//! - **SARIF**: Standard format for GitHub Code Scanning and static analysis
//!
//! # Example
//!
//! ```no_run
//! # use evnx::commands::scan::OutputFormat;
//! # use evnx::commands::scan::ScanResults;
//! # use evnx::commands::scan::render;
//! let results = ScanResults::new(10);
//! render(&results, OutputFormat::Pretty, &[]).unwrap();
//! ```

use super::models::{Confidence, ScanResults};
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;

/// Column the confidence level is right-aligned against.
const WIDTH: usize = 44;
use anyhow::Result;
use colored::*;
use std::path::PathBuf;
use std::str::FromStr;

/// Output format for scan results.
///
/// Determines how results are presented to the user or downstream tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable terminal output with colors
    Pretty,
    /// JSON format for machine parsing
    Json,
    /// SARIF format for GitHub Code Scanning
    Sarif,
    /// GitHub Actions workflow commands — inline annotations on the PR diff
    Github,
}

// impl OutputFormat {
//     /// Parse output format from string argument.
//     ///
//     /// # Arguments
//     ///
//     /// * `s` - Format string (case-insensitive)
//     ///
//     /// # Returns
//     ///
//     /// Matching OutputFormat variant, defaults to Pretty for unknown values.
//     ///
//     /// # Example
//     ///
//     /// ```
//     /// # use evnx::commands::scan::OutputFormat;
//     /// assert_eq!(OutputFormat::from_str("json"), OutputFormat::Json);
//     /// assert_eq!(OutputFormat::from_str("SARIF"), OutputFormat::Sarif);
//     /// assert_eq!(OutputFormat::from_str("unknown"), OutputFormat::Pretty);
//     /// ```
//     pub fn from_str(s: &str) -> Self {
//         match s.to_lowercase().as_str() {
//             "json" => Self::Json,
//             "sarif" => Self::Sarif,
//             _ => Self::Pretty,
//         }
//     }
// }
impl OutputFormat {
    /// Accepted spellings, for error messages and `--help`.
    pub const NAMES: &'static [&'static str] = &["pretty", "json", "sarif", "github"];
}

impl FromStr for OutputFormat {
    type Err = anyhow::Error;

    /// ⚠️ An unrecognised format is an **error**.
    ///
    /// This used to be `_ => Ok(Self::Pretty)`. A typo, or a format that was only
    /// ever documented, therefore printed human-readable output and exited 0 —
    /// `evnx scan --format github` did exactly that for as long as the flag was
    /// documented and unimplemented, so a CI step that looked configured was
    /// checking nothing. Failing loudly is the only safe default for a secret
    /// scanner's output contract.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "pretty" | "text" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            "sarif" => Ok(Self::Sarif),
            // `evnx validate` spells this `github-actions`; accept both
            // everywhere rather than making people remember which is which.
            "github" | "github-actions" => Ok(Self::Github),
            other => Err(anyhow::anyhow!(
                "unknown format '{other}' — expected one of: {}",
                Self::NAMES.join(", ")
            )),
        }
    }
}

/// Render scan results in the specified format.
///
/// # Arguments
///
/// * `results` - Scan results to render
/// * `format` - Output format to use
/// * `files` - List of scanned files (used in pretty output)
///
/// # Returns
///
/// Ok(()) on success, Err if output fails (e.g., JSON serialization error).
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::scan::OutputFormat;
/// # use evnx::commands::scan::ScanResults;
/// # use evnx::commands::scan::output::render;
/// let results = ScanResults::new(10);
/// render(&results, OutputFormat::Json, &[]).unwrap();
/// ```
pub fn render(results: &ScanResults, format: OutputFormat, files: &[PathBuf]) -> Result<()> {
    match format {
        OutputFormat::Pretty => render_pretty(results, files),
        OutputFormat::Json => render_json(results),
        OutputFormat::Sarif => render_sarif(results),
        OutputFormat::Github => render_github(results),
    }
}

/// Render results in human-readable pretty format.
///
/// Includes colored output, icons, and remediation guidance.
///
/// # Arguments
///
/// * `results` - Scan results to render
/// * `files` - List of scanned files for summary
fn render_pretty(results: &ScanResults, files: &[PathBuf]) -> Result<()> {
    if !files.is_empty() {
        let shown = files
            .iter()
            .take(3)
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let rest = files.len().saturating_sub(3);
        let suffix = if rest > 0 {
            format!(" and {}", pluralize(rest, "other file", "other files"))
        } else {
            String::new()
        };
        println!("  scanning {}{}\n", shown.dimmed(), suffix.dimmed());
    }

    if results.secrets_found == 0 {
        println!("  {}  No secrets detected", glyph::OK.green());
        println!(
            "\n  {}",
            pluralize(results.files_scanned, "file scanned", "files scanned").dimmed()
        );
        return Ok(());
    }

    // ⚠️ Every finding starts with its glyph at the same column, and every field
    // under it is indented past that glyph. The remediation note used to be
    // printed by `ui::warning`, which writes at column 0 — so a line that belongs
    // to one finding appeared to belong to none.
    for finding in &results.findings {
        let (mark, level) = match finding.confidence {
            Confidence::High => (glyph::FAIL.red(), "high".red()),
            Confidence::Medium => (glyph::WARN.yellow(), "medium".yellow()),
            Confidence::Low => (glyph::INFO.dimmed(), "low".dimmed()),
        };

        // Confidence right-aligned against a fixed column, so the levels form a
        // column of their own however long the pattern name is.
        let name = finding.pattern.bold().to_string();
        // `.max(2)` so a pattern name longer than the column still gets a gap
        // before its level, rather than running straight into it.
        let pad = WIDTH.saturating_sub(finding.pattern.chars().count()).max(2);
        println!("  {mark}  {name}{:pad$}{level}", "", pad = pad);

        println!(
            "     {}  {}",
            finding.location.dimmed(),
            finding.value_preview
        );

        if finding.confidence == Confidence::High {
            println!(
                "     {}  {}",
                glyph::WARN.yellow(),
                "matches a live key format, not a placeholder".dimmed()
            );
        }
        if let Some(url) = &finding.action_url {
            println!("     {}  revoke at {}", glyph::ARROW.dimmed(), url.cyan());
        }
        println!();
    }

    // One line, and only the counts that are non-zero — "0 medium-confidence
    // secrets" is noise, and "1 secrets" was simply wrong.
    let mut parts = vec![pluralize(results.secrets_found, "finding", "findings")];
    if results.high_confidence > 0 {
        parts.push(format!("{} high", results.high_confidence));
    }
    if results.medium_confidence > 0 {
        parts.push(format!("{} medium", results.medium_confidence));
    }
    if results.low_confidence > 0 {
        parts.push(format!("{} low", results.low_confidence));
    }
    println!("  {}", parts.join("  ·  ").bold());

    if results.has_critical_findings() {
        println!();
        ui::print_next_steps(&[
            "Revoke or rotate the keys above — assume they are compromised",
            "Remove them from history: git filter-repo --path .env --invert-paths",
            "Force push, after coordinating with everyone who has a clone",
        ]);
    }

    Ok(())
}

/// Render results in JSON format.
///
/// Suitable for machine parsing, CI/CD integration, and custom tooling.
///
/// # Arguments
///
/// * `results` - Scan results to serialize
fn render_json(results: &ScanResults) -> Result<()> {
    // `flatten` keeps every existing top-level key exactly where it was — adding
    // `summary` is purely additive, so anything already parsing `secrets_found`
    // or `high_confidence` keeps working.
    #[derive(serde::Serialize)]
    struct JsonReport<'a> {
        #[serde(flatten)]
        results: &'a ScanResults,
        summary: super::models::Summary,
    }

    let json = serde_json::to_string_pretty(&JsonReport {
        results,
        summary: results.summary(),
    })?;
    println!("{}", json);
    Ok(())
}

/// Split a finding's `location` into (file, line).
///
/// `location` is built as `path:line` or `path:line (VAR)`. Splitting from the
/// **right** matters: a Windows path like `C:\\src\\.env:8` contains a colon that is
/// not the separator, and splitting from the left would report the file as `C`.
fn split_location(location: &str) -> (&str, usize) {
    let head = location.split(" (").next().unwrap_or(location);
    match head.rsplit_once(':') {
        Some((file, line)) => (file, line.parse().unwrap_or(1)),
        None => (head, 1),
    }
}

/// Escape a value for a GitHub Actions workflow command.
///
/// GitHub's own spec: `%`, CR and LF must be encoded everywhere, and inside a
/// *property* value `:` and `,` must be encoded too — they are the property
/// delimiters, so a raw one silently truncates the annotation.
fn gh_escape(value: &str, is_property: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '%' => out.push_str("%25"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            ':' if is_property => out.push_str("%3A"),
            ',' if is_property => out.push_str("%2C"),
            _ => out.push(c),
        }
    }
    out
}

/// Render findings as GitHub Actions annotations.
///
/// ```text
/// ::error file=.env,line=8,title=Secret detected (high)::AWS_ACCESS_KEY_ID matches AWS Access Key
/// ```
///
/// GitHub renders these on the pull request diff, so a reviewer sees the flagged
/// line without opening the workflow log.
///
/// ⚠️ **The value never appears in the annotation** — only the variable name and
/// the pattern that matched. Annotations are shown on the PR and kept in the
/// workflow log, both of which are readable by more people than the branch is.
/// A secret scanner that publishes the secret in order to report it would be
/// doing the leaking itself, even in truncated form.
fn render_github(results: &ScanResults) -> Result<()> {
    for f in &results.findings {
        let level = match f.confidence {
            Confidence::High => "error",
            Confidence::Medium => "warning",
            Confidence::Low => "notice",
        };
        let (file, line) = split_location(&f.location);

        let what = f
            .variable
            .as_deref()
            .map(|v| format!("{v} matches {}", f.pattern))
            .unwrap_or_else(|| format!("{} detected", f.pattern));

        let message = match &f.action_url {
            Some(url) => format!("{what}. Rotate it at {url}"),
            None => format!("{what}. Rotate this credential at its source"),
        };

        println!(
            "::{level} file={},line={},title={}::{}",
            gh_escape(file, true),
            line,
            gh_escape(&format!("Secret detected ({})", f.confidence), true),
            gh_escape(&message, false),
        );
    }
    Ok(())
}

/// Render results in SARIF format for GitHub Code Scanning.
///
/// SARIF (Static Analysis Results Interchange Format) is a standard
/// format for reporting static analysis results. This output can be
/// uploaded to GitHub Security tab or other SARIF-compatible platforms.
///
/// # Arguments
///
/// * `results` - Scan results to convert
///
/// # References
///
/// - [SARIF Specification](https://docs.oasis-open.org/sarif/sarif/v2.1.0/)
/// - [GitHub Code Scanning](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/sarif-support-for-code-scanning)
fn render_sarif(results: &ScanResults) -> Result<()> {
    let sarif_results: Vec<serde_json::Value> = results
        .findings
        .iter()
        .map(|f| {
            let level = match f.confidence {
                Confidence::High => "error",
                Confidence::Medium => "warning",
                Confidence::Low => "note",
            };

            let (file, line) = split_location(&f.location);

            serde_json::json!({
                "ruleId": format!("secret/{}", f.pattern.to_lowercase().replace(' ', "-")),
                "level": level,
                "message": { "text": format!("{} detected", f.pattern) },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": file },
                        "region": { "startLine": line }
                    }
                }]
            })
        })
        .collect();

    let sarif = serde_json::json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "evnx scan",
                    "version": env!("CARGO_PKG_VERSION")
                }
            },
            "results": sarif_results
        }]
    });

    println!("{}", serde_json::to_string_pretty(&sarif)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::models::{Confidence, Finding};
    use super::*;

    #[test]
    fn test_output_format_from_str() {
        for (input, want) in [
            ("json", OutputFormat::Json),
            ("JSON", OutputFormat::Json),
            ("sarif", OutputFormat::Sarif),
            ("pretty", OutputFormat::Pretty),
            ("github", OutputFormat::Github),
            ("github-actions", OutputFormat::Github),
            ("  Github  ", OutputFormat::Github),
        ] {
            assert_eq!(input.parse::<OutputFormat>().unwrap(), want, "{input}");
        }

        // ⚠️ Previously this asserted `unknown` silently became Pretty.
        let err = "unknown".parse::<OutputFormat>().unwrap_err().to_string();
        assert!(err.contains("unknown format"), "{err}");
        assert!(
            err.contains("sarif"),
            "the error must list what is valid: {err}"
        );
    }

    #[test]
    fn test_render_json_structure() {
        let mut results = ScanResults::new(5);
        results.add_finding(Finding::new(
            "Test Key",
            Confidence::High,
            "test_...123",
            "test.rs:10",
            None,
            None,
        ));

        // Just verify it doesn't panic and produces valid JSON
        let result = render_json(&results);
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_sarif_structure() {
        let mut results = ScanResults::new(5);
        results.add_finding(Finding::new(
            "AWS Key",
            Confidence::High,
            "AKIA...XYZ",
            ".env:15",
            Some("AWS_KEY".to_string()),
            Some("https://aws.amazon.com".to_string()),
        ));

        let result = render_sarif(&results);
        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod github_tests {
    use super::*;

    #[test]
    fn location_splits_from_the_right() {
        assert_eq!(split_location(".env:8"), (".env", 8));
        assert_eq!(split_location(".env:8 (API_KEY)"), (".env", 8));
        assert_eq!(
            split_location("config/.env.staging:3"),
            ("config/.env.staging", 3)
        );
        // A drive letter is not the separator.
        assert_eq!(split_location(r"C:\src\.env:12"), (r"C:\src\.env", 12));
        // No line number at all.
        assert_eq!(split_location(".env"), (".env", 1));
    }

    #[test]
    fn property_values_escape_the_delimiters() {
        // `:` and `,` terminate a property, so a raw one truncates the annotation.
        assert_eq!(gh_escape("a,b", true), "a%2Cb");
        assert_eq!(gh_escape("a:b", true), "a%3Ab");
        // In the message body they are ordinary characters.
        assert_eq!(gh_escape("a,b:c", false), "a,b:c");
    }

    #[test]
    fn newlines_and_percent_escape_everywhere() {
        for is_property in [true, false] {
            assert_eq!(gh_escape("100%", is_property), "100%25");
            assert_eq!(gh_escape("a\nb", is_property), "a%0Ab");
            assert_eq!(gh_escape("a\r\nb", is_property), "a%0D%0Ab");
        }
    }
}
