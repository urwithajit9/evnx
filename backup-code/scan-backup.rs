/// Secret scanning command
///
/// Scans files for accidentally committed secrets using pattern matching
/// and entropy analysis. Outputs findings with confidence levels and
/// remediation steps.
use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
// use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::core::Parser;
use crate::utils::patterns::{detect_secret, Confidence};

/// A detected secret
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub pattern: String,
    pub confidence: String,
    pub value_preview: String,
    pub location: String,
    pub variable: Option<String>,
    pub action_url: Option<String>,
}

/// Scan results
#[derive(Debug, Serialize, Deserialize)]
pub struct ScanResults {
    pub files_scanned: usize,
    pub secrets_found: usize,
    pub findings: Vec<Finding>,
    pub high_confidence: usize,
    pub medium_confidence: usize,
    pub low_confidence: usize,
}

/// Run the scan command
pub fn run(
    paths: Vec<String>,
    exclude: Vec<String>,
    _pattern: Vec<String>,
    ignore_placeholders: bool,
    format: String,
    exit_zero: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running scan in verbose mode".dimmed());
    }

    println!(
        "\n{}",
        "┌─ Scanning for exposed secrets ──────────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Checking for real-looking credentials               │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    // Collect files to scan
    let files = collect_files(&paths, &exclude)?;

    if verbose {
        println!("Scanning {} files...", files.len());
    }

    let mut results = ScanResults {
        files_scanned: files.len(),
        secrets_found: 0,
        findings: Vec::new(),
        high_confidence: 0,
        medium_confidence: 0,
        low_confidence: 0,
    };

    // Scan each file
    for file in &files {
        if verbose {
            println!("  Scanning: {}", file.display());
        }
        scan_file(file, &mut results, ignore_placeholders)?;
    }

    // Output results
    match format.as_str() {
        "json" => output_json(&results)?,
        "sarif" => output_sarif(&results)?,
        _ => output_pretty(&results, &files)?,
    }

    // Exit code
    if !exit_zero && results.secrets_found > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Collect files to scan
fn collect_files(paths: &[String], exclude: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for path_str in paths {
        let path = Path::new(path_str);

        if path.is_file() {
            if !should_exclude(path, exclude) {
                files.push(path.to_path_buf());
            }
        } else if path.is_dir() {
            for entry in WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let entry_path = entry.path();
                if entry_path.is_file() && !should_exclude(entry_path, exclude) {
                    // Only scan text-like files
                    if is_scannable_file(entry_path) {
                        files.push(entry_path.to_path_buf());
                    }
                }
            }
        }
    }

    Ok(files)
}

/// Check if a file should be excluded
fn should_exclude(path: &Path, exclude: &[String]) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in exclude {
        // Simple glob matching
        if pattern.contains('*') {
            let pattern = pattern.replace('*', "");
            if path_str.contains(&pattern) {
                return true;
            }
        } else if path_str.contains(pattern) {
            return true;
        }
    }

    // Always exclude common non-secret files
    let always_exclude = [
        ".git/",
        "node_modules/",
        "target/",
        "dist/",
        "build/",
        ".env.example",
        ".env.sample",
        ".env.template",
    ];

    for pattern in &always_exclude {
        if path_str.contains(pattern) {
            return true;
        }
    }

    false
}

/// Check if a file is scannable (text-based)
fn is_scannable_file(path: &Path) -> bool {
    let scannable_extensions = [
        "env",
        "txt",
        "sh",
        "bash",
        "zsh",
        "py",
        "js",
        "ts",
        "rs",
        "go",
        "java",
        "rb",
        "php",
        "yml",
        "yaml",
        "json",
        "toml",
        "xml",
        "conf",
        "config",
        "ini",
        "properties",
    ];

    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        if scannable_extensions.contains(&ext_str.as_str()) {
            return true;
        }
    }

    // Files without extensions (Dockerfile, Makefile, etc.)
    if path.extension().is_none() {
        if let Some(name) = path.file_name() {
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".env")
                || name_str == "Dockerfile"
                || name_str == "Makefile"
                || name_str == "docker-compose.yml"
            {
                return true;
            }
        }
    }

    false
}

/// Scan a single file for secrets
fn scan_file(path: &Path, results: &mut ScanResults, ignore_placeholders: bool) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Skip binary files
    };

    // If it's a .env file, parse it properly
    if path.to_string_lossy().contains(".env") {
        scan_env_file(path, &content, results, ignore_placeholders)?;
    } else {
        // Scan line by line for secrets
        scan_text_file(path, &content, results, ignore_placeholders)?;
    }

    Ok(())
}

/// Scan a .env file using the parser
fn scan_env_file(
    path: &Path,
    content: &str,
    results: &mut ScanResults,
    ignore_placeholders: bool,
) -> Result<()> {
    let parser = Parser::default();
    let _vars = match parser.parse_content(content) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Skip invalid files
    };

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num + 1;

        // Skip comments and empty lines
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }

        // Extract key=value
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().trim_start_matches("export").trim();
            let value = value.trim();

            if let Some((pattern, confidence, action_url)) = detect_secret(value, key) {
                // Skip if ignoring placeholders and this is one
                if ignore_placeholders && crate::utils::patterns::is_placeholder(value) {
                    continue;
                }

                let finding = Finding {
                    pattern: pattern.clone(),
                    confidence: format!("{}", confidence),
                    value_preview: truncate_value(value),
                    location: format!("{}:{} ({})", path.display(), line_num, key),
                    variable: Some(key.to_string()),
                    action_url,
                };

                results.findings.push(finding);
                results.secrets_found += 1;

                match confidence {
                    Confidence::High => results.high_confidence += 1,
                    Confidence::Medium => results.medium_confidence += 1,
                    Confidence::Low => results.low_confidence += 1,
                }
            }
        }
    }

    Ok(())
}

