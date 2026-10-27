//! `evnx doctor`: what it concludes, and how it says so (R-doctor, 0.10.15).
//!
//! - I-161, DEC-D7: a syntax error is an error (exit 1), the verdict
//!   `validate` gives; so is a file doctor cannot read, with its cause.
//! - I-162, DEC-D8: no env file is a verdict of its own — an error beside a
//!   template, a warning in a project without one, and "nothing to diagnose"
//!   (exit 2) in a folder with no project at all.
//! - DEC-D17: `--skip-without-env`, for the pre-commit hook, so a fresh clone
//!   (template, no `.env`) does not fail its first commit.
//! - I-163, DEC-D9: a check that cannot run is reported by name with its
//!   cause, the others finish, and the run exits 2: no complete verdict.
//! - I-166: a skipped check keeps its description; the reason is a detail.
//! - I-167, DEC-D12: `EVNX_AUTO_FIX` says on stderr that it turned repair on.
//! - D-118, D-120: `--help` has Exit codes and Environment; the hook's entry
//!   and description match what fails a commit.

mod support;

use support::{run, run_with, Project};

const IGNORE: &str = ".env*\n!.env.example\n!.env.sample\n!.env.template\n";

/// A project with a tracked template and no `.env`: a fresh clone.
fn project() -> Project {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .example("A=\n")
        .file("package.json", "{}\n");
    p.track(".env.example");
    p
}

fn healthy() -> Project {
    let p = project().env("A=1\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p
}

fn json(out: &support::Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}):\n{}\n{}", out.stdout, out.stderr))
}

fn check<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no {name} check in {report}"))
}

// ── Syntax (I-161, DEC-D7) ───────────────────────────────────────────────────

