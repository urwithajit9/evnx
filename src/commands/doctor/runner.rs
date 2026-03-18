//! Doctor command - diagnose and fix environment setup issues
//!
//! This module provides comprehensive diagnostics for project environment configuration,
//! including validation of `.env` files, Git configuration, project structure detection,
//! and security best practices.
//!
//! # Features
//! - 🔍 Multi-format output (text/JSON) for human and CI/CD consumption
//! - 🔧 Auto-fix mode for common issues (.gitignore, permissions)
//! - 🐍 Broad project detection (Python, Node, Rust, Go, Poetry, Pipenv)
//! - 🔐 Cross-platform permission checks with Windows fallbacks
//! - 🧪 `.env` syntax validation with helpful error messages
//!
//! # Environment Variables
//! - `EVNX_OUTPUT_JSON=1` - Output results as JSON (for CI/CD)
//! - `EVNX_AUTO_FIX=1` - Attempt to auto-fix detected issues
//!
//! # Usage
//! ```bash
//! evnx doctor                    # Check current directory
//! evnx doctor --path ./my-app   # Check specific project
//! evnx doctor --verbose         # Show detailed diagnostics
//! EVNX_OUTPUT_JSON=1 evnx doctor | jq '.summary.errors'  # CI/CD integration
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use colored::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// Import existing UI utilities from project
use super::types::*;
use crate::docs;
use crate::utils::ui;

// ─────────────────────────────────────────────────────────────
// Main Entry Point
// ─────────────────────────────────────────────────────────────

/// Run the doctor diagnostic checks
///
/// # Arguments
/// * `path` - Project directory to analyze (defaults to "." from CLI)
/// * `verbose` - Enable detailed output when true
///
/// # Returns
/// * `Ok(())` on success, `Err` on IO or parsing failures
///
/// # Environment Variables
/// * `EVNX_OUTPUT_JSON=1` - Output JSON instead of text
/// * `EVNX_AUTO_FIX=1` - Attempt to auto-fix detected issues
pub fn run(path: String, verbose: bool) -> Result<()> {
    let project_root = PathBuf::from(&path);

    let json_output = std::env::var("EVNX_OUTPUT_JSON")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "json"))
        .unwrap_or(false);

    let auto_fix = std::env::var("EVNX_AUTO_FIX")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false);

    let result = if json_output {
        run_json(&project_root, verbose, auto_fix)
    } else {
        run_text(&project_root, verbose, auto_fix)
    };

    // ✅ Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::DOCTOR);

    result
}

// ─────────────────────────────────────────────────────────────
// Text Output Mode (uses existing ui:: functions)
// ─────────────────────────────────────────────────────────────

