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

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use serde_json;

use crate::core::{Parser, ParserConfig};
use crate::docs;
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;

use self::checks::*;
use self::fixer::*;

// ─────────────────────────────────────────────────────────────
// Inputs
// ─────────────────────────────────────────────────────────────

/// Everything `evnx validate` was asked, after `.evnx.toml` has been merged in.
///
/// The lists that can come from both places arrive separately, so an unknown
/// value can be blamed on the one that holds it (I-038).
pub struct Options {
    /// The env file, already resolved from `--env`, `--env-name` or
    /// `[defaults] env_name`.
    pub env: String,
    pub example: String,
    pub strict: bool,
    pub fix: bool,
    /// `--fix --dry-run`: work the fixes out, write nothing.
    pub dry_run: bool,
    pub rotate_weak_secrets: bool,
    pub format: String,
    pub exit_zero: bool,
    /// `--fail-on`.
    pub fail_on: Option<String>,
    /// `[validate] fail_on`.
    pub fail_on_configured: Option<String>,
    pub verbose: bool,
    /// `--ignore`.
    pub ignore: Vec<String>,
    /// `[validate] ignore`.
    pub ignore_configured: Vec<String>,
    pub validate_formats: bool,
    pub spec: crate::core::spec::Spec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pretty,
    Json,
    Github,
}

/// The lowest severity that fails the run (`--fail-on`, `[validate] fail_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOn {
    Error,
    Warning,
}

fn parse_fail_on(value: &str) -> Result<FailOn> {
    crate::core::args::one_of(
        "level",
        value,
        &[("warning", FailOn::Warning), ("error", FailOn::Error)],
        &["warning", "error"],
    )
}

/// The issue types in `list`, each one checked (I-038).
fn issue_types(list: &[String]) -> Result<Vec<&'static str>> {
    let accepted: Vec<(&str, &'static str)> = IssueType::ALL
        .iter()
        .map(|t| (t.as_str(), t.as_str()))
        .collect();
    let shown: Vec<&str> = accepted.iter().map(|(n, _)| *n).collect();
    list.iter()
        .map(|t| crate::core::args::one_of("issue type", t, &accepted, &shown))
        .collect()
}

// ─────────────────────────────────────────────────────────────
// Facts about the file's surroundings
// ─────────────────────────────────────────────────────────────

