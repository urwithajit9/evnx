//! `sync --check` where there is no `.env` (U8d: DEC-S4 / DEC-U8-6, I-128 / DEC-U8-5).
//!
//! CI and a fresh clone have a template and no `.env`, so forward `--check`
//! exited 2 "file not found" — a CI gate that could never pass, and the
//! shipped `evnx-sync` pre-commit hook blocked every new contributor.
//!
//! - DEC-S4: with no `.env`, the template is checked against the spec — each
//!   required `[vars]` name must be declared in it (exit 1 names the ones that
//!   are not). With no spec either, there is nothing to compare: exit 2.
//! - DEC-U8-5: `--skip-without-env` turns "nothing to compare" into a skip
//!   that says so (exit 0). The hooks use it; plain `--check` does not, so CI
//!   still cannot pass on nothing.
//! - DEC-S15: the `evnx-sync` hook runs `sync --check` (it needs no terminal),
//!   and `evnx-sync-fix` is the optional writing variant.

mod support;

use support::{run, Project};

const SPEC: &str = "[vars.APP]\n[vars.DB_URL]\n[vars.SENTRY_DSN]\nrequired = false\n";

#[test]
fn t_dec_s4_the_template_is_checked_against_the_spec() {
    let p = Project::new()
        .example("APP=\nDB_URL=\n")
        .file(".evnx.toml", SPEC);
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(0);
    assert!(
        out.all().contains("spec"),
        "says what it compared:\n{}",
        out.all()
    );
}

#[test]
fn t_dec_s4_a_required_name_missing_from_the_template_is_drift() {
    let p = Project::new().example("APP=\n").file(".evnx.toml", SPEC);
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(1);
    assert!(out.all().contains("DB_URL"), "{}", out.all());
    // Optional names are not demanded.
    assert!(!out.all().contains("SENTRY_DSN"), "{}", out.all());
}

#[test]
fn t_dec_s4_json_reports_the_spec_comparison() {
    let p = Project::new().example("APP=\n").file(".evnx.toml", SPEC);
    let out = run(p.path(), &["sync", "--check", "--format", "json"]);
    out.assert_code(1);
    let j: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(j["missing"], serde_json::json!(["DB_URL"]));
    assert_eq!(j["compared_with"], "spec");
    assert_eq!(j["in_sync"], false);
}

#[test]
fn t_dec_s4_nothing_to_compare_is_no_verdict() {
    let p = Project::new().example("APP=\n");
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(2);
    assert!(out.all().contains("nothing to compare"), "{}", out.all());
}

#[test]
fn t_i128_skip_without_env_skips_and_says_so() {
    let p = Project::new().example("APP=\n");
    let out = run(p.path(), &["sync", "--check", "--skip-without-env"]);
    out.assert_code(0);
    assert!(out.stderr.contains("skipped"), "{}", out.stderr);
}

/// The skip is for "nothing to compare" only: a spec is still checked.
#[test]
fn t_i128_skip_without_env_still_checks_the_spec() {
    let p = Project::new().example("APP=\n").file(".evnx.toml", SPEC);
    let out = run(p.path(), &["sync", "--check", "--skip-without-env"]);
    out.assert_code(1);
    assert!(out.all().contains("DB_URL"), "{}", out.all());
}

/// With a `.env`, the flag changes nothing.
#[test]
fn t_i128_skip_without_env_is_inert_when_env_exists() {
    let p = Project::new().env("APP=1\nNEW=2\n").example("APP=\n");
    let out = run(p.path(), &["sync", "--check", "--skip-without-env"]);
    out.assert_code(1);
}

/// Reverse creates `.env`; a skip there would hide that it was never created.
#[test]
fn t_i128_skip_without_env_is_forward_only() {
    let p = Project::new().example("APP=\n");
    let out = run(
        p.path(),
        &["sync", "--direction", "reverse", "--skip-without-env", "-f"],
    );
    out.assert_code(2);
    assert!(out.all().contains("forward"), "{}", out.all());
}

/// The hook's entry, read from the shipped `.pre-commit-hooks.yaml`.
fn hook_args(id: &str) -> Vec<String> {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".pre-commit-hooks.yaml"),
    )
    .unwrap();
    let hooks: Vec<serde_yaml::Value> = serde_yaml::from_str(&text).unwrap();
    let hook = hooks
        .iter()
        .find(|h| h["id"] == id)
        .unwrap_or_else(|| panic!("no hook {id}"));
    let entry = hook["entry"].as_str().unwrap();
    let mut words = entry.split_whitespace();
    assert_eq!(words.next(), Some("evnx"), "{entry}");
    words.map(String::from).collect()
}

fn run_hook(p: &Project, id: &str) -> support::Output {
    let args = hook_args(id);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(p.path(), &args)
}

/// T-SYNC-51. The shipped hook, with no terminal: a fresh clone is not
/// blocked; drift blocks with the command that fixes it, not "not a terminal".
#[test]
fn t_i128_the_shipped_hook_works_without_a_terminal() {
    let fresh = Project::new().example("APP=\n");
    run_hook(&fresh, "evnx-sync").assert_code(0);

    let drift = Project::new().env("APP=1\nNEW=2\n").example("APP=\n");
    let out = run_hook(&drift, "evnx-sync");
    out.assert_code(1);
    assert!(out.all().contains("evnx sync"), "{}", out.all());
    assert!(!out.all().contains("terminal"), "{}", out.all());
}

#[test]
fn t_i128_the_fix_hook_updates_the_template() {
    let fresh = Project::new().example("APP=\n");
    run_hook(&fresh, "evnx-sync-fix").assert_code(0);

    let drift = Project::new().env("APP=1\nNEW=2\n").example("APP=\n");
    run_hook(&drift, "evnx-sync-fix").assert_code(0);
    let example = std::fs::read_to_string(drift.path().join(".env.example")).unwrap();
    assert!(example.lines().any(|l| l.starts_with("NEW=")), "{example}");
}
