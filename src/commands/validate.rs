/// Enhanced validation command
///
/// Validates .env against .env.example with comprehensive checks:
/// - Missing/extra variables
/// - Placeholder detection
/// - Boolean string trap
/// - Weak SECRET_KEY
/// - localhost in Docker context
/// - Multiple output formats (pretty, json, github-actions)
use anyhow::{Context, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::core::{Parser, ParserConfig};

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub status: String,
    pub required_present: usize,
    pub required_total: usize,
    pub issues: Vec<Issue>,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub style: usize,
}

pub fn run(
    env: String,
    example: String,
    strict: bool,
    fix: bool,
    format: String,
    exit_zero: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running validate in verbose mode".dimmed());
    }

    if format == "pretty" {
        println!(
            "\n{}",
            "┌─ Validating environment variables ──────────────────┐".cyan()
        );
        println!(
            "{}",
            "│ Comparing .env against .env.example                 │".cyan()
        );
        println!(
            "{}\n",
            "└──────────────────────────────────────────────────────┘".cyan()
        );
    }

    let mut parser_config = ParserConfig::default();
    if strict {
        parser_config.strict = true;
    }
    let parser = Parser::new(parser_config);

    let example_file = parser
        .parse_file(&example)
        .with_context(|| format!("Failed to parse {}", example))?;

    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse {}", env))?;

    if verbose {
        println!(
            "Parsed {} variables from {}",
            example_file.vars.len(),
            example
        );
        println!("Parsed {} variables from {}", env_file.vars.len(), env);
    }

    let mut issues = Vec::new();

    // Check 1: Required variables present
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let env_keys: HashSet<_> = env_file.vars.keys().collect();

    let missing: Vec<_> = example_keys.difference(&env_keys).collect();
    for key in &missing {
        issues.push(Issue {
            severity: "error".to_string(),
            issue_type: "missing_variable".to_string(),
            variable: key.to_string(),
            message: format!("Missing required variable: {}", key),
            location: format!("{}:?", env),
            suggestion: Some(format!("Add {}=<value> to {}", key, env)),
        });
    }

    // Check 2: Extra variables in strict mode
    if strict {
        let extra: Vec<_> = env_keys.difference(&example_keys).collect();
        for key in &extra {
            issues.push(Issue {
                severity: "warning".to_string(),
                issue_type: "extra_variable".to_string(),
                variable: key.to_string(),
                message: format!("Extra variable not in .env.example: {}", key),
                location: format!("{}:?", env),
                suggestion: Some(format!("Add {} to {} or remove from {}", key, example, env)),
            });
        }
    }

    // Check 3: Placeholder values
    for (key, value) in &env_file.vars {
        if is_placeholder(value) {
            let suggestion = match key.as_str() {
                "SECRET_KEY" => Some("Run: openssl rand -hex 32".to_string()),
                k if k.contains("AWS") => Some("Get from AWS Console".to_string()),
                k if k.contains("STRIPE") => Some("Get from Stripe Dashboard".to_string()),
                _ => None,
            };

            issues.push(Issue {
                severity: "error".to_string(),
                issue_type: "placeholder_value".to_string(),
                variable: key.clone(),
                message: format!("{} looks like a placeholder", key),
                location: format!("{}:?", env),
                suggestion,
            });
        }
    }

    // Check 4: Boolean string trap
    for (key, value) in &env_file.vars {
        if value == "False" || value == "True" {
            issues.push(Issue {
                severity: "warning".to_string(),
                issue_type: "boolean_trap".to_string(),
                variable: key.clone(),
                message: format!("{} is set to \"{}\" (string)", key, value),
                location: format!("{}:?", env),
                suggestion: Some(format!(
                    "This is truthy in Python — use {} or 0 instead",
                    if value == "False" { "False" } else { "True" }
                )),
            });
        }
    }

    // Check 5: Weak SECRET_KEY
    if let Some(secret_key) = env_file.vars.get("SECRET_KEY") {
        if is_weak_secret_key(secret_key) {
            issues.push(Issue {
                severity: "error".to_string(),
                issue_type: "weak_secret".to_string(),
                variable: "SECRET_KEY".to_string(),
                message: "SECRET_KEY is too weak".to_string(),
                location: format!("{}:?", env),
                suggestion: Some("Run: openssl rand -hex 32".to_string()),
            });
        }
    }

    // Check 6: localhost in Docker context
    let has_docker = Path::new("docker-compose.yml").exists()
        || Path::new("docker-compose.yaml").exists()
        || Path::new("Dockerfile").exists();

    if has_docker {
        for (key, value) in &env_file.vars {
            if value.contains("localhost") && (key.contains("URL") || key.contains("HOST")) {
                issues.push(Issue {
                    severity: "warning".to_string(),
                    issue_type: "localhost_in_docker".to_string(),
                    variable: key.clone(),
                    message: format!("{} uses localhost", key),
                    location: format!("{}:?", env),
                    suggestion: Some(
                        "In Docker, use service name instead (e.g., db:5432)".to_string(),
                    ),
                });
            }
        }
    }

    // Create result
    let errors = issues.iter().filter(|i| i.severity == "error").count();
    let warnings = issues.iter().filter(|i| i.severity == "warning").count();
    let style = issues.iter().filter(|i| i.severity == "style").count();

    let result = ValidationResult {
        status: if errors > 0 {
            "failed".to_string()
        } else {
            "passed".to_string()
        },
        required_present: env_file.vars.len().min(example_file.vars.len()),
        required_total: example_file.vars.len(),
        issues,
        summary: Summary {
            errors,
            warnings,
            style,
        },
    };

    // Output
    match format.as_str() {
        "json" => output_json(&result)?,
        "github-actions" => output_github_actions(&result)?,
        _ => output_pretty(&result, &env_file.vars, &example_file.vars)?,
    }

    // Handle --fix flag
    if fix && result.summary.errors > 0 {
        println!("\n{} Auto-fix is not yet implemented", "ℹ️".cyan());
        println!("  This will be added in a future version");
    }

    // Exit code
    if !exit_zero && result.summary.errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn output_pretty(
    result: &ValidationResult,
    _env_vars: &HashMap<String, String>,
    _example_vars: &HashMap<String, String>,
) -> Result<()> {
    if result.issues.is_empty() {
        println!(
            "{} All required variables present ({}/{})",
            "✓".green(),
            result.required_present,
            result.required_total
        );
        println!("{} No issues found", "✓".green());
        return Ok(());
    }

    if result.required_present == result.required_total {
        println!(
            "{} All required variables present ({}/{})",
            "✓".green(),
            result.required_present,
            result.required_total
        );
    } else {
        println!(
            "{} Missing {} required variables",
            "✗".red(),
            result.required_total - result.required_present
        );
    }

    println!("{} Found {} issues\n", "✗".red(), result.issues.len());

    println!("{}", "Issues:".bold());
    for (i, issue) in result.issues.iter().enumerate() {
        let icon = match issue.severity.as_str() {
            "error" => "🚨",
            "warning" => "⚠️ ",
            _ => "ℹ️ ",
        };

        println!("  {}. {} {}", i + 1, icon, issue.message);
        if let Some(suggestion) = &issue.suggestion {
            println!("     → {}", suggestion.dimmed());
        }
        println!("     Location: {}", issue.location.dimmed());
    }

    println!("\n{}", "Summary:".bold());
    if result.summary.errors > 0 {
        println!("  🚨 {} critical issues", result.summary.errors);
    }
    if result.summary.warnings > 0 {
        println!("  ⚠️  {} warnings", result.summary.warnings);
    }
    if result.summary.errors == 0 && result.summary.warnings == 0 {
        println!("  {} 0 issues found", "✓".green());
    }

    Ok(())
}