/// The folder an env file path is in, as a path that can be joined to.
fn folder_of(env_path: &str) -> &Path {
    match Path::new(env_path).parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Does `localhost` in this file mean a container's own loopback?
///
/// The env file's folder and each folder above it up to the repository root,
/// then the current directory (where it looked before), then `DOCKER_HOST`
/// (I-046). The file list is [`crate::core::project::DOCKER_FILES`], shared
/// with `doctor` (DEC-VA3).
fn has_docker_context(env_path: &str) -> bool {
    crate::core::project::docker_dir(folder_of(env_path)).is_some()
        || !crate::core::project::docker_files_in(Path::new(".")).is_empty()
        || std::env::var_os("DOCKER_HOST").is_some()
}

/// How git treats the env file, or `None` outside a repository (D-27).
///
/// ⚠️ Only inside a work tree. `gitignore::exposure` falls back to reading
/// `.gitignore` when git cannot answer, and without a repository there is no
/// `.gitignore` to read — every file would be "not ignored", in a folder where
/// nothing can commit it.
fn git_exposure(env_path: &str) -> Option<crate::core::gitignore::Exposure> {
    let dir = folder_of(env_path);
    let name = Path::new(env_path).file_name()?.to_str()?;
    crate::core::gitignore::in_work_tree(dir).then(|| crate::core::gitignore::exposure(dir, name))
}

/// The environments that exist beside the env file: its `.env.<name>`
/// siblings and the suggested names (I-042).
fn known_environments(env_path: &str) -> Vec<String> {
    let mut known: Vec<String> = crate::core::env_name::SUGGESTED
        .iter()
        .map(|s| s.to_string())
        .collect();
    for name in crate::core::env_name::available(folder_of(env_path)) {
        if !known.contains(&name) {
            known.push(name);
        }
    }
    known
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point
// ─────────────────────────────────────────────────────────────

/// Run the validate command.
///
/// This function is called from `main.rs` after CLI args are parsed.
pub fn run(opts: Options) -> Result<()> {
    // Reject an unknown --format before doing any work. Discovering it after a
    // full parse-and-check pass wastes the run and buries the message under
    // output the user did not ask for. One rule with scan and doctor: any case,
    // spaces trimmed (I-038).
    let format = crate::core::args::one_of(
        "format",
        &opts.format,
        &[
            ("pretty", Format::Pretty),
            ("text", Format::Pretty),
            ("json", Format::Json),
            ("github-actions", Format::Github),
            ("github", Format::Github),
        ],
        &["pretty", "json", "github-actions"],
    )?;

    // ⚠️ I-038: a typo in `--ignore placeholder_vlaue` used to suppress
    // nothing and say nothing. Both lists are checked, whichever is used.
    let mut ignore_issues: HashSet<String> = HashSet::new();
    for t in issue_types(&opts.ignore)? {
        ignore_issues.insert(t.to_string());
    }
    let configured = issue_types(&opts.ignore_configured)
        .with_context(|| format!("{} [validate] ignore", crate::core::config::PROJECT_FILE))?;
    for t in configured {
        ignore_issues.insert(t.to_string());
    }

    let configured_fail_on = opts
        .fail_on_configured
        .as_deref()
        .map(parse_fail_on)
        .transpose()
        .with_context(|| format!("{} [validate] fail_on", crate::core::config::PROJECT_FILE))?;
    let fail_on = match opts.fail_on.as_deref() {
        Some(v) => parse_fail_on(v)?,
        None => configured_fail_on.unwrap_or(FailOn::Error),
    };

    // ─────────────────────────────────────────
    // UI: Header (only for pretty output)
    // ─────────────────────────────────────────
    if format == Format::Pretty {
        ui::print_header("evnx validate", Some("Check environment configuration"));
        if opts.verbose {
            ui::info("Running in verbose mode");
        }
    } else if opts.verbose {
        // Verbose for machine formats goes to stderr
        eprintln!(
            "[verbose] validate: env={}, example={}",
            opts.env, opts.example
        );
    }

    // `--env` already carries the resolved path: `--env-name production` is
    // turned into `.env.production` before the command is called.
    let env_path = opts.env.clone();
    let example = opts.example.clone();
    let spec = &opts.spec;

    // Build configuration
    let config = ValidationConfig {
        strict: opts.strict,
        fix: opts.fix,
        rotate_weak_secrets: opts.rotate_weak_secrets,
        validate_formats: opts.validate_formats,
        ignore_issues,
    };

    // Parse files with progress indicator
    let pb = ui::spinner("Parsing configuration files...");
    let parser = Parser::new(ParserConfig {
        strict: opts.strict,
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

    if opts.verbose {
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

    // Resolved once, so the report and the repair cannot disagree about it.
    let weak_secret_policy = if config.rotate_weak_secrets {
        fixer::WeakSecretPolicy::Rotate
    } else {
        fixer::WeakSecretPolicy::PlaceholdersOnly
    };

    // Facts about the file's surroundings, looked up once: `--fix` changes
    // none of them.
    let docker = has_docker_context(&env_path);
    let exposure = git_exposure(&env_path);
    let known = known_environments(&env_path);
    let known: Vec<&str> = known.iter().map(String::as_str).collect();

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
            issues.extend(check_extra_variables(
                vars,
                &example_file.vars,
                &env_path,
                config.strict,
                &config.ignore_issues,
            ));
        } else {
            issues.extend(checks::check_spec_required(
                vars,
                spec,
                env_label,
                &env_path,
                &config.ignore_issues,
            ));
            issues.extend(checks::check_spec_format(
                vars,
                spec,
                env_label,
                &env_path,
                &config.ignore_issues,
            ));
            issues.extend(checks::check_spec_keys(spec, &config.ignore_issues));
            issues.extend(checks::check_environments(
                spec,
                env_label,
                &known,
                &env_path,
                &config.ignore_issues,
            ));
            // With a contract, "declared" means declared in `[vars]` for this
            // environment — what `--help` has always said (I-039, I-043).
            issues.extend(checks::check_spec_extras(
                vars,
                spec,
                env_label,
                &env_path,
                config.strict,
                &config.ignore_issues,
            ));
        }

        // Reads the file, not the parsed map: `insert` has already discarded
        // the evidence by the time `vars` exists.
        issues.extend(check_duplicate_keys(
            &raw_env_content,
            &env_path,
            &config.ignore_issues,
        ));

        issues.extend(check_placeholders(vars, &env_path, &config.ignore_issues));

        issues.extend(check_boolean_trap(vars, &env_path, &config.ignore_issues));

        issues.extend(check_weak_secret(
            vars,
            &env_path,
            &config.ignore_issues,
            weak_secret_policy,
        ));

        issues.extend(check_localhost_docker(
            vars,
            &env_path,
            docker,
            &config.ignore_issues,
        ));

        issues.extend(check_formats(
            vars,
            &env_path,
            config.validate_formats,
            &config.ignore_issues,
        ));

        issues.extend(checks::check_exposure(
            exposure,
            &env_path,
            &config.ignore_issues,
        ));
        checks::one_finding_per_cause(issues)
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

        // Missing variables: a new secret where evnx may generate one
        // (DEC-VA6), the template's value where it may copy it (I-037, D-25),
        // a placeholder where it may do neither.
        for issue in &issues {
            if issue.issue_type == IssueType::MissingVariable.as_str() && issue.auto_fixable {
                let action = if crate::core::classify::may_generate(&issue.variable) {
                    FixAction::AddSecret
                } else {
                    FixAction::AddMissing(fixer::fill_for_missing(
                        &issue.variable,
                        example_file.vars.get(&issue.variable).map(String::as_str),
                    ))
                };
                fixes_to_apply.push((issue.variable.clone(), String::new(), action));
            }
        }

        // Placeholders
        for issue in &issues {
            if issue.issue_type == IssueType::PlaceholderValue.as_str() && issue.auto_fixable {
                if let Some(val) = env_vars.get(&issue.variable) {
                    let action = suggest_fix(
                        &issue.variable,
                        val,
                        &IssueType::PlaceholderValue,
                        weak_secret_policy,
                    );
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
                    let action = suggest_fix(
                        &issue.variable,
                        val,
                        &IssueType::BooleanTrap,
                        weak_secret_policy,
                    );
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
                    let action = suggest_fix(
                        &issue.variable,
                        val,
                        &IssueType::WeakSecret,
                        weak_secret_policy,
                    );
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
            if let Some(mut fix) = apply_fix(&key, &old_val, &action, &mut env_vars) {
                // ⚠️ A dry run never shows a generated secret: the real run
                // makes a different one, and a value printed here would be
                // copied somewhere and match nothing.
                if opts.dry_run
                    && matches!(action, FixAction::GenerateSecret | FixAction::AddSecret)
                {
                    fix.new_value = "(a new random secret)".to_string();
                }
                fixes_applied.push(fix);
            }
        }

        pb.finish_with_message(format!("Applied {} fix(es) ✓", fixes_applied.len()));

        // Write to file if any fixes were applied
        if !fixes_applied.is_empty() && !opts.dry_run {
            let original = std::fs::read_to_string(&env_path).unwrap_or_default();
            // ⚠️ Only the keys actually repaired. Passing the whole map is what
            // made `write_fixed_file` rewrite — and damage — every line in the
            // file; see the note on that function.
            let changed: std::collections::BTreeSet<String> =
                fixes_applied.iter().map(|f| f.variable.clone()).collect();
            let (backup, written) = write_fixed_file(&env_path, &env_vars, &original, &changed)?;
            if format == Format::Pretty {
                ui::success(format!(
                    "Saved fixes to {env_path} — original kept at {backup}"
                ));
                crate::core::safe_write::report(&written);
            }

            // ⚠️ Recount against the file as it now stands, not as it arrived.
            // Everything below — the summary, the rendered output and the exit
            // code — reads `issues`, so without this `--fix` reports the very
            // problems it just repaired and exits 1.
            issues = collect_issues(&env_vars);
        }
    }

    // ⚠️ I-037: a variable `--fix` could only add with a placeholder is not
    // fixed. It is listed apart, and is not counted as a repair.
    let (needs_input, fixed): (Vec<_>, Vec<_>) = fixes_applied
        .into_iter()
        .partition(|f| checks::is_placeholder(&f.variable, &f.new_value));

    // ─────────────────────────────────────────
    // Build Result
    // ─────────────────────────────────────────
    let errors = issues.iter().filter(|i| i.severity == "error").count();
    let warnings = issues.iter().filter(|i| i.severity == "warning").count();
    let style = issues.iter().filter(|i| i.severity == "style").count();

    // Counted on the file as it is now: before the fix for a dry run (nothing
    // was written), after it otherwise.
    let counted = if opts.dry_run {
        &env_file.vars
    } else {
        &env_vars
    };
    let result = ValidationResult {
        status: if errors > 0 {
            "failed".to_string()
        } else {
            "passed".to_string()
        },
        dry_run: opts.dry_run,
        // With a contract, "required" means what the contract says — not how
        // many lines the template happens to have.
        required_present: required_present(spec, counted, env_label, &example_file.vars),
        required_total: required_total(spec, env_label, &example_file.vars),
        issues,
        summary: Summary {
            errors,
            warnings,
            style,
            fixed_count: fixed.len(),
            needs_input: needs_input.len(),
        },
        fixed,
        needs_input,
    };

    // ─────────────────────────────────────────
    // Output (Improvement #6: UI integration)
    // ─────────────────────────────────────────
    match format {
        Format::Json => output_json(&result)?,
        Format::Github => output_github_actions(&result, &env_path)?,
        Format::Pretty => output_pretty(&result, &env_path)?,
    }

    // ─────────────────────────────────────────
    // Exit Code Handling
    // ─────────────────────────────────────────
    let fails =
        result.summary.errors > 0 || (fail_on == FailOn::Warning && result.summary.warnings > 0);

    if result.summary.errors == 0 && result.summary.warnings == 0 {
        if format == Format::Pretty {
            ui::success("All checks passed");
        }
    } else if !opts.exit_zero && fails {
        eprintln!("Validation failed: {}", failure_counts(&result, fail_on));
    }

    // ✅ Always print — eprintln never pollutes stdout, always before process::exit
    ui::print_docs_hint(&docs::VALIDATE);

    if !opts.exit_zero && fails {
        std::process::exit(1);
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Placeholder findings for variables listed under "Needs your input": the
/// pretty report shows them once, there, rather than again as fresh errors
/// (I-037). They stay in `issues` for the machine formats.
fn hidden_by_needs_input(result: &ValidationResult) -> HashSet<&str> {
    if result.dry_run {
        return HashSet::new();
    }
    result
        .needs_input
        .iter()
        .map(|f| f.variable.as_str())
        .filter(|v| {
            result
                .issues
                .iter()
                .any(|i| i.variable == *v && i.issue_type == IssueType::PlaceholderValue.as_str())
        })
        .collect()
}

/// `1 needs your input`, `2 need your input`.
fn need_input(n: usize) -> String {
    format!("{n} {} your input", if n == 1 { "needs" } else { "need" })
}

/// `2 errors`, `1 error, 2 need your input`, `1 warning` — what failed the run.
fn failure_counts(result: &ValidationResult, fail_on: FailOn) -> String {
    let hidden = hidden_by_needs_input(result).len();
    let mut parts = Vec::new();
    let shown_errors = result.summary.errors - hidden;
    if shown_errors > 0 {
        parts.push(pluralize(shown_errors, "error", "errors"));
    }
    if hidden > 0 {
        parts.push(need_input(hidden));
    }
    if fail_on == FailOn::Warning && result.summary.warnings > 0 {
        parts.push(pluralize(result.summary.warnings, "warning", "warnings"));
    }
    parts.join(", ")
}

// ─────────────────────────────────────────────────────────────
// Output Functions (UI Integration)
// ─────────────────────────────────────────────────────────────

/// One applied fix: `• NAME: "old" → new`.
fn print_fix(fix: &FixApplied) {
    // Three states, not two: shown, withheld, and never existed.
    // Collapsing the middle one into "(new)" would say a credential was
    // added when it was in fact replaced.
    let old = match (&fix.old_value, fix.old_value_withheld) {
        (Some(v), _) => format!("\"{v}\""),
        (None, true) => "(previous value withheld — see the .env.bak)".to_string(),
        (None, false) => "(new)".to_string(),
    };
    let new = if fix.new_value.is_empty() {
        "(empty)".to_string()
    } else {
        fix.new_value.clone()
    };
    println!(
        "  • {}: {} → {}",
        fix.variable.bold(),
        old.dimmed(),
        new.green()
    );
}

fn output_pretty(result: &ValidationResult, env_path: &str) -> Result<()> {
    // ⚠️ No "Preview:" heading. It was printed unconditionally, so a clean run
    // showed a section title with nothing beneath it — and once its emoji was
    // removed it rendered as a lone indented space.
    if result.fixed.is_empty() && result.needs_input.is_empty() && result.issues.is_empty() {
        ui::success("All required variables present");
        return Ok(());
    }

    let hidden = hidden_by_needs_input(result);

    // A real run shows its fixes first: they already happened. A dry run
    // shows the file's findings first, then what `--fix` would do about them.
    if !result.dry_run {
        print_fixes(result, true);
    }

    // Show issues
    for issue in result.issues.iter().filter(|i| {
        !(hidden.contains(i.variable.as_str())
            && i.issue_type == IssueType::PlaceholderValue.as_str())
    }) {
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

    if result.dry_run {
        // Each finding above ends with its own blank line.
        print_fixes(result, result.issues.is_empty());
        println!(
            "  {}",
            format!(
                "Dry run — nothing was written: {env_path} is unchanged and no backup was made"
            )
            .dimmed()
        );
        println!();
    }

    // One line, only the non-zero counts — a three-line box around three numbers
    // of which two are usually zero is scaffolding, not information.
    let shown_errors = result.summary.errors - hidden.len();
    let mut parts = Vec::new();
    if shown_errors > 0 {
        parts.push(pluralize(shown_errors, "error", "errors").red().to_string());
    }
    if result.summary.warnings > 0 {
        parts.push(
            pluralize(result.summary.warnings, "warning", "warnings")
                .yellow()
                .to_string(),
        );
    }
    if !result.dry_run {
        if result.summary.fixed_count > 0 {
            parts.push(
                pluralize(result.summary.fixed_count, "fixed", "fixed")
                    .green()
                    .to_string(),
            );
        }
        if result.summary.needs_input > 0 {
            parts.push(need_input(result.summary.needs_input).yellow().to_string());
        }
    }
    if !parts.is_empty() {
        println!("  {}", parts.join("  ·  "));
    }

    // Next steps if there are errors
    if shown_errors > 0 {
        ui::print_next_steps(&[
            "Review and fix the errors above",
            "Run with --fix to auto-correct common issues",
            "Use --ignore issue_type to suppress specific warnings",
        ]);
    } else if !hidden.is_empty() {
        ui::print_next_steps(&[
            "Fill in the values listed under \"Needs your input\" — evnx cannot invent them",
            "Run evnx validate again",
        ]);
    }

    Ok(())
}

/// "Fixed (n)" and "Needs your input (n)", or their dry-run forms.
/// `lead` puts a blank line before the first heading.
fn print_fixes(result: &ValidationResult, lead: bool) {
    let (fixed, needs) = if result.dry_run {
        ("Would fix", "Would need your input")
    } else {
        ("Fixed", "Needs your input")
    };
    let mut lead = lead;
    let mut heading = |title: String| {
        if lead {
            println!();
        }
        lead = true;
        println!("{}:", title.bold());
    };
    if !result.fixed.is_empty() {
        heading(format!("{fixed} ({})", result.fixed.len()));
        for fix in &result.fixed {
            print_fix(fix);
        }
    }
    if !result.needs_input.is_empty() {
        heading(format!("{needs} ({})", result.needs_input.len()));
        for fix in &result.needs_input {
            print_fix(fix);
            // The placeholder finding is not printed again, but its advice
            // ("Get from Stripe Dashboard") is still worth having.
            if let Some(advice) = result
                .issues
                .iter()
                .find(|i| {
                    i.variable == fix.variable
                        && i.issue_type == IssueType::PlaceholderValue.as_str()
                })
                .and_then(|i| i.suggestion.as_deref())
            {
                println!("    {}  {}", glyph::ARROW.dimmed(), advice.dimmed());
            }
        }
        // ⚠️ Adding a missing key is half a repair: the key exists, the value
        // does not. Said once, under the list, rather than reported again as
        // "✗ B looks like a placeholder" (I-037).
        println!(
            "    {}",
            "evnx cannot invent these values: fill them in, then run validate again".dimmed()
        );
    }
    if !result.fixed.is_empty() || !result.needs_input.is_empty() {
        println!();
    }
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
    if key.is_empty() {
        return 1;
    }
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
    // Only completed repairs are "Fixed": a placeholder `--fix` added keeps its
    // `::error` above until someone fills it in (I-037).
    let verb = if result.dry_run { "Would fix" } else { "Fixed" };
    for fix in &result.fixed {
        println!(
            "::notice file={},line={}::{verb}: {} → {}",
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
///
/// ⚠️ I-034: without a contract this was `env_vars.len().min(example_vars.len())`
/// — any variables at all, declared or not — so a file missing two of eight
/// with one extra reported 7 of 8, and a file of extras alone reported them as
/// present requirements. It counts the declared names that are there.
fn required_present(
    spec: &crate::core::spec::Spec,
    env_vars: &indexmap::IndexMap<String, String>,
    env_name: Option<&str>,
    example_vars: &indexmap::IndexMap<String, String>,
) -> usize {
    if spec.is_empty() {
        return example_vars
            .keys()
            .filter(|k| env_vars.contains_key(*k))
            .count();
    }
    spec.iter()
        .filter(|(_, v)| v.is_required() && v.applies_to(env_name))
        .filter(|(k, _)| env_vars.contains_key(*k))
        .count()
}
