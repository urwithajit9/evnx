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

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use colored::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

// Import existing UI utilities from project
use super::types::*;
use crate::core::env_name::{self, FileKind};
use crate::core::{gitignore, permissions, verdict};
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
/// * `skip_without_env` - With no env file, say so and exit 0 (a pre-commit
///   hook on a fresh clone, DEC-D17)
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
/// | 2 | doctor could not run, or a check could not finish |
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
    skip_without_env: bool,
) -> Result<()> {
    let project_root = PathBuf::from(&path);

    // ⚠️ Checked before anything else. Diagnosing a directory that is not there
    // would otherwise report "no .env file" — a finding about the project rather
    // than about the path, which is the shape of fail-open `scan` just had fixed.
    if !project_root.is_dir() {
        no_verdict(format!(
            "{} is not a directory evnx can read",
            project_root.display()
        ));
    }

    // ⚠️ Precedence, not "either wins" — unlike `fix` below. A format is a
    // choice between two answers rather than a guard, so an explicit
    // `--format pretty` must be able to override `EVNX_OUTPUT_JSON=1` exported
    // by a CI image. `Option<String>` is what makes "asked for pretty"
    // distinguishable from "did not ask".
    let json_output = match format.as_deref() {
        Some(value) => match crate::core::args::one_of(
            "format",
            value,
            &[("json", true), ("pretty", false), ("text", false)],
            &["pretty", "json"],
        ) {
            Ok(json) => json,
            Err(e) => no_verdict(e.to_string()),
        },
        None => std::env::var("EVNX_OUTPUT_JSON")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "json"))
            .unwrap_or(false),
    };

    // Either source saying yes is enough. `EVNX_AUTO_FIX` shipped first and is
    // documented, so it keeps working; a flag can turn repair on, never off.
    let from_env = std::env::var("EVNX_AUTO_FIX")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false);
    let auto_fix = fix || from_env;

    // No env file is a verdict of its own (I-162, DEC-D8), decided before any
    // check: a folder with nothing in it was "healthy", exit 0, so a CI step
    // pointed at the wrong checkout passed.
    if env_files(&project_root).is_empty() {
        if skip_without_env {
            verdict::skipped(&format!("no env file in {}", project_root.display()));
            return Ok(());
        }
        if templates_in(&project_root).is_empty() && !is_a_project(&project_root) {
            no_verdict(format!(
                "{} has no env file, no .env.example and no project files: nothing to \
                 diagnose here",
                project_root.display()
            ));
        }
    }

    // DEC-D12: a variable in a CI image turning every `doctor` into a repair
    // run is surprising, so it says so. Not for `--fix`, which was asked for.
    let notice = from_env && !fix;

    let result = if json_output {
        run_json(&project_root, verbose, auto_fix, strict, notice)
    } else {
        run_text(&project_root, verbose, auto_fix, strict, notice)
    };

    // ✅ Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::DOCTOR);

    result
}

/// Healthy — no errors, and no warnings under `--strict`.
pub const EXIT_HEALTHY: i32 = verdict::EXIT_OK;
/// Problems found.
pub const EXIT_PROBLEMS: i32 = verdict::EXIT_FINDINGS;
/// Doctor could not run, so there is no verdict.
pub const EXIT_ERROR: i32 = verdict::EXIT_ERROR;

/// Say why doctor examined nothing, on stderr, and exit 2.
fn no_verdict(message: String) -> ! {
    eprintln!("{} {message}", "Error:".on_red().bold());
    eprintln!();
    eprintln!("No verdict: doctor examined nothing. This is not a clean result.");
    std::process::exit(EXIT_ERROR);
}

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

/// Run every check, in order, and hand each result to `each` as it completes.
///
/// ⚠️ One loop for both renderers (I-169). `run_text` and `run_json` each had
/// their own copy; every change to how a check runs or is fixed had to be made
/// twice, and a check that could not run (I-163) would have been a third.
fn build_report(
    project_root: &Path,
    verbose: bool,
    auto_fix: bool,
    mut each: impl FnMut(&CheckResult),
) -> DiagnosticReport {
    let mut report = DiagnosticReport {
        project_path: project_root.to_string_lossy().to_string(),
        checks: Vec::new(),
        summary: Summary::new(),
        timestamp: Utc::now().to_rfc3339(),
    };
    for check in get_all_checks() {
        let mut result = run_check(check.as_ref(), project_root, verbose, auto_fix);
        normalise_details(&mut result);
        report.summary.add(&result);
        each(&result);
        report.checks.push(result);
    }
    report
}

