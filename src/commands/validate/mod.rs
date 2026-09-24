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

use anyhow::Result;
use colored::Colorize;
use lazy_static::lazy_static;
use serde_json;

use crate::core::{Parser, ParserConfig};
use crate::docs;
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;

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
    spec: crate::core::spec::Spec,
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

    // Reject an unknown --format before doing any work. Discovering it after a
    // full parse-and-check pass wastes the run and buries the message under
    // output the user did not ask for.
    if !matches!(
        format.as_str(),
        "pretty" | "text" | "json" | "github-actions" | "github"
    ) {
        anyhow::bail!("unknown format '{format}' — expected one of: pretty, json, github-actions");
    }

    // `--env` already carries the resolved path: `--env-name production` is
    // turned into `.env.production` before the command is called.
    let env_path = env.clone();

    // Build configuration
    let config = ValidationConfig {
        strict,
        fix,
        validate_formats,
        ignore_issues: ignore.into_iter().collect(),
    };

    // Parse files with progress indicator
    let pb = ui::spinner("Parsing configuration files...");
    // let parser = Parser::new(ParserConfig { strict });
    let parser = Parser::new(ParserConfig {
        strict,
        ..ParserConfig::default()
    });

    let example_file = parser.parse_file_or_hint(
        &example,
        "validate compares .env against it, so it needs one. Create it with \
         `evnx sync` (from your .env) or `evnx init`, or point at another file \
         with --example.",
    )?;
    let env_file = parser.parse_file_or_hint(
        &env_path,
        "Create it with `evnx init`, or point at another file with --env.",
    )?;

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
    let env_label = crate::core::env_name::name_of(&env_path);
    let raw_env_content = std::fs::read_to_string(&env_path).unwrap_or_default();

    // ⚠️ A closure, so the checks can be run **again** after `--fix`.
    //
    // The summary used to count this first pass and never recount, so
    // `validate --fix` repaired every issue and then exited 1 — reporting the
    // problems it had just removed. `evnx validate --fix && deploy` therefore
    // never reached `deploy`, and the next plain `validate` exited 0.
    //
    // Recounting by re-running the same checks, rather than by subtracting the
    // fixes that were applied, is what keeps the two from drifting: a fix that
    // does not actually resolve its issue stays visible.
    let collect_issues = |vars: &indexmap::IndexMap<String, String>| -> Vec<Issue> {
        let mut issues = Vec::new();

        // ⚠️ The spec replaces the template as the source of requiredness when the
        // project declares one. Running both would report the same variable twice,
        // and would keep the behaviour the spec exists to fix: `.env.example` has no
        // way to say "optional", so every line in it counts as required.
        if spec.is_empty() {
            issues.extend(check_missing_variables(
                vars,
                &example_file.vars,
                &env_path,
                &config.ignore_issues,
            ));
        } else {
            issues.extend(checks::check_spec_required(
                vars,
                &spec,
                env_label,
                &env_path,
                &config.ignore_issues,
            ));
            issues.extend(checks::check_spec_format(
                vars,
                &spec,
                env_label,
                &env_path,
                &config.ignore_issues,
            ));
        }

        issues.extend(check_extra_variables(
            vars,
            &example_file.vars,
            &env_path,
            config.strict,
            &config.ignore_issues,
        ));

        // Reads the file, not the parsed map: `insert` has already discarded
        // the evidence by the time `vars` exists.
        issues.extend(check_duplicate_keys(
            &raw_env_content,
            &env_path,
            &config.ignore_issues,
        ));

        issues.extend(check_placeholders(vars, &env_path, &config.ignore_issues));

        issues.extend(check_boolean_trap(vars, &env_path, &config.ignore_issues));

        issues.extend(check_weak_secret(vars, &env_path, &config.ignore_issues));

        issues.extend(check_localhost_docker(
            vars,
            &env_path,
            has_docker_context(),
            &config.ignore_issues,
        ));

        issues.extend(check_formats(
            vars,
            &env_path,
            config.validate_formats,
            &config.ignore_issues,
        ));
        issues
    };

    let mut issues = collect_issues(&env_file.vars);

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

        // Weak secrets
        //
        // ⚠️ This was `.find(...)` followed by `env_vars.get("SECRET_KEY")` —
        // the same hardcoded name N2 found in the *check*. So it repaired at
        // most one variable per run, and always looked up SECRET_KEY no matter
        // which variable the issue was actually about.
        for issue in &issues {
            if issue.issue_type == IssueType::WeakSecret.as_str() && issue.auto_fixable {
                if let Some(val) = env_vars.get(&issue.variable) {
                    let action = suggest_fix(&issue.variable, val, &IssueType::WeakSecret);
                    if !matches!(action, FixAction::Skip) {
                        fixes_to_apply.push((issue.variable.clone(), val.clone(), action));
                    }
                }
            }
        }

        // ⚠️ One repair per variable.
        //
        // `SECRET_KEY=CHANGE_ME` trips both the placeholder check and the
        // weak-secret check, and each queued its own fix. Both called
        // `generate_secure_secret()`, so **two different 256-bit secrets were
        // generated, both printed in the report, and only the second reached the
        // file**. A user who copied the first into a dashboard or a password
        // manager held a credential that appeared nowhere in their `.env`, and
        // would debug an authentication failure with no visible cause.
        //
        // It also reported "2 fixed" for one repaired variable, and burned two
        // CSPRNG draws for one value.
        //
        // First wins: the checks run in a fixed order, so this is deterministic.
        let mut already_fixed: Vec<String> = Vec::new();
        let fixes_to_apply: Vec<_> = fixes_to_apply
            .into_iter()
            .filter(|(key, _, _)| {
                if already_fixed.contains(key) {
                    false
                } else {
                    already_fixed.push(key.clone());
                    true
                }
            })
            .collect();

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

            // ⚠️ Recount against the file as it now stands, not as it arrived.
            // Everything below — the summary, the rendered output and the exit
            // code — reads `issues`, so without this `--fix` reports the very
            // problems it just repaired and exits 1.
            issues = collect_issues(&env_vars);
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
        // With a contract, "required" means what the contract says — not how
        // many lines the template happens to have.
        required_present: required_present(&spec, &env_vars, env_label, &example_file.vars),
        required_total: required_total(&spec, env_label, &example_file.vars),
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
        "github-actions" | "github" => output_github_actions(&result, &env_path)?,
        "pretty" | "text" => output_pretty(&result, &env_path, &example)?,
        // ⚠️ Was `_ => output_pretty(..)`. A typo — or `--format sarif`, which
        // `scan` supports and `validate` does not — printed human-readable output
        // and exited as though the requested format had been produced.
        other => {
            anyhow::bail!(
                "unknown format '{other}' — expected one of: pretty, json, github-actions"
            )
        }
    }

    // ─────────────────────────────────────────
    // Exit Code Handling
    // ─────────────────────────────────────────
    if result.summary.errors == 0 && result.summary.warnings == 0 {
        if format == "pretty" {
            ui::success("All checks passed");
        }
    } else if !exit_zero && result.summary.errors > 0 {
        eprintln!(
            "Validation failed: {}",
            pluralize(result.summary.errors, "error", "errors")
        );
    }

    // ✅ Always print — eprintln never pollutes stdout, always before process::exit
    ui::print_docs_hint(&docs::VALIDATE);

    if !exit_zero && result.summary.errors > 0 {
        std::process::exit(1);
    }

    Ok(())
}
// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────
// Output Functions (UI Integration)
// ─────────────────────────────────────────────────────────────

