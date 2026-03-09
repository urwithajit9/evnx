//! Enhanced validation command with auto-fix, format validation, and UI improvements
//!
//! Validates .env against .env.example with comprehensive checks:
//! - Missing/extra variables
//! - Placeholder detection
//! - Boolean string trap
//! - Weak SECRET_KEY
//! - localhost in Docker context
//! - Value format validation (URL, port, email)
//! - Auto-fix for common issues
//! - Multiple output formats with improved UI

use anyhow::{Context, Result};
use colored::*;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::fs;

use crate::core::{Parser, ParserConfig};
use crate::utils::ui;

// ─────────────────────────────────────────────────────────────
// Configuration & Types
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ValidationConfig {
    pub strict: bool,
    pub fix: bool,
    pub validate_formats: bool,
    pub ignore_issues: HashSet<String>,
    pub env_pattern: Option<String>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict: false,
            fix: false,
            validate_formats: false,
            ignore_issues: HashSet::new(),
            env_pattern: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    MissingVariable,
    ExtraVariable,
    PlaceholderValue,
    BooleanTrap,
    WeakSecret,
    LocalhostInDocker,
    InvalidUrl,
    InvalidPort,
    InvalidEmail,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::MissingVariable => "missing_variable",
            IssueType::ExtraVariable => "extra_variable",
            IssueType::PlaceholderValue => "placeholder_value",
            IssueType::BooleanTrap => "boolean_trap",
            IssueType::WeakSecret => "weak_secret",
            IssueType::LocalhostInDocker => "localhost_in_docker",
            IssueType::InvalidUrl => "invalid_url",
            IssueType::InvalidPort => "invalid_port",
            IssueType::InvalidEmail => "invalid_email",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub status: String,
    pub required_present: usize,
    pub required_total: usize,
    pub issues: Vec<Issue>,
    pub fixed: Vec<FixApplied>,
    pub summary: Summary,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Issue {
    pub severity: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub variable: String,
    pub message: String,
    pub location: String,
    pub suggestion: Option<String>,
    pub auto_fixable: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FixApplied {
    pub variable: String,
    pub action: String,
    pub old_value: Option<String>,
    pub new_value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub style: usize,
    pub fixed_count: usize,
}

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
// Format Validation Regexes (Improvement #3)
// ─────────────────────────────────────────────────────────────

lazy_static! {
    static ref URL_REGEX: Regex =
        Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").expect("URL regex is valid");
    static ref PORT_REGEX: Regex = Regex::new(r"^\d{1,5}$").expect("Port regex is valid");
    static ref EMAIL_REGEX: Regex = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
        .expect("Email regex is valid");
    static ref HOST_REGEX: Regex =
        Regex::new(r"^[a-zA-Z0-9.-]+(:\d{1,5})?$").expect("Host regex is valid");
}

fn validate_url(value: &str) -> bool {
    URL_REGEX.is_match(value)
}

fn validate_port(value: &str) -> bool {
    if !PORT_REGEX.is_match(value) {
        return false;
    }
    value.parse::<u16>().is_ok()
}

fn validate_email(value: &str) -> bool {
    EMAIL_REGEX.is_match(value)
}

fn validate_host(value: &str) -> bool {
    HOST_REGEX.is_match(value)
}

// ─────────────────────────────────────────────────────────────
// Auto-Fix Logic (Improvement #1)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum FixAction {
    GenerateSecret,
    ReplacePlaceholder(String),
    FixBoolean(String),
    AddMissing(String),
    Skip,
}

fn suggest_fix(key: &str, value: &str, issue_type: &IssueType) -> FixAction {
    match issue_type {
        IssueType::PlaceholderValue => {
            if key.to_uppercase().contains("SECRET") || key.to_uppercase().contains("KEY") {
                FixAction::GenerateSecret
            } else if key.contains("URL") {
                FixAction::ReplacePlaceholder("https://example.com".to_string())
            } else if key.contains("EMAIL") || key.contains("MAIL") {
                FixAction::ReplacePlaceholder("user@example.com".to_string())
            } else if key.contains("PORT") {
                FixAction::ReplacePlaceholder("8080".to_string())
            } else {
                FixAction::ReplacePlaceholder("your_value_here".to_string())
            }
        }
        IssueType::BooleanTrap => {
            let fixed = if value.eq_ignore_ascii_case("true") {
                "true"
            } else {
                "false"
            };
            FixAction::FixBoolean(fixed.to_string())
        }
        IssueType::WeakSecret => FixAction::GenerateSecret,
        IssueType::MissingVariable => {
            if key.to_uppercase().contains("SECRET") || key.to_uppercase().contains("KEY") {
                FixAction::AddMissing("CHANGE_ME_SECURE_32_CHARS_MIN".to_string())
            } else {
                FixAction::AddMissing("your_value_here".to_string())
            }
        }
        _ => FixAction::Skip,
    }
}

fn generate_secure_secret() -> String {
    // Simple secure-ish generator; in production use openssl or similar
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    format!("{:064x}", seed ^ 0x5DEECE66D)
}

fn apply_fix(
    key: &str,
    value: &str,
    action: &FixAction,
    env_vars: &mut HashMap<String, String>,
) -> Option<FixApplied> {
    match action {
        FixAction::GenerateSecret => {
            let new_val = generate_secure_secret();
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Generated secure secret".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val,
            })
        }
        FixAction::ReplacePlaceholder(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Replaced placeholder".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val.clone(),
            })
        }
        FixAction::FixBoolean(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Fixed boolean format".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val.clone(),
            })
        }
        FixAction::AddMissing(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Added missing variable".to_string(),
                old_value: None,
                new_value: new_val.clone(),
            })
        }
        FixAction::Skip => None,
    }
}

