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
use crate::core::gitignore;
use crate::docs;
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;

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
/// # Exit codes
///
/// | code | meaning |
/// |------|---------|
/// | 0 | healthy — no errors, and no warnings under `--strict` |
/// | 1 | problems found |
/// | 2 | doctor could not run |
///
/// ⚠️ **2 is not a louder 1**, the same distinction `scan` and `sync --check`
/// make. A directory that cannot be read and a directory that is merely unhealthy
/// are different answers, and a CI step that treats them alike will one day pass
/// because the checkout was empty.
///
/// Warnings exit 0 unless `--strict` is given. A missing `.env.example` is worth
/// saying and is not worth failing a build that never asked about it.
///
/// # Environment Variables
/// * `EVNX_OUTPUT_JSON=1` - Same as `--format json`, kept because it shipped first
/// * `EVNX_AUTO_FIX=1` - Same as `--fix`, kept because it shipped first
pub fn run(
    path: String,
    verbose: bool,
    fix: bool,
    strict: bool,
    format: Option<String>,
) -> Result<()> {
    let project_root = PathBuf::from(&path);

    // ⚠️ Checked before anything else. Diagnosing a directory that is not there
    // would otherwise report "no .env file" — a finding about the project rather
    // than about the path, which is the shape of fail-open `scan` just had fixed.
    if !project_root.is_dir() {
        eprintln!(
            "{} {} is not a directory evnx can read",
            "Error:".on_red().bold(),
            project_root.display()
        );
        eprintln!();
        eprintln!("No verdict: doctor examined nothing. This is not a clean result.");
        std::process::exit(EXIT_ERROR);
    }

    // ⚠️ Precedence, not "either wins" — unlike `fix` below. A format is a
    // choice between two answers rather than a guard, so an explicit
    // `--format pretty` must be able to override `EVNX_OUTPUT_JSON=1` exported
    // by a CI image. `Option<String>` is what makes "asked for pretty"
    // distinguishable from "did not ask".
    let json_output = match format.as_deref() {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "json" => true,
            "pretty" | "text" => false,
            other => {
                eprintln!(
                    "{} unknown format '{other}' — expected one of: pretty, json",
                    "Error:".on_red().bold()
                );
                eprintln!();
                eprintln!("No verdict: doctor examined nothing. This is not a clean result.");
                std::process::exit(EXIT_ERROR);
            }
        },
        None => std::env::var("EVNX_OUTPUT_JSON")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "json"))
            .unwrap_or(false),
    };

    // Either source saying yes is enough. `EVNX_AUTO_FIX` shipped first and is
    // documented, so it keeps working; a flag can turn repair on, never off.
    let auto_fix = fix
        || std::env::var("EVNX_AUTO_FIX")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
            .unwrap_or(false);

    let result = if json_output {
        run_json(&project_root, verbose, auto_fix, strict)
    } else {
        run_text(&project_root, verbose, auto_fix, strict)
    };

    // ✅ Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::DOCTOR);

    result
}

/// Healthy — no errors, and no warnings under `--strict`.
pub const EXIT_HEALTHY: i32 = 0;
/// Problems found.
pub const EXIT_PROBLEMS: i32 = 1;
/// Doctor could not run, so there is no verdict.
pub const EXIT_ERROR: i32 = 2;

/// Drop a detail line's marker when it only repeats the check's own severity.
///
/// ⚠️ Applied once, before either renderer sees the result — so the terminal and
/// the JSON agree, and `details` is prose in both.
///
/// Detail lines are per-item: `env_file` emits one per `.env*` file, so a line
/// whose marker *differs* from the check's is saying something the check-level
/// severity does not, and keeps it. A line that repeats it is saying the same
/// thing twice — in the terminal that rendered as `✗` on two consecutive lines,
/// and in the JSON it put a presentation glyph inside a machine-readable field
/// whose `severity` sibling already carried the answer.
fn normalise_details(result: &mut CheckResult) {
    let own = format!("{} ", result.severity.icon());
    if let Some(details) = &result.details {
        let cleaned: Vec<&str> = details
            .lines()
            .map(|line| line.strip_prefix(&own).unwrap_or(line))
            .collect();
        result.details = Some(cleaned.join("\n"));
    }
}