/// Exit with the report's verdict: 2 when a check could not run (DEC-D9),
/// 1 on problems, otherwise return.
fn finish(report: &DiagnosticReport, strict: bool) -> Result<()> {
    let not_run = report.checks.iter().filter(|c| !c.ran).count();
    if not_run > 0 {
        eprintln!();
        eprintln!(
            "No complete verdict: {} could not run. This is not a clean result.",
            pluralize(not_run, "check", "checks")
        );
        std::process::exit(EXIT_ERROR);
    }
    if is_failure(&report.summary, strict) {
        std::process::exit(EXIT_PROBLEMS);
    }
    Ok(())
}

/// DEC-D12. On stderr, so `--format json` keeps stdout as JSON; hidden by `-q`.
fn auto_fix_notice() {
    if !ui::is_quiet() {
        eprintln!(
            "{} auto-fix enabled by EVNX_AUTO_FIX: repairs are made as with --fix",
            glyph::INFO.dimmed()
        );
    }
}

// ─────────────────────────────────────────────────────────────
// Text Output Mode (uses existing ui:: functions)
// ─────────────────────────────────────────────────────────────

fn run_text(
    project_root: &Path,
    verbose: bool,
    auto_fix: bool,
    strict: bool,
    notice: bool,
) -> Result<()> {
    // Use existing UI header function (requires subtitle param)
    ui::print_header("evnx doctor", Some("Diagnosing environment setup"));
    if notice {
        auto_fix_notice();
    }

    if verbose {
        ui::info(format!("Project path: {}", project_root.display()));
    }
    // `-q` (I-164, decided: hides the report): checks run as usual, but nothing
    // prints until the end, and then only if something needs attention. A
    // healthy project prints nothing; findings still print (G1).
    let quiet = ui::is_quiet();
    if !quiet {
        println!();
    }

    // Print each result as it completes, using existing UI utilities
    let report = build_report(project_root, verbose, auto_fix, |result| {
        if !quiet {
            print_check_result_text(result, verbose);
        }
    });

    if quiet {
        // No errors and no warnings cannot be a failure, even under `--strict`.
        if report.summary.errors == 0 && report.summary.warnings == 0 {
            return Ok(());
        }
        println!();
        for result in &report.checks {
            print_check_result_text(result, verbose);
        }
    }

    // Print summary
    print_summary_text(&report.summary);

    // Print recommendations if needed
    if report.summary.errors > 0 || (verbose && report.summary.warnings > 0) {
        print_recommendations_text(&report.checks, auto_fix);
    }

    finish(&report, strict)
}

// ─────────────────────────────────────────────────────────────
// JSON Output Mode (for CI/CD integration)
// ─────────────────────────────────────────────────────────────

fn run_json(
    project_root: &Path,
    verbose: bool,
    auto_fix: bool,
    strict: bool,
    notice: bool,
) -> Result<()> {
    if notice {
        auto_fix_notice();
    }
    let report = build_report(project_root, verbose, auto_fix, |_| {});

    // Output JSON to stdout (machine-readable for CI/CD)
    let json =
        serde_json::to_string_pretty(&report).context("Failed to serialize diagnostic report")?;
    println!("{}", json);

    finish(&report, strict)
}

// ─────────────────────────────────────────────────────────────
// Check Definition and Execution
// ─────────────────────────────────────────────────────────────

/// Trait for individual diagnostic checks
trait DiagnosticCheck: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult>;

    /// This check's result, with nothing to fix. The description is always
    /// the check's own (I-166).
    fn result(&self, severity: Severity, details: Option<String>) -> CheckResult {
        CheckResult {
            name: self.name().to_string(),
            description: self.description().to_string(),
            severity,
            details,
            fixable: false,
            fixed: false,
            ran: true,
            fix_action: None,
        }
    }
}