fn run_text(project_root: &Path, verbose: bool, auto_fix: bool) -> Result<()> {
    // Use existing UI header function (requires subtitle param)
    ui::print_header("evnx doctor", Some("Diagnosing environment setup"));

    if verbose {
        ui::info(format!("Project path: {}", project_root.display()));
    }
    println!();

    let mut report = DiagnosticReport {
        project_path: project_root.to_string_lossy().to_string(),
        checks: Vec::new(),
        summary: Summary::new(),
        timestamp: Utc::now().to_rfc3339(),
    };

    // Run all checks
    let checks = get_all_checks();
    for check in checks {
        let mut result = check.run(project_root, verbose)?;

        // Attempt auto-fix if enabled and applicable
        if auto_fix && result.fixable && !result.fixed {
            if let Some(fix_fn) = result.fix_action {
                if fix_fn(project_root, verbose)? {
                    result.fixed = true;
                    result.severity = Severity::Ok;
                    if verbose {
                        ui::success(format!("✓ Fixed: {}", result.name));
                    }
                }
            }
        }

        report.summary.add(&result);
        report.checks.push(result.clone());

        // Print immediate result using existing UI utilities
        print_check_result_text(&result, verbose);
    }

    // Print summary
    print_summary_text(&report.summary);

    // Print recommendations if needed
    if report.summary.errors > 0 || (verbose && report.summary.warnings > 0) {
        print_recommendations_text(&report.checks, auto_fix);
    }

    // Exit with error code for CI/CD if critical issues exist
    if report.summary.errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// JSON Output Mode (for CI/CD integration)
// ─────────────────────────────────────────────────────────────

fn run_json(project_root: &Path, verbose: bool, auto_fix: bool) -> Result<()> {
    let mut report = DiagnosticReport {
        project_path: project_root.to_string_lossy().to_string(),
        checks: Vec::new(),
        summary: Summary::new(),
        timestamp: Utc::now().to_rfc3339(),
    };

    let checks = get_all_checks();
    for check in checks {
        let mut result = check.run(project_root, verbose)?;

        // Attempt auto-fix if enabled
        if auto_fix && result.fixable && !result.fixed {
            if let Some(fix_fn) = result.fix_action {
                if fix_fn(project_root, verbose)? {
                    result.fixed = true;
                    result.severity = Severity::Ok;
                }
            }
        }

        report.summary.add(&result);
        report.checks.push(result);
    }

    // Output JSON to stdout (machine-readable for CI/CD)
    let json =
        serde_json::to_string_pretty(&report).context("Failed to serialize diagnostic report")?;
    println!("{}", json);

    // Exit with error code for CI/CD if critical issues exist
    if report.summary.errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Check Definition and Execution
// ─────────────────────────────────────────────────────────────

/// Trait for individual diagnostic checks
trait DiagnosticCheck: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult>;
}

/// Registry of all available checks
fn get_all_checks() -> Vec<Box<dyn DiagnosticCheck>> {
    vec![
        Box::new(EnvFileCheck),
        Box::new(EnvExampleCheck),
        Box::new(ProjectStructureCheck),
        Box::new(DockerCheck),
        Box::new(PermissionCheck),
    ]
}

// ─────────────────────────────────────────────────────────────
// Individual Check Implementations
// ─────────────────────────────────────────────────────────────

/// Check: .env file existence, gitignore status, and syntax
struct EnvFileCheck;

impl DiagnosticCheck for EnvFileCheck {
    fn name(&self) -> &'static str {
        "env_file"
    }
    fn description(&self) -> &'static str {
        "Validate .env file existence, security, and syntax"
    }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let env_path = project_root.join(".env");

        if !env_path.exists() {
            return Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Warning,
                details: Some(".env file not found - create from .env.example".into()),
                fixable: false,
                fixed: false,
                fix_action: None,
            });
        }

        let mut details = Vec::new();
        let mut severity = Severity::Ok;

        // Check gitignore status
        let gitignored = is_gitignored(project_root, ".env")
            .unwrap_or_else(|_| fallback_gitignore_check(project_root, ".env"));

        if !gitignored {
            severity = Severity::Error;
            details.push("❌ .env is NOT in .gitignore (security risk)".into());
        } else if verbose {
            details.push("✓ .env is properly ignored by git".into());
        }

        // Validate syntax
        match validate_env_syntax(&env_path) {
            Ok(_) if verbose => details.push("✓ .env syntax is valid".into()),
            Ok(_) => {}
            Err(e) => {
                if severity == Severity::Ok {
                    severity = Severity::Warning;
                }
                details.push(format!("⚠️ Syntax issues: {}", e));
            }
        }

        Ok(CheckResult {
            name: self.name().to_string(),
            description: self.description().to_string(),
            severity,
            details: if details.is_empty() {
                None
            } else {
                Some(details.join("\n"))
            },
            fixable: severity == Severity::Error,
            fixed: false,
            fix_action: Some(fix_env_gitignore),
        })
    }
}

fn fix_env_gitignore(project_root: &Path, verbose: bool) -> Result<bool> {
    let gitignore_path = project_root.join(".gitignore");

    if !is_gitignored(project_root, ".env").unwrap_or(false) {
        add_to_gitignore(&gitignore_path, ".env")?;
        if verbose {
            ui::success("Added .env to .gitignore");
        }
        return Ok(true);
    }
    Ok(false)
}

/// Check: .env.example existence and Git tracking
struct EnvExampleCheck;

impl DiagnosticCheck for EnvExampleCheck {
    fn name(&self) -> &'static str {
        "env_example"
    }
    fn description(&self) -> &'static str {
        "Check .env.example exists and is Git-tracked"
    }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let example_path = project_root.join(".env.example");

        if !example_path.exists() {
            return Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Warning,
                details: Some(".env.example not found - consider creating a template".into()),
                fixable: false,
                fixed: false,
                fix_action: None,
            });
        }

        let is_tracked = Command::new("git")
            .args([
                "-C",
                project_root.to_string_lossy().as_ref(),
                "ls-files",
                "--error-unmatch",
                ".env.example",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !is_tracked {
            Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Warning,
                details: Some(".env.example is NOT tracked in Git (recommended)".into()),
                fixable: true,
                fixed: false,
                fix_action: Some(fix_track_env_example),
            })
        } else {
            Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Ok,
                details: if verbose {
                    Some("✓ File is tracked in Git".into())
                } else {
                    None
                },
                fixable: false,
                fixed: false,
                fix_action: None,
            })
        }
    }
}