/// Scan a text file line by line
fn scan_text_file(
    path: &Path,
    content: &str,
    results: &mut ScanResults,
    ignore_placeholders: bool,
) -> Result<()> {
    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num + 1;

        // Try to detect secrets in the line
        // Split by common separators to find tokens
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == '=' || c == ':' || c == '"' || c == '\'')
            .filter(|t| t.len() > 20) // Only check reasonably long tokens
            .collect();

        for token in tokens {
            if let Some((pattern, confidence, action_url)) = detect_secret(token, "") {
                if ignore_placeholders && crate::utils::patterns::is_placeholder(token) {
                    continue;
                }

                let finding = Finding {
                    pattern: pattern.clone(),
                    confidence: format!("{}", confidence),
                    value_preview: truncate_value(token),
                    location: format!("{}:{}", path.display(), line_num),
                    variable: None,
                    action_url,
                };

                results.findings.push(finding);
                results.secrets_found += 1;

                match confidence {
                    Confidence::High => results.high_confidence += 1,
                    Confidence::Medium => results.medium_confidence += 1,
                    Confidence::Low => results.low_confidence += 1,
                }
            }
        }
    }

    Ok(())
}

/// Truncate a value for display (show first and last few chars)
const PREFIX_LEN: usize = 8;
const SUFFIX_LEN: usize = 5;
const MAX_VISIBLE: usize = 20;

fn truncate_value(value: &str) -> String {
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

/// Output results in pretty format
fn output_pretty(results: &ScanResults, files: &[PathBuf]) -> Result<()> {
    println!(
        "Scanning: {}",
        files
            .iter()
            .take(3)
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if files.len() > 3 {
        println!("  ... and {} more files", files.len() - 3);
    }
    println!();

    if results.secrets_found == 0 {
        println!("{} No secrets detected", "✓".green());
        println!("\nScanned {} files", results.files_scanned);
        return Ok(());
    }

    println!(
        "{} Found {} potential secrets\n",
        "✗".red(),
        results.secrets_found
    );

    println!("{}", "Secrets detected:".bold());
    for (i, finding) in results.findings.iter().enumerate() {
        let icon = match finding.confidence.as_str() {
            "high" => "🚨",
            "medium" => "⚠️ ",
            _ => "ℹ️ ",
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

        if finding.confidence == "high" {
            println!(
                "     {}",
                "This looks like a real secret, not a placeholder.".yellow()
            );
        }

        if let Some(url) = &finding.action_url {
            println!("     Action: Revoke immediately at {}", url.cyan());
        }
        println!();
    }

    println!("{}", "Summary:".bold());
    println!("  🚨 {} high-confidence secrets", results.high_confidence);
    println!(
        "  ⚠️  {} medium-confidence secrets",
        results.medium_confidence
    );
    if results.low_confidence > 0 {
        println!("  ℹ️ {} low-confidence detections", results.low_confidence);
    }

    println!(
        "\n  {}",
        "Recommendation: These should NOT be committed to Git."
            .yellow()
            .bold()
    );

    if results.high_confidence > 0 || results.medium_confidence > 0 {
        println!("\n  If already committed:");
        println!("    1. Revoke/rotate all keys immediately");
        println!("    2. Run: git filter-repo --path .env --invert-paths");
        println!("    3. Force push (after team coordination)");
    }

    Ok(())
}

/// Output results in JSON format
fn output_json(results: &ScanResults) -> Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    println!("{}", json);
    Ok(())
}

/// Output results in SARIF format (for GitHub Code Scanning)
fn output_sarif(results: &ScanResults) -> Result<()> {
    let sarif_results: Vec<serde_json::Value> = results
        .findings
        .iter()
        .map(|f| {
            let level = match f.confidence.as_str() {
                "high" => "error",
                "medium" => "warning",
                _ => "note",
            };

            // Parse location to extract file and line
            let parts: Vec<&str> = f.location.split(':').collect();
            let file = parts.first().unwrap_or(&"unknown");
            let line: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

            serde_json::json!({
                "ruleId": format!("secret/{}", f.pattern.to_lowercase().replace(' ', "-")),
                "level": level,
                "message": {
                    "text": format!("{} detected", f.pattern)
                },
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
    use super::*;

    #[test]
    fn test_truncate_value() {
        assert_eq!(truncate_value("short"), "short");
        assert_eq!(
            truncate_value("this_is_a_very_long_secret_key_value_12345678"),
            "this_is_...45678"
        );
    }

    #[test]
    fn test_is_scannable_file() {
        assert!(is_scannable_file(Path::new(".env")));
        assert!(is_scannable_file(Path::new("config.py")));
        assert!(is_scannable_file(Path::new("Dockerfile")));
        assert!(!is_scannable_file(Path::new("image.png")));
    }

    #[test]
    fn test_should_exclude() {
        assert!(should_exclude(Path::new(".env.example"), &[]));
        assert!(should_exclude(Path::new("node_modules/package.json"), &[]));
        assert!(!should_exclude(Path::new(".env"), &[]));

        assert!(should_exclude(Path::new("test.py"), &["test*".to_string()]));
    }
}