/// `lines` as a check's `details`: one per line, `None` when there are none.
fn details(lines: Vec<String>) -> Option<String> {
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Run `check`; under `--fix`, apply its fix and **run it again** (DEC-D1).
///
/// ⚠️ A fix used to count as done when the fix function returned `true`, and the
/// check was then shown as ✓ with exit 0. `.gitignore` reading `.env*` then
/// `!.env` was "fixed" that way while git still committed `.env` (I-156). Now
/// the check is the judge: still failing after the fix means "not fixed",
/// exit 1, and doctor stops offering `--fix` for it.
///
/// ⚠️ A check that fails to run is a result, not an abort (I-163, DEC-D9). A
/// directory named `requirements.txt` used to stop the whole run at the second
/// check with `Error: Is a directory (os error 21)`: no check name, no path,
/// no summary. A fix that fails is a result too — "not fixed", with why.
fn run_check(
    check: &dyn DiagnosticCheck,
    project_root: &Path,
    verbose: bool,
    auto_fix: bool,
) -> CheckResult {
    let result = match check.run(project_root, verbose) {
        Ok(result) => result,
        Err(e) => return could_not_run(check, &e),
    };
    let fix_fn = match result.fix_action {
        Some(f) if auto_fix && result.fixable && !result.fixed => f,
        _ => return result,
    };
    let outcome = fix_fn(project_root, verbose);
    let mut after = match check.run(project_root, verbose) {
        Ok(after) => after,
        Err(e) => return could_not_run(check, &e),
    };
    let changed = matches!(outcome, Ok(true));
    if after.fixable || after.severity == Severity::Error {
        after.fixable = false;
        let note = match &outcome {
            Err(e) => format!("--fix could not change this: {e:#}"),
            Ok(true) => "--fix changed what it could; the problem above remains".into(),
            Ok(false) => "--fix could not change this; the problem above remains".into(),
        };
        after.details = Some(match after.details.take() {
            Some(d) => format!("{d}\n! not fixed: {note}"),
            None => format!("! not fixed: {note}"),
        });
    } else if changed {
        after.fixed = true;
    }
    after
}

/// The result of a check that stopped before it could answer (DEC-D9).
fn could_not_run(check: &dyn DiagnosticCheck, error: &anyhow::Error) -> CheckResult {
    CheckResult {
        ran: false,
        ..check.result(Severity::Error, Some(format!("could not run: {error:#}")))
    }
}

/// One "Fixed:" line per change (DEC-D1). On stderr, so `--format json` keeps
/// stdout as JSON; hidden by `-q` like other success lines.
fn report_fix(line: String) {
    if !ui::is_quiet() {
        eprintln!("{} Fixed: {line}", ui::glyph::OK.green());
    }
}

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
        // ⚠️ Every env file, not only `.env`.
        //
        // This check looked at `.env` alone, so a project with `.env.production`
        // sitting unignored got a green tick — evnx actively reported that the
        // setup was fine while the file holding production credentials was
        // committable. Reporting "healthy" over a real exposure is worse than
        // saying nothing.
        let env_files = env_files(project_root);

        if env_files.is_empty() {
            // DEC-D8: beside a template, a missing `.env` is the project not
            // set up — an error. Without one it is only worth saying.
            return Ok(match templates_in(project_root).first() {
                Some(template) => self.result(
                    Severity::Error,
                    details(vec![
                        format!(
                            ".env not found, though {template} is here: this project expects one"
                        ),
                        format!(
                            "→ create it from the template: {}",
                            create_from_template(project_root, template)
                        ),
                    ]),
                ),
                None => self.result(
                    Severity::Warning,
                    Some(".env not found, and there is no .env.example to create it from".into()),
                ),
            });
        }

        let mut lines = Vec::new();
        let mut severity = Severity::Ok;

        // Fixable only when a file is merely unignored: `--fix` edits
        // `.gitignore` and never runs git (DEC-D2), so a tracked file is not
        // something it can repair.
        let mut fixable = false;
        for file in &env_files {
            let name = &file.name;
            // A plaintext copy exposes what the file it copies does (DEC-D18).
            let copy = if file.copy {
                ": a plaintext copy of an env file"
            } else {
                ""
            };
            if let Some(target) = &file.link {
                lines.push(format!("· {name} is a symlink to {}", target.display()));
            }
            // ⓘ Git is asked about the name in the project, never a symlink's
            // target: git tracks and ignores the link itself.
            match gitignore::exposure(project_root, name) {
                gitignore::Exposure::Tracked => {
                    // ⚠️ I-157: this said "NOT in .gitignore", which is false
                    // when .gitignore covers it, and sent the user to the one
                    // place that cannot help. The values are already in git.
                    severity = Severity::Error;
                    lines.push(format!(
                        "✗ {name} is tracked by git: its values are in the repository history{copy}"
                    ));
                    lines.push(format!(
                        "→ git rm --cached {name}, commit, then rotate every secret it held \
                         (`evnx scan .` lists them)"
                    ));
                }
                gitignore::Exposure::Unignored => {
                    severity = Severity::Error;
                    fixable = true;
                    // Wording unchanged: `doctor --format json` carries it
                    // and other programs parse that output.
                    lines.push(format!(
                        "✗ {name} is NOT in .gitignore (security risk){copy}"
                    ));
                }
                gitignore::Exposure::Protected => {
                    if verbose {
                        lines.push(format!("✓ {name} is properly ignored by git"));
                    }
                }
            }

            // A copy is not loaded by anything, so its syntax is no one's
            // problem; structured files are not dotenv at all (DEC-D19).
            if file.copy || env_name::is_structured(name) {
                continue;
            }
            // DEC-D7: the verdict `validate` gives — a file the parser rejects,
            // or cannot read, is an error, not a warning that exits 0 (I-161).
            match read_text(&project_root.join(name), name) {
                Err(why) => {
                    severity = Severity::Error;
                    lines.push(format!("✗ {why}"));
                }
                Ok(content) => match syntax_problems(&content) {
                    problems if problems.is_empty() => {
                        if verbose {
                            lines.push(format!("✓ {name} syntax is valid"));
                        }
                    }
                    problems => {
                        severity = Severity::Error;
                        lines.push(format!("✗ {name}: {}", problems.join("; ")));
                    }
                },
            }
        }

        Ok(CheckResult {
            fixable,
            fix_action: Some(fix_env_gitignore),
            ..self.result(severity, details(lines))
        })
    }
}