fn fix_track_env_example(_project_root: &Path, verbose: bool) -> Result<bool> {
    // Note: We can't actually git add from here without user confirmation
    if verbose {
        ui::info("To track .env.example: git add .env.example && git commit -m 'Add env template'");
    }
    Ok(false) // Requires manual git command
}

/// Check: Project type detection and dependency validation
struct ProjectStructureCheck;

impl DiagnosticCheck for ProjectStructureCheck {
    fn name(&self) -> &'static str {
        "project_structure"
    }
    fn description(&self) -> &'static str {
        "Detect project type and validate dependencies"
    }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let detected = detect_project_type(project_root);
        let mut details = Vec::new();
        let mut severity = Severity::Info;
        let mut warnings = 0usize;

        if let Some(project_type) = detected.as_deref() {
            if project_type.starts_with("Python") {
                details.push(format!("✓ Detected Python project ({})", project_type));
                if project_type.contains("requirements") {
                    if !check_requirements_txt_has_dotenv(project_root)? {
                        details.push("⚠️ python-dotenv not in requirements.txt".into());
                        warnings += 1;
                    } else if verbose {
                        details.push("✓ python-dotenv dependency found".into());
                    }
                } else if !check_pyproject_has_dotenv(project_root)? && verbose {
                    details.push("ℹ️ Consider adding python-dotenv or pydantic-settings".into());
                }
            } else if project_type == "Node.js" {
                details.push("✓ Detected Node.js project".into());
                if verbose && !check_package_json_has_dotenv(project_root)? {
                    details.push("ℹ️ Consider adding 'dotenv' package".into());
                }
            } else if project_type == "Rust" {
                details.push("✓ Detected Rust project".into());
                if verbose && !check_cargo_has_dotenv(project_root)? {
                    details.push("ℹ️ Consider adding 'dotenvy' crate".into());
                }
            } else {
                details.push(format!("✓ Detected {} project", project_type));
            }
            severity = Severity::Ok;
        } else {
            details.push("ℹ️ No recognized project configuration".into());
            if verbose {
                details.push("Supported: requirements.txt, pyproject.toml, Pipfile, poetry.lock, package.json, Cargo.toml, go.mod, composer.json".into());
            }
        }

        if warnings > 0 && severity == Severity::Ok {
            severity = Severity::Warning;
        }

        Ok(CheckResult {
            name: self.name().to_string(),
            description: self.description().to_string(),
            severity,
            details: if details.is_empty() {
                None
            } else {
                Some(details.join("\n"))
            },
            fixable: false,
            fixed: false,
            fix_action: None,
        })
    }
}

/// Check: Docker configuration presence
struct DockerCheck;

impl DiagnosticCheck for DockerCheck {
    fn name(&self) -> &'static str {
        "docker_config"
    }
    fn description(&self) -> &'static str {
        "Check for Docker configuration files"
    }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let docker_files = [
            "docker-compose.yml",
            "docker-compose.yaml",
            "Dockerfile",
            "Containerfile",
            ".dockerignore",
        ];

        let found: Vec<&str> = docker_files
            .iter()
            .copied()
            .filter(|f| project_root.join(f).exists())
            .collect();

        if found.is_empty() {
            Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Info,
                details: Some("No Docker configuration detected".into()),
                fixable: false,
                fixed: false,
                fix_action: None,
            })
        } else {
            Ok(CheckResult {
                name: self.name().to_string(),
                description: self.description().to_string(),
                severity: Severity::Ok,
                details: if verbose {
                    Some(format!("Found: {}", found.join(", ")))
                } else {
                    None
                },
                fixable: false,
                fixed: false,
                fix_action: None,
            })
        }
    }
}

/// Check: File permissions for sensitive files (Unix only)
struct PermissionCheck;