/// Whether the report should be treated as a failure.
///
/// Split out so text and JSON modes cannot drift apart on the one thing CI reads.
fn is_failure(summary: &Summary, strict: bool) -> bool {
    summary.errors > 0 || (strict && summary.warnings > 0)
}

// ─────────────────────────────────────────────────────────────
// Text Output Mode (uses existing ui:: functions)
// ─────────────────────────────────────────────────────────────

fn run_text(project_root: &Path, verbose: bool, auto_fix: bool, strict: bool) -> Result<()> {
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

        normalise_details(&mut result);
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

    if is_failure(&report.summary, strict) {
        std::process::exit(EXIT_PROBLEMS);
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// JSON Output Mode (for CI/CD integration)
// ─────────────────────────────────────────────────────────────

fn run_json(project_root: &Path, verbose: bool, auto_fix: bool, strict: bool) -> Result<()> {
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

        normalise_details(&mut result);
        report.summary.add(&result);
        report.checks.push(result);
    }

    // Output JSON to stdout (machine-readable for CI/CD)
    let json =
        serde_json::to_string_pretty(&report).context("Failed to serialize diagnostic report")?;
    println!("{}", json);

    if is_failure(&report.summary, strict) {
        std::process::exit(EXIT_PROBLEMS);
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
        // ⚠️ Every `.env*` file, not only `.env`.
        //
        // This check looked at `.env` alone, so a project with `.env.production`
        // sitting unignored got a green tick — evnx actively reported that the
        // setup was fine while the file holding production credentials was
        // committable. Reporting "healthy" over a real exposure is worse than
        // saying nothing.
        let env_files = secret_bearing_env_files(project_root);

        if env_files.is_empty() {
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

        for name in &env_files {
            let gitignored = gitignore::is_gitignored(project_root, name)
                .unwrap_or_else(|_| gitignore::fallback_gitignore_check(project_root, name));

            if !gitignored {
                severity = Severity::Error;
                details.push(format!("✗ {name} is NOT in .gitignore (security risk)"));
            } else if verbose {
                details.push(format!("✓ {name} is properly ignored by git"));
            }

            match validate_env_syntax(&project_root.join(name)) {
                Ok(_) if verbose => details.push(format!("✓ {name} syntax is valid")),
                Ok(_) => {}
                Err(e) => {
                    if severity == Severity::Ok {
                        severity = Severity::Warning;
                    }
                    details.push(format!("! {name}: {e}"));
                }
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

/// Every `.env*` in `project_root` that is meant to hold real values.
///
/// The `.env.example` / `.env.sample` / `.env.template` family is excluded: those
/// are supposed to be committed, so "not gitignored" is correct for them. So are
/// `evnx backup` artefacts, which are ciphertext.
fn secret_bearing_env_files(project_root: &Path) -> Vec<String> {
    const TEMPLATES: &[&str] = &[".env.example", ".env.sample", ".env.template"];

    let Ok(entries) = fs::read_dir(project_root) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(".env"))
        .filter(|n| !TEMPLATES.contains(&n.as_str()))
        .filter(|n| !n.contains(".backup"))
        .collect();

    names.sort();
    names
}

fn fix_env_gitignore(project_root: &Path, verbose: bool) -> Result<bool> {
    let gitignore_path = project_root.join(".gitignore");
    let mut changed = false;

    // Mirrors what `evnx init` writes — one pattern covering every env file, with
    // the committable template family carved back out. Adding `.env` alone would
    // leave `.env.production` exposed, which is the bug this fix exists for.
    for entry in crate::core::gitignore::ENV_ENTRIES {
        if !gitignore::is_gitignored(project_root, entry).unwrap_or(false) {
            add_to_gitignore(&gitignore_path, entry)?;
            changed = true;
        }
    }

    if changed && verbose {
        ui::success("Added .env* to .gitignore (keeping .env.example committable)");
    }
    Ok(changed)
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
                // ⚠️ Not `fixable`. It was, with a `fix_action` that only printed
                // advice and returned `Ok(false)` — so the summary counted a fix
                // that could never happen. Harmless while auto-fix was an
                // undocumented environment variable; a lie once `--fix` is a flag
                // someone runs expecting the count to go down.
                //
                // Staging a file is also not doctor's call. `git add` puts
                // content in the next commit, and a tool that does that
                // unasked is one you stop trusting near a repository.
                details: Some(
                    ".env.example is not tracked in Git — run: git add .env.example".into(),
                ),
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
                        details.push("! python-dotenv not in requirements.txt".into());
                        warnings += 1;
                    } else if verbose {
                        details.push("✓ python-dotenv dependency found".into());
                    }
                } else if !check_pyproject_has_dotenv(project_root)? && verbose {
                    details.push("· Consider adding python-dotenv or pydantic-settings".into());
                }
            } else if project_type == "Node.js" {
                details.push("✓ Detected Node.js project".into());
                if verbose && !check_package_json_has_dotenv(project_root)? {
                    details.push("· Consider adding 'dotenv' package".into());
                }
            } else if project_type == "Rust" {
                details.push("✓ Detected Rust project".into());
                if verbose && !check_cargo_has_dotenv(project_root)? {
                    details.push("· Consider adding 'dotenvy' crate".into());
                }
            } else {
                details.push(format!("✓ Detected {} project", project_type));
            }
            severity = Severity::Ok;
        } else {
            details.push("· No recognized project configuration".into());
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

/// Is every line of this file something `core::Parser` will accept?
///
/// ⚠️ **This must agree with the parser, and it did not.** The pattern omitted
/// the optional `export` prefix, so `export FOO=bar` — which the parser's own
/// documentation lists as supported and which it strips at `parser.rs:385` —
/// was reported as *"invalid syntax"*. The same file passed `evnx validate`
/// with zero errors. A user was told their `.env` was broken by one command and
/// fine by another.
///
/// The duplication is deliberate rather than accidental: the parser stops at its
/// first error, and `doctor` needs to report every bad line at once. What stops
/// the two drifting again is `doctor_agrees_with_the_parser` in
/// `tests/devrel_review.rs`, which runs both over the same corpus.
///
/// ⚠️ It also returned on the first bad line, so a file with three problems was
/// three `doctor --fix` runs to diagnose. All of them are reported now.
fn validate_env_syntax(path: &Path) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let line_re = Regex::new(r"^(?:\s*$|\s*#.*|\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=.*)$")
        .context("Invalid regex pattern")?;

    let bad: Vec<String> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line_re.is_match(line))
        .map(|(idx, line)| {
            let snippet = line.chars().take(50).collect::<String>();
            format!(
                "Line {}: invalid syntax '{}{}'",
                idx + 1,
                snippet,
                if line.chars().count() > 50 { "..." } else { "" }
            )
        })
        .collect();

    if bad.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(bad.join("; ")))
    }
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
    // ⚠️ `description`, not `name`. `name` is the struct key — `env_file`,
    // `project_structure`, `docker_config` — and printing it asked the reader to
    // know evnx's internals to read evnx's report. The human string was on the
    // same struct the whole time; only the JSON wants the key.
    println!(
        "  {}  {}",
        result.severity.colored_icon(),
        result.description.bold()
    );

    if verbose || result.severity != Severity::Ok {
        if let Some(ref details) = result.details {
            // Markers that repeat the check's own severity were already
            // dropped by `normalise_details`, so this only prints.
            for line in details.lines() {
                println!("     {}", line.dimmed());
            }
        }
    }
    println!();
}