/// One env file doctor checks: in the project folder, by name.
struct EnvFile {
    name: String,
    /// A plaintext copy (`.env.bak`, `.env.backup`, …), not the file itself
    /// (DEC-D5). An `evnx backup` is ciphertext and never listed.
    copy: bool,
    /// Where the name points, as the link says, when it is a symlink (DEC-D6).
    link: Option<PathBuf>,
}

impl EnvFile {
    /// The file as a finding names it: a symlink says where it points.
    fn label(&self) -> String {
        match &self.link {
            Some(target) => format!("{} (symlink to {})", self.name, target.display()),
            None => self.name.clone(),
        }
    }
}

/// Every env file in `project_root` that holds, or may hold, real values
/// (`core::env_name::kind`), in name order.
///
/// ⚠️ **By the shared classifier, not `starts_with(".env")`** (I-173, DEC-D3):
/// that took `.envrc` — a direnv script teams commit on purpose — for an env
/// file, reported it unignored as a security error, and its `layout python`
/// line as invalid syntax.
///
/// ⚠️ **Copies are read** (I-160, DEC-D5): every name containing `.backup` was
/// skipped because evnx's backups are ciphertext, so `cp .env .env.backup` was
/// never checked. Now an `evnx backup` is recognised by its content.
///
/// ⚠️ **Symlinks are listed** (I-159, DEC-D6). Only regular files were, so a
/// symlinked `.env` was "not found" here while the permission check followed
/// the link — and `--fix` chmod'ed a target outside the project.
fn env_files(project_root: &Path) -> Vec<EnvFile> {
    let Ok(entries) = fs::read_dir(project_root) else {
        return Vec::new();
    };

    let mut files: Vec<EnvFile> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let kind = env_name::kind(&name);
            if !matches!(kind, FileKind::Env | FileKind::Copy) {
                return None;
            }
            let path = entry.path();
            let link = match entry.file_type().ok()? {
                t if t.is_symlink() => Some(fs::read_link(&path).ok()?),
                t if t.is_file() => None,
                // A directory named `.env` (a Python virtualenv) is not one.
                _ => return None,
            };
            if path.is_dir() {
                return None;
            }
            let copy = kind == FileKind::Copy;
            if copy && is_ciphertext(&path) {
                return None;
            }
            Some(EnvFile { name, copy, link })
        })
        .collect();

    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// Whether `path` holds an `evnx backup`. A file that cannot be read cannot