impl DiagnosticCheck for PermissionCheck {
    fn name(&self) -> &'static str {
        "file_permissions"
    }
    fn description(&self) -> &'static str {
        "Check file permissions for sensitive files"
    }

    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let env_path = project_root.join(".env");

        if !env_path.exists() {
            return Ok(CheckResult {
                name: self.name().to_string(),
                description: "Skipped: .env not found".into(),
                severity: Severity::Info,
                details: None,
                fixable: false,
                fixed: false,
                fix_action: None,
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            match fs::metadata(&env_path) {
                Ok(metadata) => {
                    let mode = metadata.permissions().mode() & 0o777;
                    let is_secure = mode == 0o600 || mode == 0o400;

                    if is_secure {
                        Ok(CheckResult {
                            name: self.name().to_string(),
                            description: self.description().to_string(),
                            severity: Severity::Ok,
                            details: if verbose {
                                Some(format!("Permissions: {:o} (secure)", mode))
                            } else {
                                None
                            },
                            fixable: false,
                            fixed: false,
                            fix_action: None,
                        })
                    } else {
                        Ok(CheckResult {
                            name: self.name().to_string(),
                            description: self.description().to_string(),
                            severity: Severity::Warning,
                            details: Some(format!("Permissions: {:o} (recommended: 600)", mode)),
                            fixable: true,
                            fixed: false,
                            fix_action: Some(fix_file_permissions),
                        })
                    }
                }
                Err(e) => Ok(CheckResult {
                    name: self.name().to_string(),
                    description: self.description().to_string(),
                    severity: Severity::Warning,
                    details: Some(format!("Could not check permissions: {}", e)),
                    fixable: false,
                    fixed: false,
                    fix_action: None,
                }),
            }
        }

        #[cfg(not(unix))]
        {
            Ok(CheckResult {
                name: self.name().to_string(),
                description: "Skipped: Windows platform".into(),
                severity: Severity::Info,
                details: if verbose {
                    Some("Permission checks require Unix-like system".into())
                } else {
                    None
                },
                fixable: false,
                fixed: false,
                fix_action: None,
            })
        }
    }
}

#[cfg(unix)]
fn fix_file_permissions(project_root: &Path, verbose: bool) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let env_path = project_root.join(".env");
    if !env_path.exists() {
        return Ok(false);
    }

    let mut perms = fs::metadata(&env_path)?.permissions();
    let current = perms.mode() & 0o777;
    perms.set_mode(0o600);
    fs::set_permissions(&env_path, perms)?;

    if verbose {
        ui::success(format!("Fixed permissions: {:o} → 600", current));
    }
    Ok(true)
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

fn is_gitignored(project_root: &Path, filename: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "-C",
            project_root.to_string_lossy().as_ref(),
            "check-ignore",
            filename,
        ])
        .output()
        .context("Failed to execute git check-ignore")?;
    Ok(output.status.success())
}

fn fallback_gitignore_check(project_root: &Path, filename: &str) -> bool {
    let gitignore_path = project_root.join(".gitignore");
    if !gitignore_path.exists() {
        return false;
    }

    fs::read_to_string(&gitignore_path)
        .ok()
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .any(|line| line == filename || line.starts_with(&format!("/{}", filename)))
        })
        .unwrap_or(false)
}

fn add_to_gitignore(gitignore_path: &Path, pattern: &str) -> Result<()> {
    if gitignore_path.exists() {
        let content = fs::read_to_string(gitignore_path)?;
        if content.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        fs::write(
            gitignore_path,
            format!("{}\n{}\n", content.trim_end(), pattern),
        )?;
    } else {
        fs::write(gitignore_path, format!("{}\n", pattern))?;
    }
    Ok(())
}

fn validate_env_syntax(path: &Path) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let line_re = Regex::new(r"^(?:\s*$|\s*#.*|\s*[A-Za-z_][A-Za-z0-9_]*\s*=.*)$")
        .context("Invalid regex pattern")?;

    for (idx, line) in content.lines().enumerate() {
        if !line_re.is_match(line) {
            let snippet = line.chars().take(50).collect::<String>();
            return Err(anyhow::anyhow!(
                "Line {}: invalid syntax '{}{}'",
                idx + 1,
                snippet,
                if line.len() > 50 { "..." } else { "" }
            ));
        }
    }
    Ok(())
}

fn detect_project_type(project_root: &Path) -> Option<String> {
    // Python ecosystem (priority order)
    if project_root.join("poetry.lock").exists()
        || (project_root.join("pyproject.toml").exists()
            && fs::read_to_string(project_root.join("pyproject.toml"))
                .ok()?
                .contains("[tool.poetry]"))
    {
        return Some("Python (Poetry)".to_string());
    }
    if project_root.join("Pipfile").exists() {
        return Some("Python (Pipenv)".to_string());
    }
    if project_root.join("pyproject.toml").exists() {
        return Some("Python (pyproject)".to_string());
    }
    if project_root.join("requirements.txt").exists() {
        return Some("Python (requirements)".to_string());
    }
    // Other languages
    if project_root.join("package.json").exists() {
        return Some("Node.js".to_string());
    }
    if project_root.join("Cargo.toml").exists() {
        return Some("Rust".to_string());
    }
    if project_root.join("go.mod").exists() {
        return Some("Go".to_string());
    }
    if project_root.join("composer.json").exists() {
        return Some("PHP".to_string());
    }
    None
}

fn check_requirements_txt_has_dotenv(project_root: &Path) -> Result<bool> {
    let content = fs::read_to_string(project_root.join("requirements.txt"))?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .any(|line| {
            line.starts_with("python-dotenv")
                || (line.starts_with("dotenv") && !line.contains("django-dotenv"))
        }))
}