fn print_summary_text(summary: &Summary) {
    // One line, only what is non-zero. A "Summary:" heading over two numbers and
    // an "Overall health:" line restating those same two numbers is three lines
    // of scaffolding around one fact.
    let mut parts = Vec::new();
    if summary.errors > 0 {
        parts.push(
            pluralize(summary.errors, "error", "errors")
                .red()
                .to_string(),
        );
    }
    if summary.warnings > 0 {
        parts.push(
            pluralize(summary.warnings, "warning", "warnings")
                .yellow()
                .to_string(),
        );
    }
    if summary.passed > 0 {
        parts.push(
            pluralize(summary.passed, "check passed", "checks passed")
                .green()
                .to_string(),
        );
    }

    if parts.is_empty() {
        println!("  {}  nothing to report", glyph::INFO.dimmed());
    } else {
        println!("  {}", parts.join("  ·  "));
    }
}

fn print_recommendations_text(checks: &[CheckResult], auto_fix_enabled: bool) {
    let fixable: Vec<_> = checks.iter().filter(|c| c.fixable && !c.fixed).collect();

    if !fixable.is_empty() {
        println!();
        if auto_fix_enabled {
            // `--fix` ran and these are what it could not repair.
            println!(
                "  {}  {}",
                glyph::WARN.yellow(),
                "--fix could not repair these:".bold()
            );
        } else {
            println!(
                "  {}  {} {}",
                glyph::ARROW.dimmed(),
                pluralize(fixable.len(), "issue", "issues").bold(),
                "evnx doctor --fix can repair:".bold()
            );
        }
        for check in fixable {
            // The description, again — not `env_file (Validate .env file …)`,
            // which put the struct key first and the readable half in brackets.
            println!("     {}  {}", glyph::INFO.dimmed(), check.description);
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
        assert!(gitignore::fallback_gitignore_check(dir.path(), ".env"));
        assert!(!gitignore::fallback_gitignore_check(
            dir.path(),
            ".env.local"
        ));
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

#[cfg(test)]
mod env_file_coverage_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn picks_up_every_secret_bearing_env_file() {
        let dir = TempDir::new().unwrap();
        for n in [
            ".env",
            ".env.local",
            ".env.production",
            ".env.staging",
            ".env.test",
            ".env.prod",
        ] {
            fs::write(dir.path().join(n), "A=1\n").unwrap();
        }

        let found = secret_bearing_env_files(dir.path());
        assert_eq!(found.len(), 6, "{found:?}");
        assert!(found.contains(&".env.production".to_string()));
        assert!(found.contains(&".env.prod".to_string()));
    }

    /// The template family is meant to be committed, so "not gitignored" is the
    /// correct state for it — flagging it would train people to ignore the check.
    #[test]
    fn leaves_out_the_template_family_and_backups() {
        let dir = TempDir::new().unwrap();
        for n in [
            ".env",
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.backup",
            ".env.production.backup",
        ] {
            fs::write(dir.path().join(n), "A=1\n").unwrap();
        }

        assert_eq!(
            secret_bearing_env_files(dir.path()),
            vec![".env".to_string()]
        );
    }

    #[test]
    fn ignores_directories_and_unrelated_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".envoy")).unwrap();
        fs::write(dir.path().join("README.md"), "x").unwrap();
        fs::write(dir.path().join(".env"), "A=1\n").unwrap();

        assert_eq!(
            secret_bearing_env_files(dir.path()),
            vec![".env".to_string()]
        );
    }
}