/// be shown to be ciphertext, so it is checked like a plaintext copy.
fn is_ciphertext(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|content| crate::core::envfile::is_evnx_backup(&content))
        .unwrap_or(false)
}

/// The templates (`.env.example`, `.env.sample`, `.env.template`) present in
/// `project_root`, in that order.
fn templates_in(project_root: &Path) -> Vec<String> {
    env_name::TEMPLATE_SUFFIXES
        .iter()
        .map(|suffix| format!(".env.{suffix}"))
        .filter(|name| project_root.join(name).is_file())
        .collect()
}

/// Whether `project_root` shows a project: a known project type or a Docker
/// file. A folder with neither, and no env file or template, has nothing for
/// doctor to diagnose (DEC-D8).
fn is_a_project(project_root: &Path) -> bool {
    detect_project_type(project_root).is_some()
        || !crate::core::project::docker_files_in(project_root).is_empty()
}

/// The command that creates `.env` from `template` in `project_root`: the one
/// `sync` names for the same case.
fn create_from_template(project_root: &Path, template: &str) -> String {
    use crate::core::shell::word;
    let here = project_root == Path::new(".");
    let mut cmd = String::from("evnx sync --direction reverse");
    if !here {
        cmd.push_str(&format!(
            " --env {}",
            word(&project_root.join(".env").to_string_lossy())
        ));
    }
    if !here || template != ".env.example" {
        let path = if here {
            template.to_string()
        } else {
            project_root.join(template).to_string_lossy().into_owned()
        };
        cmd.push_str(&format!(" --example {}", word(&path)));
    }
    cmd
}