fn output_pretty(result: &ValidationResult, _env_path: &str, _example_path: &str) -> Result<()> {
    // ⚠️ No "Preview:" heading. It was printed unconditionally, so a clean run
    // showed a section title with nothing beneath it — and once its emoji was
    // removed it rendered as a lone indented space.
    if result.fixed.is_empty() && result.issues.is_empty() {
        ui::success("All required variables present");
        return Ok(());
    }

    // Show applied fixes first
    if !result.fixed.is_empty() {
        ui::print_section_header("", "Applied fixes");
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
            // ⚠️ Adding a missing key is half a repair: the key exists, the
            // value does not. Without this line the report reads as evnx
            // contradicting itself — "Applied fixes: B → your_value_here"
            // directly above "✗ B looks like a placeholder".
            if checks::is_placeholder(&fix.new_value) {
                println!(
                    "    {}",
                    "a placeholder — evnx cannot invent this value, so fill it in".dimmed()
                );
            }
        }
        println!();
    }

    // Show issues
    if !result.issues.is_empty() {
        for issue in &result.issues {
            let mark = match issue.severity.as_str() {
                "error" => glyph::FAIL.red(),
                "warning" => glyph::WARN.yellow(),
                _ => glyph::INFO.dimmed(),
            };

            // The location leads the detail line rather than trailing it behind a
            // 📍, so every issue's file:line sits in one column.
            println!("  {}  {}", mark, issue.message);
            println!("     {}", issue.location.dimmed());

            if let Some(suggestion) = &issue.suggestion {
                println!("     {}  {}", glyph::ARROW.dimmed(), suggestion.dimmed());
            }
            if issue.auto_fixable {
                println!(
                    "     {}  {}",
                    glyph::INFO.dimmed(),
                    "fixable with --fix".dimmed()
                );
            }
            println!();
        }
    }

    // One line, only the non-zero counts — a three-line box around three numbers
    // of which two are usually zero is scaffolding, not information.
    let mut parts = Vec::new();
    if result.summary.errors > 0 {
        parts.push(
            pluralize(result.summary.errors, "error", "errors")
                .red()
                .to_string(),
        );
    }
    if result.summary.warnings > 0 {
        parts.push(
            pluralize(result.summary.warnings, "warning", "warnings")
                .yellow()
                .to_string(),
        );
    }
    if result.summary.fixed_count > 0 {
        parts.push(
            pluralize(result.summary.fixed_count, "fixed", "fixed")
                .green()
                .to_string(),
        );
    }
    if !parts.is_empty() {
        println!("  {}", parts.join("  ·  "));
    }

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

