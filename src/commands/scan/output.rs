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
use crate::utils::ui;
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
impl FromStr for OutputFormat {
    type Err = (); // Simple error type; could use a custom error if needed

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "sarif" => Ok(Self::Sarif),
            _ => Ok(Self::Pretty), // Default fallback
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
    // File summary using ui patterns
    if !files.is_empty() {
        let file_list = files
            .iter()
            .take(3)
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");

        println!("Scanning: {}", file_list.dimmed());
        if files.len() > 3 {
            println!("  ... and {} more files", files.len() - 3);
        }
        println!();
    }

    if results.secrets_found == 0 {
        ui::success("No secrets detected");
        println!("\nScanned {} files", results.files_scanned);
        return Ok(());
    }

    // Use colored output matching ui.rs style
    println!(
        "{} Found {} potential secrets\n",
        "✗".red(),
        results.secrets_found.to_string().red()
    );

    ui::print_section_header("🔍", "Secrets detected");

    for (i, finding) in results.findings.iter().enumerate() {
        let icon = match finding.confidence {
            Confidence::High => "🚨",
            Confidence::Medium => "⚠️ ",
            Confidence::Low => "ℹ️ ",
        };

        println!(
            "  {}. {} {} ({} confidence)",
            i + 1,
            icon,
            finding.pattern.bold(),
            finding.confidence
        );
        println!("     Pattern: {}", finding.pattern);
        println!("     Value: {}", finding.value_preview.dimmed());
        println!("     Location: {}", finding.location);

        if finding.confidence == Confidence::High {
            ui::warning("This looks like a real secret, not a placeholder.");
        }

        if let Some(url) = &finding.action_url {
            println!("     Action: Revoke immediately at {}", url.cyan());
        }
        println!();
    }

    // Summary section
    ui::print_section_header("📊", "Summary");
    println!(
        "  🚨 {} high-confidence secrets",
        results.high_confidence.to_string().red()
    );
    println!(
        "  ⚠️  {} medium-confidence secrets",
        results.medium_confidence.to_string().yellow()
    );
    if results.low_confidence > 0 {
        println!("  ℹ️  {} low-confidence detections", results.low_confidence);
    }

    println!(
        "\n  {}",
        "Recommendation: These should NOT be committed to Git."
            .yellow()
            .bold()
    );

    if results.has_critical_findings() {
        ui::print_next_steps(&[
            "Revoke/rotate all keys immediately",
            "Run: git filter-repo --path .env --invert-paths",
            "Force push (after team coordination)",
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
    let json = serde_json::to_string_pretty(results)?;
    println!("{}", json);
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

            let parts: Vec<&str> = f.location.split(':').collect();
            let file = parts.first().unwrap_or(&"unknown");
            let line: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

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
        assert_eq!("json".parse::<OutputFormat>(), Ok(OutputFormat::Json));
        assert_eq!("JSON".parse::<OutputFormat>(), Ok(OutputFormat::Json));
        assert_eq!("sarif".parse::<OutputFormat>(), Ok(OutputFormat::Sarif));
        assert_eq!("pretty".parse::<OutputFormat>(), Ok(OutputFormat::Pretty));
        assert_eq!("unknown".parse::<OutputFormat>(), Ok(OutputFormat::Pretty));
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