fn fix_env_gitignore(project_root: &Path, _verbose: bool) -> Result<bool> {
    let gitignore_path = project_root.join(".gitignore");

    // Mirrors what `evnx init` writes — one pattern covering every env file, with
    // the committable template family carved back out — through the one
    // `.gitignore` writer (A-28).
    //
    // ⚠️ This asked `git check-ignore ".env*"` — the pattern as a *file name* —
    // so it decided what to add from the wrong question. It now adds the lines
    // the file lacks, and `run_check` asks git again about the real files. A
    // later `!.env` is the user's line and is reported, never edited (G8).
    let added = gitignore::append_entries(&gitignore_path, gitignore::ENV_ENTRIES)?;
    if !added.is_empty() {
        report_fix(format!(".gitignore: added {}", added.join(", ")));
    }
    Ok(!added.is_empty())
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
            return Ok(self.result(
                Severity::Warning,
                Some(".env.example not found - consider creating a template".into()),
            ));
        }

        if !gitignore::is_tracked(project_root, ".env.example") {
            // ⚠️ Not `fixable`. It was, with a `fix_action` that only printed
            // advice and returned `Ok(false)` — so the summary counted a fix
            // that could never happen. Harmless while auto-fix was an
            // undocumented environment variable; a lie once `--fix` is a flag
            // someone runs expecting the count to go down.
            //
            // Staging a file is also not doctor's call. `git add` puts
            // content in the next commit, and a tool that does that
            // unasked is one you stop trusting near a repository.
            Ok(self.result(
                Severity::Warning,
                Some(".env.example is not tracked in Git — run: git add .env.example".into()),
            ))
        } else {
            Ok(self.result(
                Severity::Ok,
                verbose.then(|| "✓ File is tracked in Git".into()),
            ))
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
        let mut lines = Vec::new();
        let mut severity = Severity::Info;
        let mut warnings = 0usize;

        if let Some(project_type) = detected.as_deref() {
            if project_type.starts_with("Python") {
                lines.push(format!("✓ Detected Python project ({})", project_type));
                if project_type.contains("requirements") {
                    if !check_requirements_txt_has_dotenv(project_root)? {
                        lines.push("! python-dotenv not in requirements.txt".into());
                        warnings += 1;
                    } else if verbose {
                        lines.push("✓ python-dotenv dependency found".into());
                    }
                } else if verbose && !check_python_manifest_has_dotenv(project_root)? {
                    lines.push("· Consider adding python-dotenv or pydantic-settings".into());
                }
            } else if project_type == "Node.js" {
                lines.push("✓ Detected Node.js project".into());
                if verbose && !check_package_json_has_dotenv(project_root)? {
                    lines.push("· Consider adding 'dotenv' package".into());
                }
            } else if project_type == "Rust" {
                lines.push("✓ Detected Rust project".into());
                if verbose && !check_cargo_has_dotenv(project_root)? {
                    lines.push("· Consider adding 'dotenvy' crate".into());
                }
            } else {
                lines.push(format!("✓ Detected {} project", project_type));
            }
            severity = Severity::Ok;
        } else {
            lines.push("· No recognized project configuration".into());
            if verbose {
                lines.push("Supported: requirements.txt, pyproject.toml, Pipfile, poetry.lock, package.json, Cargo.toml, go.mod, composer.json".into());
            }
        }

        if warnings > 0 && severity == Severity::Ok {
            severity = Severity::Warning;
        }

        Ok(self.result(severity, details(lines)))
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
        // One list with `validate` (DEC-VA3), which adds Compose v2's
        // `compose.yaml` / `compose.yml` (I-257).
        let found = crate::core::project::docker_files_in(project_root);

        if found.is_empty() {
            Ok(self.result(
                Severity::Info,
                Some("No Docker configuration detected".into()),
            ))
        } else {
            Ok(self.result(
                Severity::Ok,
                verbose.then(|| format!("Found: {}", found.join(", "))),
            ))
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

    /// Every env file and plaintext copy, each named (I-158, DEC-D4); private
    /// means `core::permissions`' rule — no group or other bits — so 700
    /// passes (I-170). A template is never checked: it is meant to be shared.
    fn run(&self, project_root: &Path, verbose: bool) -> Result<CheckResult> {
        let files = env_files(project_root);

        // ⚠️ The reason goes in `details`. It replaced the description, so the
        // JSON `description` changed with the state and the line had no
        // check name (I-166).
        if files.is_empty() {
            return Ok(self.result(Severity::Info, Some("Skipped: no env file to check".into())));
        }
        if !permissions::MODES_APPLY {
            return Ok(self.result(
                Severity::Info,
                Some(
                    "Skipped: Windows platform — permission checks need a Unix-like system".into(),
                ),
            ));
        }

        let root = fs::canonicalize(project_root).ok();
        let mut lines = Vec::new();
        let mut severity = Severity::Ok;
        let mut fixable = false;
        for file in &files {
            let path = project_root.join(&file.name);
            let label = file.label();
            match permissions::mode(&path) {
                None => {
                    severity = Severity::Warning;
                    lines.push(format!("! {label}: could not read its permissions"));
                }
                Some(mode) if permissions::is_private_mode(mode) => {
                    if verbose {
                        lines.push(format!("✓ {label}: permissions {mode:o} (private)"));
                    }
                }
                Some(mode) => {
                    severity = Severity::Warning;
                    lines.push(format!(
                        "! {label}: permissions {mode:o} (recommended: 600)"
                    ));
                    // DEC-D6: a target outside the project is not doctor's to
                    // change; it says what to run instead.
                    if inside(root.as_deref(), &path) {
                        fixable = true;
                    } else {
                        lines.push(format!(
                            "→ the target is outside the project, so --fix leaves it: {}",
                            permissions::chmod_hint(&link_target(project_root, file))
                        ));
                    }
                }
            }
        }

        Ok(CheckResult {
            fixable,
            fix_action: Some(fix_file_permissions),
            ..self.result(severity, details(lines))
        })
    }
}

/// Whether `path`, followed through any symlink, is inside `root` (canonical).
fn inside(root: Option<&Path>, path: &Path) -> bool {
    match (root, fs::canonicalize(path)) {
        (Some(root), Ok(real)) => real.starts_with(root),
        _ => false,
    }
}

/// A symlinked env file's target, as a person would type it from where they
/// ran doctor: the link as written, joined to the project folder if relative.
fn link_target(project_root: &Path, file: &EnvFile) -> PathBuf {
    match &file.link {
        Some(target) if target.is_absolute() || project_root == Path::new(".") => target.clone(),
        Some(target) => project_root.join(target),
        None => project_root.join(&file.name),
    }
}

fn fix_file_permissions(project_root: &Path, _verbose: bool) -> Result<bool> {
    let root = fs::canonicalize(project_root).ok();
    let mut changed = false;
    let mut failed = Vec::new();
    for file in env_files(project_root) {
        let path = project_root.join(&file.name);
        let Some(mode) = permissions::mode(&path) else {
            continue;
        };
        if permissions::is_private_mode(mode) || !inside(root.as_deref(), &path) {
            continue;
        }
        match permissions::make_private(&path) {
            Ok(_) => {
                report_fix(format!("{} mode {mode:o} → 600", file.label()));
                changed = true;
            }
            Err(e) => failed.push(format!("{e:#}")),
        }
    }
    if !failed.is_empty() {
        return Err(anyhow!(failed.join("; ")));
    }
    Ok(changed)
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// `err` as a person reads it: the message without its `(os error N)`.
fn io_cause(err: &std::io::Error) -> String {
    let text = err.to_string();
    match err.raw_os_error() {
        Some(code) => text
            .strip_suffix(&format!(" (os error {code})"))
            .unwrap_or(&text)
            .to_string(),
        None => text,
    }
}

/// `file` in the project as text, or why not as `<cause> (<file>)` — the shape
/// a check that could not run reports (DEC-D9).
fn read_project_file(project_root: &Path, file: &str) -> Result<String> {
    fs::read_to_string(project_root.join(file)).map_err(|e| anyhow!("{} ({file})", io_cause(&e)))
}

/// An env file's text, or a sentence saying why it cannot be read (DEC-D7):
/// a permission problem and a file that is not UTF-8 are different answers.
fn read_text(path: &Path, name: &str) -> std::result::Result<String, String> {
    let bytes =
        fs::read(path).map_err(|e| format!("{name} could not be read: {}", io_cause(&e)))?;
    String::from_utf8(bytes).map_err(|e| format!("{name} is not valid UTF-8: {}", e.utf8_error()))
}

/// Every line of `content` that `core::Parser` would refuse, as messages.
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
///
/// ⚠️ **And it was line-based with no notion of a value spanning lines**, so
/// every continuation line of a valid multiline value was reported as
/// `invalid syntax` — a PEM key produced two or three complaints about a file
/// that `validate`, `convert` and `scan` all accepted. That was bug B6. The
/// multiline state is now tracked with the parser's own `closes_multiline` and
/// `line_opens_multiline`, so the two cannot disagree about where a value ends.
fn syntax_problems(content: &str) -> Vec<String> {
    use crate::core::parser::{closes_multiline, line_opens_multiline};
    use std::sync::OnceLock;

    static LINE: OnceLock<Regex> = OnceLock::new();
    let line_re = LINE.get_or_init(|| {
        Regex::new(r"^(?:\s*$|\s*#.*|\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=.*)$")
            .expect("a valid pattern")
    });

    let mut bad: Vec<String> = Vec::new();
    // The quote a multiline value is open under, and the line it opened on.
    let mut open: Option<(char, usize)> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_num = idx + 1;

        // Inside a multiline value, the following lines are value *content*.
        // They are not entries and cannot be malformed ones.
        if let Some((q, _)) = open {
            if closes_multiline(line, q).is_some() {
                open = None;
            }
            continue;
        }

        if !line_re.is_match(line) {
            let snippet = line.chars().take(50).collect::<String>();
            bad.push(format!(
                "Line {}: invalid syntax '{}{}'",
                line_num,
                snippet,
                if line.chars().count() > 50 { "..." } else { "" }
            ));
            continue;
        }

        open = line_opens_multiline(line).map(|q| (q, line_num));
    }

    // A value whose quote never closes is a genuine error — the parser reports
    // `UnterminatedString` for it. The old line-based check could not represent
    // this at all, so `doctor` called such a file clean while `validate` failed
    // on it: the same disagreement as B6, in the opposite direction.
    if let Some((q, line_num)) = open {
        bad.push(format!(
            "Line {}: unterminated {} value — the quote is never closed",
            line_num,
            if q == '"' { "double-quoted" } else { "quoted" }
        ));
    }

    bad
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
    let content = read_project_file(project_root, "requirements.txt")?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .any(|line| {
            line.starts_with("python-dotenv")
                || (line.starts_with("dotenv") && !line.contains("django-dotenv"))
        }))
}