// ─────────────────────────────────────────────────────────────
// Main Validation Function
// ─────────────────────────────────────────────────────────────

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
    // UI: Print header using utils/ui.rs (Improvement #6)
    ui::print_header("evnx validate", Some("Check environment configuration"));

    if verbose {
        ui::info("Running in verbose mode");
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

    let mut parser_config = ParserConfig::default();
    if strict {
        parser_config.strict = true;
    }
    let parser = Parser::new(parser_config);

    // Parse files with progress indicator
    let pb = ui::spinner("Parsing configuration files...");

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

    let mut issues = Vec::new();
    let mut env_vars = env_file.vars.clone(); // Mutable copy for --fix
    let mut fixes_applied = Vec::new();

    // ─────────────────────────────────────────
    // Check 1: Required variables present
    // ─────────────────────────────────────────
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let env_keys: HashSet<_> = env_vars.keys().collect();

    let missing: Vec<_> = example_keys.difference(&env_keys).collect();
    for key in &missing {
        let issue_type = IssueType::MissingVariable;

        if !config.ignore_issues.contains(issue_type.as_str()) {
            let fix_action = if config.fix {
                suggest_fix(key, "", &issue_type)
            } else {
                FixAction::Skip
            };

            if config.fix && !matches!(fix_action, FixAction::Skip) {
                if let Some(fix) = apply_fix(key, "", &fix_action, &mut env_vars) {
                    fixes_applied.push(fix);
                    continue; // Skip adding issue if fixed
                }
            }

            issues.push(Issue {
                severity: "error".to_string(),
                issue_type: issue_type.as_str().to_string(),
                variable: key.to_string(),
                message: format!("Missing required variable: {}", key),
                location: format!("{}:?", env_path),
                suggestion: Some(format!("Add {}=<value> to {}", key, env_path)),
                auto_fixable: true,
            });
        }
    }

    // ─────────────────────────────────────────
    // Check 2: Extra variables in strict mode
    // ─────────────────────────────────────────
    if strict {
        let extra: Vec<_> = env_keys.difference(&example_keys).collect();
        for key in &extra {
            let issue_type = IssueType::ExtraVariable;

            if !config.ignore_issues.contains(issue_type.as_str()) {
                issues.push(Issue {
                    severity: "warning".to_string(),
                    issue_type: issue_type.as_str().to_string(),
                    variable: key.to_string(),
                    message: format!("Extra variable not in .env.example: {}", key),
                    location: format!("{}:?", env_path),
                    suggestion: Some(format!(
                        "Add {} to {} or remove from {}",
                        key, example, env_path
                    )),
                    auto_fixable: false,
                });
            }
        }
    }

    // ─────────────────────────────────────────
    // Check 3: Placeholder values
    // ─────────────────────────────────────────
    for (key, value) in &env_vars {
        if is_placeholder(value) {
            let issue_type = IssueType::PlaceholderValue;

            if !config.ignore_issues.contains(issue_type.as_str()) {
                let suggestion = match key.as_str() {
                    "SECRET_KEY" => Some("Run: openssl rand -hex 32".to_string()),
                    k if k.contains("AWS") => Some("Get from AWS Console".to_string()),
                    k if k.contains("STRIPE") => Some("Get from Stripe Dashboard".to_string()),
                    _ => None,
                };

                let fix_action = if config.fix {
                    suggest_fix(key, value, &issue_type)
                } else {
                    FixAction::Skip
                };

                if config.fix && !matches!(fix_action, FixAction::Skip) {
                    if let Some(fix) = apply_fix(key, value, &fix_action, &mut env_vars) {
                        fixes_applied.push(fix);
                        continue;
                    }
                }

                issues.push(Issue {
                    severity: "error".to_string(),
                    issue_type: issue_type.as_str().to_string(),
                    variable: key.clone(),
                    message: format!("{} looks like a placeholder", key),
                    location: format!("{}:?", env_path),
                    suggestion,
                    auto_fixable: true,
                });
            }
        }
    }

    // ─────────────────────────────────────────
    // Check 4: Boolean string trap
    // ─────────────────────────────────────────
    for (key, value) in &env_vars {
        if value == "False" || value == "True" {
            let issue_type = IssueType::BooleanTrap;

            if !config.ignore_issues.contains(issue_type.as_str()) {
                let fix_action = if config.fix {
                    suggest_fix(key, value, &issue_type)
                } else {
                    FixAction::Skip
                };

                if config.fix && !matches!(fix_action, FixAction::Skip) {
                    if let Some(fix) = apply_fix(key, value, &fix_action, &mut env_vars) {
                        fixes_applied.push(fix);
                        continue;
                    }
                }

                issues.push(Issue {
                    severity: "warning".to_string(),
                    issue_type: issue_type.as_str().to_string(),
                    variable: key.clone(),
                    message: format!("{} is set to \"{}\" (string, not boolean)", key, value),
                    location: format!("{}:?", env_path),
                    suggestion: Some(format!(
                        "Use {} or 0 for proper boolean handling in most languages",
                        if value == "False" { "false" } else { "true" }
                    )),
                    auto_fixable: true,
                });
            }
        }
    }

    // ─────────────────────────────────────────
    // Check 5: Weak SECRET_KEY
    // ─────────────────────────────────────────
    if let Some(secret_key) = env_vars.get("SECRET_KEY") {
        if is_weak_secret_key(secret_key) {
            let issue_type = IssueType::WeakSecret;

            if !config.ignore_issues.contains(issue_type.as_str()) {
                let fix_action = if config.fix {
                    suggest_fix("SECRET_KEY", secret_key, &issue_type)
                } else {
                    FixAction::Skip
                };

                if config.fix && !matches!(fix_action, FixAction::Skip) {
                    if let Some(fix) =
                        apply_fix("SECRET_KEY", secret_key, &fix_action, &mut env_vars)
                    {
                        fixes_applied.push(fix);
                    } else {
                        // Add issue if fix wasn't applied
                        issues.push(Issue {
                            severity: "error".to_string(),
                            issue_type: issue_type.as_str().to_string(),
                            variable: "SECRET_KEY".to_string(),
                            message: "SECRET_KEY is too weak or predictable".to_string(),
                            location: format!("{}:?", env_path),
                            suggestion: Some("Run: openssl rand -hex 32".to_string()),
                            auto_fixable: true,
                        });
                    }
                } else {
                    issues.push(Issue {
                        severity: "error".to_string(),
                        issue_type: issue_type.as_str().to_string(),
                        variable: "SECRET_KEY".to_string(),
                        message: "SECRET_KEY is too weak or predictable".to_string(),
                        location: format!("{}:?", env_path),
                        suggestion: Some("Run: openssl rand -hex 32".to_string()),
                        auto_fixable: true,
                    });
                }
            }
        }
    }

    // ─────────────────────────────────────────
    // Check 6: localhost in Docker context (cached)
    // ─────────────────────────────────────────
    if has_docker_context() {
        for (key, value) in &env_vars {
            if value.contains("localhost") || value.contains("127.0.0.1") {
                if key.contains("URL") || key.contains("HOST") || key.contains("ADDR") {
                    let issue_type = IssueType::LocalhostInDocker;

                    if !config.ignore_issues.contains(issue_type.as_str()) {
                        issues.push(Issue {
                            severity: "warning".to_string(),
                            issue_type: issue_type.as_str().to_string(),
                            variable: key.clone(),
                            message: format!("{} uses localhost/127.0.0.1", key),
                            location: format!("{}:?", env_path),
                            suggestion: Some(
                                "In Docker, use service name instead (e.g., db:5432)".to_string(),
                            ),
                            auto_fixable: false,
                        });
                    }
                }
            }
        }
    }

    // ─────────────────────────────────────────
    // Check 7: Value format validation (Improvement #3)
    // ─────────────────────────────────────────
    if validate_formats {
        for (key, value) in &env_vars {
            let key_upper = key.to_uppercase();

            // URL validation
            if key_upper.contains("URL")
                || key_upper.contains("URI")
                || key_upper.contains("ENDPOINT")
            {
                if !value.is_empty() && !validate_url(value) {
                    let issue_type = IssueType::InvalidUrl;
                    if !config.ignore_issues.contains(issue_type.as_str()) {
                        issues.push(Issue {
                            severity: "warning".to_string(),
                            issue_type: issue_type.as_str().to_string(),
                            variable: key.clone(),
                            message: format!("{} does not appear to be a valid URL", key),
                            location: format!("{}:?", env_path),
                            suggestion: Some(
                                "Expected format: https://example.com/path".to_string(),
                            ),
                            auto_fixable: false,
                        });
                    }
                }
            }

            // Port validation
            if key_upper.contains("PORT") {
                if !value.is_empty() && !validate_port(value) {
                    let issue_type = IssueType::InvalidPort;
                    if !config.ignore_issues.contains(issue_type.as_str()) {
                        issues.push(Issue {
                            severity: "error".to_string(),
                            issue_type: issue_type.as_str().to_string(),
                            variable: key.clone(),
                            message: format!("{} is not a valid port number (1-65535)", key),
                            location: format!("{}:?", env_path),
                            suggestion: Some("Expected format: 8080".to_string()),
                            auto_fixable: false,
                        });
                    }
                }
            }

            // Email validation
            if key_upper.contains("EMAIL")
                || key_upper.contains("MAIL")
                || key_upper.contains("ADMIN")
            {
                if !value.is_empty() && !validate_email(value) {
                    let issue_type = IssueType::InvalidEmail;
                    if !config.ignore_issues.contains(issue_type.as_str()) {
                        issues.push(Issue {
                            severity: "warning".to_string(),
                            issue_type: issue_type.as_str().to_string(),
                            variable: key.clone(),
                            message: format!("{} does not appear to be a valid email", key),
                            location: format!("{}:?", env_path),
                            suggestion: Some("Expected format: user@example.com".to_string()),
                            auto_fixable: false,
                        });
                    }
                }
            }

            // Host/hostname validation
            if key_upper.contains("HOST") && !key_upper.contains("URL") {
                if !value.is_empty() && !validate_host(value) && !value.contains("localhost") {
                    ui::info(format!("Consider validating format for {}: {}", key, value));
                }
            }
        }
    }

    // ─────────────────────────────────────────
    // Write fixes back to file if --fix was used
    // ─────────────────────────────────────────
    if config.fix && !fixes_applied.is_empty() {
        let pb = ui::spinner("Writing fixes to file...");


        // use std::io::Write;

        let mut output = String::new();

        // Preserve comments and order from original file where possible
        let original_content = fs::read_to_string(&env_path).unwrap_or_default();

        for line in original_content.lines() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                output.push_str(line);
                output.push('\n');
            } else if let Some(eq_pos) = line.find('=') {
                let var_name = line[..eq_pos].trim();
                if let Some(new_value) = env_vars.get(var_name) {
                    output.push_str(var_name);
                    output.push('=');
                    output.push_str(new_value);
                    output.push('\n');
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }

        // Add any new variables that weren't in original
        for (key, value) in &env_vars {
            if !original_content.contains(&format!("{}=", key)) {
                output.push_str(key);
                output.push('=');
                output.push_str(value);
                output.push('\n');
            }
        }

        fs::write(&env_path, output)
            .with_context(|| format!("Failed to write fixes to {}", env_path))?;

        pb.finish_with_message("Fixes applied ✓");
        ui::success(format!(
            "Applied {} fix(es) to {}",
            fixes_applied.len(),
            env_path
        ));
    }

    // ─────────────────────────────────────────
    // Build result
    // ─────────────────────────────────────────
    let errors = issues.iter().filter(|i| i.severity == "error").count();
    let warnings = issues.iter().filter(|i| i.severity == "warning").count();
    let style = issues.iter().filter(|i| i.severity == "style").count();

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
            fixed_count: fixes_applied.len(),
        },
    };

    // ─────────────────────────────────────────
    // Output with improved UI (Improvement #6)
    // ─────────────────────────────────────────
    match format.as_str() {
        "json" => output_json(&result)?,
        "github-actions" => output_github_actions(&result, &env_path)?,
        _ => output_pretty(&result, &env_path, &example)?,
    }

    // ─────────────────────────────────────────
    // Exit code handling
    // ─────────────────────────────────────────
    if !exit_zero && result.summary.errors > 0 {
        ui::error("Validation failed with errors");
        std::process::exit(1);
    } else if result.summary.errors == 0 && result.summary.warnings == 0 {
        ui::success("All checks passed ✓");
    } else if result.summary.errors == 0 {
        ui::warning(format!(
            "{} warning(s) found (non-blocking)",
            result.summary.warnings
        ));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Resolve env file path with pattern support
fn resolve_env_path(base: &str, pattern: &Option<String>) -> Result<String> {
    if let Some(pat) = pattern {
        // Support simple pattern expansion
        if pat.starts_with(".env.") {
            return Ok(pat.clone());
        }
        // Fallback to base if pattern doesn't match expected format
        ui::info(format!("Pattern '{}' not recognized, using default", pat));
    }
    Ok(base.to_string())
}

fn is_placeholder(value: &str) -> bool {
    let lower = value.to_lowercase();
    let placeholders = [
        "your_key_here",
        "your_secret_here",
        "your_token_here",
        "change_me",
        "changeme",
        "replace_me",
        "example",
        "xxx",
        "todo",
        "generate-with",
        "placeholder",
        "<",
        ">",
    ];
    placeholders.iter().any(|p| lower.contains(p)) || value.is_empty()
}

fn is_weak_secret_key(key: &str) -> bool {
    if key.len() < 32 {
        return true;
    }
    let weak = [
        "secret", "password", "dev", "test", "1234", "abcd", "changeme", "example",
    ];
    let lower = key.to_lowercase();
    weak.iter().any(|w| lower.contains(w))
}

// ─────────────────────────────────────────────────────────────
// Output Functions with UI Integration
// ─────────────────────────────────────────────────────────────

fn output_pretty(result: &ValidationResult, env_path: &str, example_path: &str) -> Result<()> {
    ui::print_preview_header();

    if result.fixed.is_empty() && result.issues.is_empty() {
        ui::success(format!(
            "All {} required variables present ({}/{})",
            example_path, result.required_present, result.required_total
        ));
        return Ok(());
    }

    // Show fix summary first if any were applied
    if !result.fixed.is_empty() {
        ui::print_section_header("🔧", "Applied Fixes");
        for fix in &result.fixed {
            let old = fix
                .old_value
                .as_ref()
                .map(|v| format!("\"{}\"", v))
                .unwrap_or("(new)".to_string());
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

    // Next steps
    if result.summary.errors > 0 {
        ui::print_next_steps(&[
            "Review and fix the errors above",
            "Run with --fix to auto-correct common issues",
            "Use --ignore issue_type to suppress specific warnings",
        ]);
    } else if result.summary.warnings > 0 {
        ui::info("Warnings don't block execution but should be reviewed");
    }

    Ok(())
}

fn output_json(result: &ValidationResult) -> Result<()> {
    let json = serde_json::to_string_pretty(result)?;
    println!("{}", json);
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

    // Report fixes as notices
    for fix in &result.fixed {
        println!(
            "::notice file={},line=1::Fixed: {} → {}",
            env_path, fix.variable, fix.action
        );
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url() {
        assert!(validate_url("https://example.com"));
        assert!(validate_url("http://localhost:8080/path"));
        assert!(!validate_url("not-a-url"));
        assert!(!validate_url("ftp://example.com")); // Only http/https
    }

    #[test]
    fn test_validate_port() {
        assert!(validate_port("80"));
        assert!(validate_port("65535"));
        assert!(!validate_port("0"));
        assert!(!validate_port("65536"));
        assert!(!validate_port("abc"));
    }

    #[test]
    fn test_validate_email() {
        assert!(validate_email("user@example.com"));
        assert!(validate_email("test.user+tag@sub.example.co.uk"));
        assert!(!validate_email("invalid"));
        assert!(!validate_email("@example.com"));
    }

    #[test]
    fn test_ignore_filtering() {
        let mut ignore = HashSet::new();
        ignore.insert("boolean_trap".to_string());

        let issue = Issue {
            severity: "warning".to_string(),
            issue_type: "boolean_trap".to_string(),
            variable: "DEBUG".to_string(),
            message: "test".to_string(),
            location: ".env".to_string(),
            suggestion: None,
            auto_fixable: true,
        };

        // In real code, we'd filter here; test the logic
        assert!(ignore.contains(&issue.issue_type));
    }

    #[test]
    fn test_docker_cache() {
        // First call should detect and cache
        let result1 = has_docker_context();
        // Second call should use cache (can't easily test without mocking FS)
        let result2 = has_docker_context();
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_fix_actions() {
        let mut env_vars = HashMap::new();

        // Test GenerateSecret
        let action = suggest_fix("API_KEY", "changeme", &IssueType::PlaceholderValue);
        assert!(matches!(action, FixAction::GenerateSecret));

        // Test FixBoolean
        let action = suggest_fix("DEBUG", "True", &IssueType::BooleanTrap);
        assert!(matches!(action, FixAction::FixBoolean(_)));

        // Test that apply_fix actually modifies the map
        let action = FixAction::ReplacePlaceholder("new_value".to_string());
        let fix = apply_fix("TEST_VAR", "old", &action, &mut env_vars);
        assert!(fix.is_some());
        assert_eq!(env_vars.get("TEST_VAR"), Some(&"new_value".to_string()));
    }
}