fn output_json(result: &ValidationResult) -> Result<()> {
    let json = serde_json::to_string_pretty(result)?;
    println!("{}", json);
    Ok(())
}

fn output_github_actions(result: &ValidationResult) -> Result<()> {
    for issue in &result.issues {
        let level = match issue.severity.as_str() {
            "error" => "error",
            "warning" => "warning",
            _ => "notice",
        };

        let location = issue.location.replace(":?", "");

        println!("::{}  file={},line=1::{}", level, location, issue.message);

        if let Some(suggestion) = &issue.suggestion {
            println!(
                "::{}  file={},line=1::Suggestion: {}",
                level, location, suggestion
            );
        }
    }
    Ok(())
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
    ];

    placeholders.iter().any(|p| lower.contains(p))
}

fn is_weak_secret_key(key: &str) -> bool {
    // Too short
    if key.len() < 32 {
        return true;
    }

    // Common weak patterns
    let weak = [
        "secret", "password", "dev", "test", "1234", "abcd", "changeme",
    ];

    let lower = key.to_lowercase();
    weak.iter().any(|w| lower.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_placeholder() {
        assert!(is_placeholder("YOUR_KEY_HERE"));
        assert!(is_placeholder("generate-with-openssl"));
        assert!(!is_placeholder("sk_live_51Hrealkey"));
    }

    #[test]
    fn test_is_weak_secret_key() {
        assert!(is_weak_secret_key("short"));
        assert!(is_weak_secret_key("this-is-a-test-secret-key-do-not-use"));
        assert!(is_weak_secret_key("devkeysecretpassword1234567890"));
        assert!(!is_weak_secret_key(
            "a7b9c4d1e8f2g5h3i6j0k9l8m7n6o5p4q3r2s1t0u9v8w7x6y5z4"
        ));
    }
}