/// Whether the Python project's manifest lists a dotenv loader (I-175).
///
/// ⛔ Reads the file that defines the type, and only one that exists. This
/// read `pyproject.toml` unconditionally — and before checking `--verbose`,
/// though only verbose output used the answer — so every Pipenv project
/// (`Pipfile`, no `pyproject.toml`) and every `poetry.lock`-only one aborted
/// the whole run: "No such file or directory", exit 2, no verdict. A lock file
/// alone names no direct dependencies, so it is not read: no manifest, no
/// finding.
fn check_python_manifest_has_dotenv(project_root: &Path) -> Result<bool> {
    let mut found = false;
    for manifest in ["pyproject.toml", "Pipfile"] {
        if !project_root.join(manifest).is_file() {
            continue;
        }
        let content = read_project_file(project_root, manifest)?;
        found |= content.contains("python-dotenv")
            || content.contains("pydantic-settings")
            || content.contains("dynaconf");
    }
    Ok(found)
}

fn check_package_json_has_dotenv(project_root: &Path) -> Result<bool> {
    let content = read_project_file(project_root, "package.json")?;
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
    let content = read_project_file(project_root, "Cargo.toml")?;
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
        assert!(syntax_problems("FOO=bar\n# comment\nEMPTY=\n").is_empty());
    }

    #[test]
    fn test_validate_env_syntax_invalid() {
        assert_eq!(
            syntax_problems("INVALID LINE\n"),
            ["Line 1: invalid syntax 'INVALID LINE'"]
        );
    }

    /// DEC-D9: the cause, without the OS's error number, then the file.
    #[test]
    fn t_i163_a_project_file_that_cannot_be_read_names_itself() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("requirements.txt")).unwrap();
        let err = read_project_file(dir.path(), "requirements.txt").unwrap_err();
        let text = err.to_string();
        assert!(text.ends_with(" (requirements.txt)"), "{text}");
        assert!(!text.contains("os error"), "{text}");
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

    fn names(dir: &std::path::Path) -> Vec<String> {
        env_files(dir).into_iter().map(|f| f.name).collect()
    }

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

        let found = names(dir.path());
        assert_eq!(found.len(), 6, "{found:?}");
        assert!(found.contains(&".env.production".to_string()));
        assert!(found.contains(&".env.prod".to_string()));
    }

    /// The template family is meant to be committed, so "not gitignored" is the
    /// correct state for it — flagging it would train people to ignore the check.
    #[test]
    fn leaves_out_the_template_family() {
        let dir = TempDir::new().unwrap();
        for n in [".env", ".env.example", ".env.sample", ".env.template"] {
            fs::write(dir.path().join(n), "A=1\n").unwrap();
        }

        assert_eq!(names(dir.path()), [".env"]);
    }

    /// I-160, DEC-D5 (T-038: this test used to lock the opposite). A
    /// plaintext copy is listed, as a copy; an `evnx backup` is not.
    #[test]
    fn t_i160_reads_plaintext_copies_and_leaves_out_evnx_backups() {
        use base64::{engine::general_purpose, Engine as _};
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".env"), "A=1\n").unwrap();
        fs::write(dir.path().join(".env.backup"), "A=1\n").unwrap();
        let blob = general_purpose::STANDARD.encode([1u8; 120]);
        fs::write(dir.path().join(".env.production.backup"), blob).unwrap();

        let files = env_files(dir.path());
        assert_eq!(
            files
                .iter()
                .map(|f| (f.name.as_str(), f.copy))
                .collect::<Vec<_>>(),
            [(".env", false), (".env.backup", true)]
        );
    }

    /// I-173: names that only start with `.env`.
    #[test]
    fn t_i173_leaves_out_lookalikes() {
        let dir = TempDir::new().unwrap();
        for n in [".env", ".envrc", ".environment", ".env-notes.md"] {
            fs::write(dir.path().join(n), "A=1\n").unwrap();
        }
        assert_eq!(names(dir.path()), [".env"]);
    }

    #[test]
    fn ignores_directories_and_unrelated_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".envoy")).unwrap();
        fs::create_dir(dir.path().join(".env.d")).unwrap();
        fs::write(dir.path().join("README.md"), "x").unwrap();
        fs::write(dir.path().join(".env"), "A=1\n").unwrap();

        assert_eq!(names(dir.path()), [".env"]);
    }

    /// I-159: a symlink is listed with where it points.
    #[cfg(unix)]
    #[test]
    fn t_i159_lists_a_symlink_with_its_target() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("real.env"), "A=1\n").unwrap();
        std::os::unix::fs::symlink("real.env", dir.path().join(".env")).unwrap();

        let files = env_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].label(), ".env (symlink to real.env)");
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