fn check_pyproject_has_dotenv(project_root: &Path) -> Result<bool> {
    let content = fs::read_to_string(project_root.join("pyproject.toml"))?;
    Ok(content.contains("python-dotenv")
        || content.contains("pydantic-settings")
        || content.contains("dynaconf"))
}

fn check_package_json_has_dotenv(project_root: &Path) -> Result<bool> {
    let content = fs::read_to_string(project_root.join("package.json"))?;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        let deps = json
            .get("dependencies")
            .or_else(|| json.get("devDependencies"));
        if let Some(deps) = deps.and_then(|d| d.as_object()) {
            return Ok(deps.contains_key("dotenv") || deps.contains_key("dotenv-cli"));
        }
    }
    Ok(false)
}

fn check_cargo_has_dotenv(project_root: &Path) -> Result<bool> {
    let content = fs::read_to_string(project_root.join("Cargo.toml"))?;
    Ok(content.contains("dotenvy") || content.contains("dotenv"))
}

// ─────────────────────────────────────────────────────────────
// Output Formatting (Text Mode) - Uses EXISTING ui:: functions
// ─────────────────────────────────────────────────────────────

/// Print a check result using existing UI utilities
///
/// Since ui.rs doesn't have print_check_item, we use direct println with colored output
fn print_check_result_text(result: &CheckResult, verbose: bool) {
    // Use colored output directly since ui module doesn't have check item function
    println!(
        "  {} {}",
        result.severity.colored_icon(),
        result.name.bold()
    );

    if verbose || result.severity != Severity::Ok {
        if let Some(ref details) = result.details {
            for line in details.lines() {
                println!("    {}", line.dimmed());
            }
        }
    }
    println!();
}

fn print_summary_text(summary: &Summary) {
    println!("\n{}", "Summary:".bold());

    if summary.errors > 0 {
        println!(
            "  🚨 {} critical issue{}",
            summary.errors,
            if summary.errors > 1 { "s" } else { "" }
        );
    }
    if summary.warnings > 0 {
        println!(
            "  ⚠️  {} warning{}",
            summary.warnings,
            if summary.warnings > 1 { "s" } else { "" }
        );
    }
    if summary.passed > 0 {
        println!("  ✓ {} checks passed", summary.passed.to_string().green());
    }

    let health = if summary.errors == 0 && summary.warnings == 0 {
        "✓ Excellent".green()
    } else if summary.errors == 0 {
        "⚠️ Needs attention".yellow()
    } else {
        "🚨 Action required".red()
    };
    println!("\nOverall health: {}", health);
}

fn print_recommendations_text(checks: &[CheckResult], auto_fix_enabled: bool) {
    let fixable: Vec<_> = checks.iter().filter(|c| c.fixable && !c.fixed).collect();

    if !fixable.is_empty() {
        // Use existing UI section header
        ui::print_section_header("🔧", "Recommendations");

        if auto_fix_enabled {
            ui::success("Auto-fix mode was enabled - issues attempted");
        } else {
            ui::info("Run with EVNX_AUTO_FIX=1 to auto-correct issues");
        }
        println!("  Or manually address the following:");
        for check in fixable {
            println!("    • {} ({})", check.name, check.description);
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_validate_env_syntax_valid() {
        let dir = TempDir::new().unwrap();
        let env_path = dir.path().join(".env");
        fs::write(&env_path, "FOO=bar\n# comment\nEMPTY=\n").unwrap();
        assert!(validate_env_syntax(&env_path).is_ok());
    }

    #[test]
    fn test_validate_env_syntax_invalid() {
        let dir = TempDir::new().unwrap();
        let env_path = dir.path().join(".env");
        fs::write(&env_path, "INVALID LINE\n").unwrap();
        assert!(validate_env_syntax(&env_path).is_err());
    }

    #[test]
    fn test_detect_project_type() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();
        assert_eq!(
            detect_project_type(dir.path()),
            Some("Python (requirements)".to_string())
        );
    }

    #[test]
    fn test_fallback_gitignore_check() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
        assert!(fallback_gitignore_check(dir.path(), ".env"));
        assert!(!fallback_gitignore_check(dir.path(), ".env.local"));
    }

    #[test]
    fn test_severity_serialization() {
        // Test JSON serialization for CI/CD integration
        let severity = Severity::Warning;
        let json = serde_json::to_string(&severity).unwrap();
        assert_eq!(json, r#""warning""#);

        let parsed: Severity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Severity::Warning);
    }
}
