//! Validate command: Check .env against .env.example
//!
//! # Module Structure
//! - `types.rs`: Shared data structures
//! - `checks.rs`: Pure validation functions (testable)
//! - `fixer.rs`: Auto-fix logic and file I/O
//! - `mod.rs`: Orchestration, CLI integration, output (this file)

pub mod checks;
pub mod fixer;
pub mod types;

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use colored::Colorize;
use lazy_static::lazy_static;
use serde_json;

use crate::core::{Parser, ParserConfig};
use crate::utils::ui;

use self::checks::*;
use self::fixer::*;
// use self::types::*;

// ─────────────────────────────────────────────────────────────
// Cached Docker Detection (Improvement #5)
// ─────────────────────────────────────────────────────────────

lazy_static! {
    static ref DOCKER_CONTEXT_CACHE: Mutex<Option<bool>> = Mutex::new(None);
}

/// Check if running in Docker context (cached to avoid repeated FS checks)
fn has_docker_context() -> bool {
    let mut cache = DOCKER_CONTEXT_CACHE.lock().unwrap();

    if let Some(cached) = *cache {
        return cached;
    }

    let result = Path::new("docker-compose.yml").exists()
        || Path::new("docker-compose.yaml").exists()
        || Path::new("Dockerfile").exists()
        || Path::new("Containerfile").exists()
        || std::env::var_os("DOCKER_HOST").is_some();

    *cache = Some(result);
    result
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point
// ─────────────────────────────────────────────────────────────

/// Run the validate command with the given configuration.
///
/// This function is called from `main.rs` after CLI args are parsed.
#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    example: String,
    strict: bool,
    fix: bool,
    format: String,
    exit_zero: bool,
    verbose: bool,
    ignore: Vec<String>,
    validate_formats: bool,
    pattern: Option<String>,
) -> Result<()> {
    // ─────────────────────────────────────────
    // UI: Header (only for pretty output)
    // ─────────────────────────────────────────
    if format == "pretty" {
        ui::print_header("evnx validate", Some("Check environment configuration"));
        if verbose {
            ui::info("Running in verbose mode");
        }
    } else if verbose {
        // Verbose for machine formats goes to stderr
        eprintln!("[verbose] validate: env={}, example={}", env, example);
    }

    // Resolve env file path with pattern support (Improvement #2)
    let env_path = resolve_env_path(&env, &pattern)?;

    // Build configuration
    let config = ValidationConfig {
        strict,
        fix,
        validate_formats,
        ignore_issues: ignore.into_iter().collect(),
        env_pattern: pattern,
    };

    // Parse files with progress indicator
    let pb = ui::spinner("Parsing configuration files...");
    // let parser = Parser::new(ParserConfig { strict });
    let parser = Parser::new(ParserConfig {
        strict,
        ..ParserConfig::default()
    });

    let example_file = parser
        .parse_file(&example)
        .with_context(|| format!("Failed to parse {}", example))?;
    let env_file = parser
        .parse_file(&env_path)
        .with_context(|| format!("Failed to parse {}", env_path))?;

    pb.finish_with_message("Files parsed ✓");

    if verbose {
        ui::info(format!(
            "Loaded {} variables from {} and {} from {}",
            example_file.vars.len(),
            example,
            env_file.vars.len(),
            env_path
        ));
    }

    // ─────────────────────────────────────────
    // Run All Validation Checks (pure functions)
    // ─────────────────────────────────────────
    let mut issues = Vec::new();

    issues.extend(check_missing_variables(
        &env_file.vars,
        &example_file.vars,
        &env_path,
        &config.ignore_issues,
    ));

    issues.extend(check_extra_variables(
        &env_file.vars,
        &example_file.vars,
        &env_path,
        config.strict,
        &config.ignore_issues,
    ));

    issues.extend(check_placeholders(
        &env_file.vars,
        &env_path,
        &config.ignore_issues,
    ));

    issues.extend(check_boolean_trap(
        &env_file.vars,
        &env_path,
        &config.ignore_issues,
    ));

    issues.extend(check_weak_secret(
        &env_file.vars,
        &env_path,
        &config.ignore_issues,
    ));

    issues.extend(check_localhost_docker(
        &env_file.vars,
        &env_path,
        has_docker_context(),
        &config.ignore_issues,
    ));

    issues.extend(check_formats(
        &env_file.vars,
        &env_path,
        config.validate_formats,
        &config.ignore_issues,
    ));

    // ─────────────────────────────────────────
    // Apply Fixes if Requested (Improvement #1)
    // ─────────────────────────────────────────
    let mut fixes_applied = Vec::new();
    let mut env_vars = env_file.vars.clone();

    if config.fix && !issues.is_empty() {
        let pb = ui::spinner("Applying auto-fixes...");

        let mut fixes_to_apply = Vec::new();

        // Missing variables
        for issue in &issues {
            if issue.issue_type == IssueType::MissingVariable.as_str() && issue.auto_fixable {
                let action = suggest_fix(&issue.variable, "", &IssueType::MissingVariable);
                if !matches!(action, FixAction::Skip) {
                    fixes_to_apply.push((issue.variable.clone(), String::new(), action));
                }
            }
        }

        // Placeholders
        for issue in &issues {
            if issue.issue_type == IssueType::PlaceholderValue.as_str() && issue.auto_fixable {
                if let Some(val) = env_vars.get(&issue.variable) {
                    let action = suggest_fix(&issue.variable, val, &IssueType::PlaceholderValue);
                    if !matches!(action, FixAction::Skip) {
                        fixes_to_apply.push((issue.variable.clone(), val.clone(), action));
                    }
                }
            }
        }

        // Boolean traps
        for issue in &issues {
            if issue.issue_type == IssueType::BooleanTrap.as_str() && issue.auto_fixable {
                if let Some(val) = env_vars.get(&issue.variable) {
                    let action = suggest_fix(&issue.variable, val, &IssueType::BooleanTrap);
                    if !matches!(action, FixAction::Skip) {
                        fixes_to_apply.push((issue.variable.clone(), val.clone(), action));
                    }
                }
            }
        }

        // Weak secret
        if let Some(issue) = issues
            .iter()
            .find(|i| i.issue_type == IssueType::WeakSecret.as_str())
        {
            if issue.auto_fixable {
                if let Some(val) = env_vars.get("SECRET_KEY") {
                    let action = suggest_fix("SECRET_KEY", val, &IssueType::WeakSecret);
                    if !matches!(action, FixAction::Skip) {
                        fixes_to_apply.push(("SECRET_KEY".to_string(), val.clone(), action));
                    }
                }
            }
        }

        // Apply all collected fixes
        for (key, old_val, action) in fixes_to_apply {
            if let Some(fix) = apply_fix(&key, &old_val, &action, &mut env_vars) {
                fixes_applied.push(fix);
            }
        }

        pb.finish_with_message(format!("Applied {} fix(es) ✓", fixes_applied.len()));

        // Write to file if any fixes were applied
        if !fixes_applied.is_empty() {
            let original = std::fs::read_to_string(&env_path).unwrap_or_default();
            write_fixed_file(&env_path, &env_vars, &original)?;
            ui::success(format!("Saved fixes to {}", env_path));
        }
    }

    // ─────────────────────────────────────────
    // Build Result
    // ─────────────────────────────────────────
    let errors = issues.iter().filter(|i| i.severity == "error").count();
    let warnings = issues.iter().filter(|i| i.severity == "warning").count();
    let style = issues.iter().filter(|i| i.severity == "style").count();
    let fixed_count = fixes_applied.len();

    let result = ValidationResult {
        status: if errors > 0 {
            "failed".to_string()
        } else {
            "passed".to_string()
        },
        required_present: env_vars.len().min(example_file.vars.len()),
        required_total: example_file.vars.len(),
        issues,
        fixed: fixes_applied,
        summary: Summary {
            errors,
            warnings,
            style,
            fixed_count,
        },
    };

    // ─────────────────────────────────────────
    // Output (Improvement #6: UI integration)
    // ─────────────────────────────────────────
    match format.as_str() {
        "json" => output_json(&result)?,
        "github-actions" => output_github_actions(&result, &env_path)?,
        _ => output_pretty(&result, &env_path, &example)?,
    }

    // ─────────────────────────────────────────
    // Exit Code Handling
    // ─────────────────────────────────────────
    if !exit_zero && result.summary.errors > 0 {
        if format == "pretty" {
            ui::error("Validation failed with errors");
        } else {
            eprintln!("Validation failed: {} error(s)", result.summary.errors);
        }
        std::process::exit(1);
    } else if result.summary.errors == 0 && result.summary.warnings == 0 {
        if format == "pretty" {
            ui::success("All checks passed ✓");
        }
        // Silent for machine formats
    } else if result.summary.errors == 0 && format == "pretty" {
        ui::warning(format!("{} warning(s) found", result.summary.warnings));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Resolve env file path with pattern support (.env.local, .env.production, etc.)
fn resolve_env_path(base: &str, pattern: &Option<String>) -> Result<String> {
    if let Some(pat) = pattern {
        if pat.starts_with(".env.") {
            return Ok(pat.clone());
        }
        ui::info(format!("Pattern '{}' not recognized, using default", pat));
    }
    Ok(base.to_string())
}

// ─────────────────────────────────────────────────────────────
// Output Functions (UI Integration)
// ─────────────────────────────────────────────────────────────

fn output_pretty(result: &ValidationResult, _env_path: &str, _example_path: &str) -> Result<()> {
    ui::print_preview_header();

    if result.fixed.is_empty() && result.issues.is_empty() {
        ui::success("All required variables present ✓");
        return Ok(());
    }

    // Show applied fixes first
    if !result.fixed.is_empty() {
        ui::print_section_header("🔧", "Applied Fixes");
        for fix in &result.fixed {
            let old = fix
                .old_value
                .as_ref()
                .map(|v| format!("\"{}\"", v))
                .unwrap_or_else(|| "(new)".to_string());
            println!(
                "  • {}: {} → {}",
                fix.variable.bold(),
                old.dimmed(),
                fix.new_value.green()
            );
        }
        println!();
    }

    // Show issues
    if !result.issues.is_empty() {
        ui::print_section_header("⚠️", "Issues Found");

        for (i, issue) in result.issues.iter().enumerate() {
            let icon = match issue.severity.as_str() {
                "error" => "🚨",
                "warning" => "⚠️",
                _ => "ℹ️",
            };

            println!(
                "  {}. {} {}",
                (i + 1).to_string().bold(),
                icon,
                issue.message
            );

            if let Some(suggestion) = &issue.suggestion {
                println!("     {} {}", "→".dimmed(), suggestion.dimmed());
            }
            if issue.auto_fixable {
                println!(
                    "     {} {}",
                    "💡".dimmed(),
                    "Auto-fixable with --fix".dimmed()
                );
            }
            println!("     {} {}", "📍".dimmed(), issue.location.dimmed());
            println!();
        }
    }

    // Summary box
    ui::print_box(
        "Summary",
        &format!(
            "Errors: {}  |  Warnings: {}  |  Fixed: {}",
            result.summary.errors.to_string().red(),
            result.summary.warnings.to_string().yellow(),
            result.summary.fixed_count.to_string().green()
        ),
    );

    // Next steps if there are errors
    if result.summary.errors > 0 {
        ui::print_next_steps(&[
            "Review and fix the errors above",
            "Run with --fix to auto-correct common issues",
            "Use --ignore issue_type to suppress specific warnings",
        ]);
    }

    Ok(())
}

fn output_json(result: &ValidationResult) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(result)?);
    Ok(())
}

fn output_github_actions(result: &ValidationResult, env_path: &str) -> Result<()> {
    for issue in &result.issues {
        let level = match issue.severity.as_str() {
            "error" => "error",
            "warning" => "warning",
            _ => "notice",
        };
        println!("::{} file={},line=1::{}", level, env_path, issue.message);
        if let Some(suggestion) = &issue.suggestion {
            println!(
                "::{} file={},line=1::Suggestion: {}",
                level, env_path, suggestion
            );
        }
    }
    for fix in &result.fixed {
        println!(
            "::notice file={},line=1::Fixed: {} → {}",
            env_path, fix.variable, fix.action
        );
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Public Re-exports
// ─────────────────────────────────────────────────────────────

pub use types::{FixApplied, Issue, IssueType, Summary, ValidationConfig, ValidationResult};