#[cfg(test)]
mod fallback_gitignore_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn with_gitignore(body: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), body).unwrap();
        dir
    }

    #[test]
    fn understands_the_pattern_evnx_writes() {
        let dir = with_gitignore(".env*\n!.env.example\n!.env.sample\n!.env.template\n");

        for ignored in [".env", ".env.local", ".env.production", ".env.prod"] {
            assert!(
                gitignore::fallback_gitignore_check(dir.path(), ignored),
                "{ignored} should be ignored"
            );
        }
        for committable in [".env.example", ".env.sample", ".env.template"] {
            assert!(
                !gitignore::fallback_gitignore_check(dir.path(), committable),
                "{committable} must stay committable"
            );
        }
    }

    #[test]
    fn still_handles_plain_and_rooted_names() {
        let dir = with_gitignore("node_modules/\n.env\n/secrets.txt\n");
        assert!(gitignore::fallback_gitignore_check(dir.path(), ".env"));
        assert!(gitignore::fallback_gitignore_check(
            dir.path(),
            "secrets.txt"
        ));
        assert!(!gitignore::fallback_gitignore_check(
            dir.path(),
            ".env.production"
        ));
    }

    /// Last match wins, so a negation cannot be undone by an earlier rule but
    /// can be by a later one.
    #[test]
    fn later_rules_win() {
        let dir = with_gitignore("!.env.example\n.env*\n");
        assert!(
            gitignore::fallback_gitignore_check(dir.path(), ".env.example"),
            "a negation before the pattern does not survive it"
        );
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let dir = with_gitignore("# .env\n\n   \n");
        assert!(
            !gitignore::fallback_gitignore_check(dir.path(), ".env"),
            "a commented mention protects nothing"
        );
    }
}
