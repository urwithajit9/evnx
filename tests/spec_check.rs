//! `evnx spec check` (RM-41): the `[vars]` contract against `.env.example`,
//! and `.env` when there is one. Read-only, exit 0/1/2, and shipped as the
//! `evnx-spec-check` pre-commit hook.

mod support;

use support::{run, Project};

const VARS: &str = "[vars.DB_URL]\nformat = \"url\"\n\n[vars.PORT]\nrequired = false\n\n\
                    [vars.STRIPE_KEY]\nenvironments = [\"production\"]\n";

#[test]
fn rm41_a_template_that_matches_passes_without_env() {
    let p = Project::new()
        .toml(VARS)
        .example("DB_URL=postgres://x\nPORT=8080\n");
    let out = run(p.path(), &["spec", "check"]);
    out.assert_code(0);
    assert!(out.stdout.contains("agrees with [vars]"), "{}", out.stdout);
    // A production-only variable may live in its own template.
    assert!(!out.stdout.contains("STRIPE_KEY"), "{}", out.stdout);
    assert!(
        out.stderr.contains("only the template was checked"),
        "{}",
        out.stderr
    );
}

#[test]
fn rm41_drift_either_way_and_a_missing_required_variable_fail() {
    let p = Project::new()
        .toml(VARS)
        .example("PORT=\nEXTRA=\n")
        .env("PORT=1\n");
    let out = run(p.path(), &["spec", "check"]);
    out.assert_code(1);
    assert!(
        out.stdout
            .contains("declared in [vars], missing from .env.example: DB_URL"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("in .env.example, not declared in [vars]: EXTRA"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("required by [vars], missing from .env: DB_URL"),
        "{}",
        out.stdout
    );
}

#[test]
fn rm41_no_contract_is_a_skip_and_no_template_is_no_verdict() {
    let p = Project::new().example("A=\n");
    let out = run(p.path(), &["spec", "check"]);
    out.assert_code(0);
    assert!(out.stderr.contains("no [vars]"), "{}", out.stderr);

    let p = Project::new().toml(VARS);
    run(p.path(), &["spec", "check"]).assert_code(2);

    // A named .env that does not exist is an error, not a skip.
    let p = Project::new().toml(VARS).example("DB_URL=\nPORT=\n");
    run(p.path(), &["spec", "check", "--env", ".env.nope"]).assert_code(2);
}

/// The shipped hook runs `evnx spec check` as written.
#[test]
fn rm41_the_shipped_hook_is_spec_check() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".pre-commit-hooks.yaml"),
    )
    .unwrap();
    let hooks: Vec<serde_yaml::Value> = serde_yaml::from_str(&text).unwrap();
    let hook = hooks
        .iter()
        .find(|h| h["id"] == "evnx-spec-check")
        .expect("no evnx-spec-check hook");
    assert_eq!(hook["entry"].as_str(), Some("evnx spec check"));
    assert_eq!(hook["pass_filenames"].as_bool(), Some(false));
}

/// `-q`: findings only, so a passing check prints nothing.
#[test]
fn rm41_quiet_prints_nothing_when_it_passes() {
    let p = Project::new()
        .toml(VARS)
        .example("DB_URL=postgres://x\nPORT=8080\n")
        .env("DB_URL=postgres://x\n");
    let out = run(p.path(), &["-q", "spec", "check"]);
    out.assert_code(0);
    assert_eq!(out.stdout, "", "{}", out.stdout);
    assert!(!out.stderr.contains("agrees"), "{}", out.stderr);
}