/// Which line of `.env` assigns `key`, if any.
///
/// A *missing* variable has no line by definition, so those stay on line 1.
/// Everything else — a placeholder, a weak secret, a boolean trap — is about a
/// line that exists, and pointing at it is the whole value of an annotation.
fn line_of(content: &str, key: &str) -> usize {
    content
        .lines()
        .enumerate()
        .find(|(_, line)| {
            let t = line.trim();
            let t = t.strip_prefix("export ").unwrap_or(t);
            t.split_once('=').is_some_and(|(k, _)| k.trim() == key)
        })
        .map_or(1, |(idx, _)| idx + 1)
}

/// ⚠️ **One annotation per finding, not two.**
///
/// The message and the suggestion were printed as separate annotations, both
/// anchored to `line=1`. GitHub caps annotations at 10 per step and 50 per run,
/// so five missing variables exhausted the step's budget and the rest were
/// dropped silently — on the command whose job is to list what is missing.
///
/// `%0A` is how a newline travels inside a workflow command; GitHub renders it
/// as a line break in the annotation body.
fn output_github_actions(result: &ValidationResult, env_path: &str) -> Result<()> {
    let content = std::fs::read_to_string(env_path).unwrap_or_default();

    for issue in &result.issues {
        let level = match issue.severity.as_str() {
            "error" => "error",
            "warning" => "warning",
            _ => "notice",
        };
        let body = match &issue.suggestion {
            Some(s) => format!("{}%0ASuggestion: {}", issue.message, s),
            None => issue.message.clone(),
        };
        println!(
            "::{} file={},line={}::{}",
            level,
            env_path,
            line_of(&content, &issue.variable),
            body
        );
    }
    for fix in &result.fixed {
        println!(
            "::notice file={},line={}::Fixed: {} → {}",
            env_path,
            line_of(&content, &fix.variable),
            fix.variable,
            fix.action
        );
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Public Re-exports
// ─────────────────────────────────────────────────────────────

pub use types::{FixApplied, Issue, IssueType, Summary, ValidationConfig, ValidationResult};

/// How many required variables the contract declares for this environment.
///
/// Falls back to the template's length, which is what `validate` counted before
/// there was a spec to ask.
fn required_total(
    spec: &crate::core::spec::Spec,
    env_name: Option<&str>,
    example_vars: &indexmap::IndexMap<String, String>,
) -> usize {
    if spec.is_empty() {
        return example_vars.len();
    }
    spec.values()
        .filter(|v| v.is_required() && v.applies_to(env_name))
        .count()
}

/// How many of those are actually present.
fn required_present(
    spec: &crate::core::spec::Spec,
    env_vars: &indexmap::IndexMap<String, String>,
    env_name: Option<&str>,
    example_vars: &indexmap::IndexMap<String, String>,
) -> usize {
    if spec.is_empty() {
        return env_vars.len().min(example_vars.len());
    }
    spec.iter()
        .filter(|(_, v)| v.is_required() && v.applies_to(env_name))
        .filter(|(k, _)| env_vars.contains_key(*k))
        .count()
}