#[test]
fn t_i161_a_syntax_error_is_an_error() {
    let p = healthy().env("GOOD=1\nthis is not valid\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains("Line 2: invalid syntax");

    let out = run(p.path(), &["doctor", "--format", "json"]);
    out.assert_code(1);
    assert_eq!(check(&json(&out), "env_file")["severity"], "error");
}

#[test]
fn t_dec_d7_a_file_that_is_not_utf8_is_an_error_that_says_so() {
    let p = healthy();
    std::fs::write(p.join(".env"), b"A=\xff\xfe\n").unwrap();

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env is not valid UTF-8");
}

#[cfg(unix)]
#[test]
fn t_dec_d7_an_unreadable_env_file_is_an_error_with_its_cause() {
    let p = healthy().mode(".env", 0o000);
    if std::fs::read(p.join(".env")).is_ok() {
        return; // root reads anything; the case cannot be built
    }

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env could not be read: Permission denied");
    // The other checks still ran.
    out.assert_contains("Check for Docker configuration files");
}

// ── No env file (I-162, DEC-D8) ──────────────────────────────────────────────

#[test]
fn t_i162_a_template_without_env_is_an_error() {
    let p = project();

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env not found");
    out.assert_contains("evnx sync --direction reverse");
}

#[test]
fn t_i162_an_empty_folder_has_nothing_to_diagnose() {
    let p = Project::new();

    let out = run(p.path(), &["doctor"]);
    out.assert_code(2);
    out.assert_contains("nothing to diagnose");
    out.assert_contains("No verdict");

    let out = run(p.path(), &["doctor", "--format", "json"]);
    out.assert_code(2);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

#[test]
fn t_i162_a_project_without_env_or_template_is_a_warning() {
    let p = Project::new().file("package.json", "{}\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_contains(".env not found");
    run(p.path(), &["doctor", "--strict"]).assert_code(1);
}

// ── --skip-without-env (DEC-D17) ─────────────────────────────────────────────

#[test]
fn t_dec_d17_skip_without_env_passes_a_fresh_clone() {
    let p = project();

    let out = run(p.path(), &["doctor", "--skip-without-env"]);
    out.assert_code(0);
    assert!(
        out.stderr.contains("nothing to check, skipped"),
        "{}",
        out.stderr
    );

    // A repository with no env setup at all, as a hook may run in.
    run(Project::new().path(), &["doctor", "--skip-without-env"]).assert_code(0);
}

#[test]
fn t_dec_d17_skip_without_env_changes_nothing_when_an_env_file_exists() {
    let p = healthy().file(".gitignore", "node_modules/\n");
    run(p.path(), &["doctor", "--skip-without-env"]).assert_code(1);
}

// ── A check that cannot run (I-163, DEC-D9) ──────────────────────────────────

#[test]
fn t_i163_a_check_that_cannot_run_is_named_and_the_rest_finish() {
    let p = healthy().dir("requirements.txt");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(2);
    out.assert_contains("could not run");
    out.assert_contains("(requirements.txt)");
    // A later check still ran.
    out.assert_contains("Check file permissions for sensitive files");
    out.assert_contains("No complete verdict");
}

#[test]
fn t_i163_json_still_prints_and_marks_the_check_as_not_run() {
    let p = healthy().dir("requirements.txt");

    let out = run(p.path(), &["doctor", "--format", "json"]);
    out.assert_code(2);
    let report = json(&out);
    // Six since RM-25 added ai_agents.
    assert_eq!(report["checks"].as_array().unwrap().len(), 6);
    let failed = check(&report, "project_structure");
    assert_eq!(failed["ran"], false);
    assert_eq!(failed["severity"], "error");
    for name in [
        "env_file",
        "env_example",
        "docker_config",
        "ai_agents",
        "file_permissions",
    ] {
        assert_eq!(check(&report, name)["ran"], true, "{name}");
    }
}

#[test]
fn t_i163_both_renderers_agree_on_a_check_that_cannot_run() {
    support::renderers::both_renderers(
        || healthy().dir("requirements.txt"),
        &["doctor"],
        &["doctor", "--format", "json"],
    );
}

/// The `.gitignore` cannot be written: `--fix` says so and why, and the check
/// stays failing (DEC-D1 / 7.2).
#[cfg(unix)]
#[test]
fn t_dec_d9_a_fix_that_cannot_write_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let p = healthy().file(".gitignore", "node_modules/\n");
    let lock = |m| std::fs::set_permissions(p.path(), std::fs::Permissions::from_mode(m));
    lock(0o555).unwrap();
    let writable = std::fs::write(p.join("probe"), "").is_ok();

    let out = run(p.path(), &["doctor", "--fix"]);
    lock(0o755).unwrap();
    if writable {
        return; // root writes anywhere; the case cannot be built
    }

    out.assert_code(1);
    out.assert_contains("not fixed");
    out.assert_contains("Permission denied");
}

// ── I-166 ────────────────────────────────────────────────────────────────────

#[test]
fn t_i166_a_skipped_check_keeps_its_description() {
    let p = Project::new().file("package.json", "{}\n");

    let out = run(p.path(), &["doctor", "--format", "json"]);
    let perms = check(&json(&out), "file_permissions").clone();
    assert_eq!(
        perms["description"],
        "Check file permissions for sensitive files"
    );
    assert!(
        perms["details"]
            .as_str()
            .unwrap_or_default()
            .contains("Skipped"),
        "{perms}"
    );
}

// ── I-167, DEC-D12 ───────────────────────────────────────────────────────────

#[test]
fn t_i167_auto_fix_from_the_environment_says_so() {
    let p = healthy().file(".gitignore", "node_modules/\n");
    let out = run_with(p.path(), &["doctor"], &[("EVNX_AUTO_FIX", "1")], None);
    assert!(
        out.stderr.contains("auto-fix enabled by EVNX_AUTO_FIX"),
        "{}",
        out.stderr
    );

    let p = healthy().file(".gitignore", "node_modules/\n");
    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_not_contains("auto-fix enabled");
}

// ── Help and hook (D-118, D-120) ─────────────────────────────────────────────

#[test]
fn t_d118_help_names_exit_codes_and_environment() {
    let p = Project::new();
    let out = run(p.path(), &["doctor", "--help"]);
    out.assert_code(0);
    for needle in [
        "Exit codes",
        "Environment",
        "EVNX_AUTO_FIX",
        "EVNX_OUTPUT_JSON",
        "--skip-without-env",
    ] {
        out.assert_contains(needle);
    }
}

#[test]
fn t_d120_the_hook_skips_a_fresh_clone_and_says_what_fails() {
    let hooks = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.pre-commit-hooks.yaml"
    ))
    .unwrap();
    let block: String = hooks
        .split("\n- id: ")
        .find(|b| b.starts_with("evnx-doctor\n"))
        .expect("an evnx-doctor hook")
        .to_string();

    assert!(
        block.contains("\n  entry: evnx doctor --skip-without-env\n"),
        "{block}"
    );
    let description = block
        .lines()
        .find(|l| l.trim_start().starts_with("description:"))
        .unwrap_or_default();
    assert!(description.contains("warnings"), "{description}");
}
