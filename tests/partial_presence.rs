//! Partial presence (N14-12, plan §9 phase 3): `validate` and `spec check`
//! report a name some environment files hold and others lack. A warning, so
//! the exit code is unchanged; it needs two environment files, so a project
//! with one sees nothing new.

mod support;

use support::{run, Project};

fn evnx(p: &Project, args: &[&str]) -> support::run::Output {
    run(p.path(), args)
}

fn json(p: &Project, args: &[&str]) -> serde_json::Value {
    let out = evnx(p, args);
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

fn partial(doc: &serde_json::Value) -> Vec<&serde_json::Value> {
    doc["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "partial_presence")
        .collect()
}

/// `.env` and `.env.production` have EXTRA; `.env.staging` does not.
fn three() -> Project {
    Project::new()
        .env("A=1\nEXTRA=2\n")
        .file(".env.staging", "A=1\n")
        .file(".env.production", "A=1\nEXTRA=2\n")
        .example("A=\n")
}

#[test]
fn validate_warns_with_both_sides_named_and_exit_stays_0() {
    let p = three();
    let doc = json(&p, &["validate", "--format", "json"]);
    let found = partial(&doc);
    assert_eq!(found.len(), 1, "{doc:#}");
    let i = found[0];
    assert_eq!(i["severity"], "warning");
    assert_eq!(i["variable"], "EXTRA");
    let msg = i["message"].as_str().unwrap();
    assert!(
        msg.contains(".env.production") && msg.contains("not in .env.staging"),
        "{msg}"
    );
    assert_eq!(doc["summary"]["errors"], 0);
    evnx(&p, &["validate"]).assert_code(0);
}

#[test]
fn it_is_reported_from_the_absent_side_too() {
    let p = three();
    let doc = json(
        &p,
        &["validate", "--env-name", "staging", "--format", "json"],
    );
    let found = partial(&doc);
    assert_eq!(found.len(), 1, "{doc:#}");
    assert_eq!(found[0]["variable"], "EXTRA");
    assert!(found[0]["suggestion"]
        .as_str()
        .unwrap()
        .contains("evnx remove EXTRA --all"));
}

#[test]
fn one_environment_file_adds_no_finding() {
    let p = Project::new().env("A=1\nONLY_HERE=2\n").example("A=\n");
    let doc = json(&p, &["validate", "--format", "json"]);
    assert!(partial(&doc).is_empty(), "{doc:#}");
}

#[test]
fn the_template_and_backups_are_not_environments() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\nB=\n")
        .file(".env.bak", "A=1\nZ=1\n")
        .file(".env.backup", "A=1\nZ=1\n");
    let doc = json(&p, &["validate", "--format", "json"]);
    assert!(partial(&doc).is_empty(), "{doc:#}");
}

#[test]
fn ignore_and_config_ignore_silence_it() {
    let p = three();
    let doc = json(
        &p,
        &[
            "validate",
            "--ignore",
            "partial_presence",
            "--format",
            "json",
        ],
    );
    assert!(partial(&doc).is_empty());
    let p = three().toml("[validate]\nignore = [\"partial_presence\"]\n");
    let doc = json(&p, &["validate", "--format", "json"]);
    assert!(partial(&doc).is_empty());
}

#[test]
fn fail_on_warning_makes_it_fail_the_run() {
    let p = three();
    evnx(&p, &["validate", "--fail-on", "warning"]).assert_code(1);
}

#[test]
fn a_name_missing_here_is_not_reported_twice() {
    // A is in the template, so .env.staging lacking it is already
    // `missing_variable`; partial presence stays quiet about that one.
    let p = Project::new()
        .env("A=1\n")
        .file(".env.staging", "")
        .example("A=\n");
    let doc = json(
        &p,
        &["validate", "--env-name", "staging", "--format", "json"],
    );
    assert!(partial(&doc).is_empty(), "{doc:#}");
    assert!(doc["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["type"] == "missing_variable"));
}

#[test]
fn the_contract_excuses_optional_and_other_environment_names() {
    let p = three().toml("[vars.A]\nrequired = true\n\n[vars.EXTRA]\nrequired = false\n");
    let doc = json(&p, &["validate", "--format", "json"]);
    assert!(partial(&doc).is_empty(), "{doc:#}");
    let p = three()
        .toml("[vars.A]\nrequired = true\n\n[vars.EXTRA]\nenvironments = [\"production\"]\n");
    let doc = json(&p, &["validate", "--format", "json"]);
    assert!(partial(&doc).is_empty(), "{doc:#}");
}

#[test]
fn stdin_input_is_not_compared() {
    let p = three();
    let out = support::run_with(
        p.path(),
        &["validate", "--env", "-", "--format", "json"],
        &[],
        Some(b"A=1\nEXTRA=2\n"),
    );
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(partial(&doc).is_empty(), "{doc:#}");
}

#[test]
fn spec_check_warns_and_keeps_its_exit_code() {
    let p = Project::new()
        .env("A=1\nEXTRA=2\n")
        .file(".env.staging", "A=1\n")
        .example("A=\nEXTRA=\n")
        .toml("[vars.A]\nrequired = true\n\n[vars.EXTRA]\nrequired = true\n");
    // .env.staging lacks a required EXTRA; the checked .env is complete.
    let out = evnx(&p, &["spec", "check"]);
    out.assert_code(0)
        .assert_contains("EXTRA is in .env but not in .env.staging");
}

#[test]
fn spec_check_with_one_env_file_is_unchanged() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\n")
        .toml("[vars.A]\nrequired = true\n");
    evnx(&p, &["spec", "check"])
        .assert_code(0)
        .assert_not_contains("partial_presence");
}

#[test]
fn the_ignore_list_names_the_new_type() {
    let p = three();
    evnx(&p, &["validate", "--ignore", "bogus_type"])
        .assert_code(2)
        .assert_contains("partial_presence");
}
