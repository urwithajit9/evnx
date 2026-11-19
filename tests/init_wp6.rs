//! WP-6 (v0.14.2): `init` for projects that already have files. The adopt
//! flow itself is in `layouts.rs`, beside the named-only layout it acts on.
//!
//! - INIT-1: a live-looking credential in the template is said, by name.
//! - INIT-4: `--no-contract` writes no `.evnx.toml`.
//! - GEN-20: a generated template never turns debugging on.

mod support;

use support::{Project, run};

#[test]
fn init1_a_real_looking_key_in_the_template_is_named_never_shown() {
    let key = "sk_live_51HxR8aK2mQ7xL4vR8tZ1wB6nC3hJ9pD";
    let p = Project::new().git().example(&format!(
        "STRIPE_SECRET_KEY={key}\nAPI_KEY=your-api-key-here\nPORT=8080\n"
    ));
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0);
    out.assert_contains("looks like a real credential: STRIPE_SECRET_KEY");
    out.assert_contains("rotate it");
    out.assert_not_contains(key);
    out.assert_not_contains("API_KEY.");
}

#[test]
fn init1_placeholders_alone_say_nothing() {
    let p = Project::new()
        .git()
        .example("API_KEY=your-api-key-here\nDATABASE_URL=postgresql://user:password@localhost:5432/dbname\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_not_contains("real credential");
}

#[test]
fn init4_no_contract_writes_no_evnx_toml() {
    let p = Project::new().git().example("PORT=8080\nAPI_KEY=\n");
    let out = run(p.path(), &["init", "--yes", "--no-contract"]);
    out.assert_code(0);
    out.assert_not_contains(".evnx.toml");
    assert!(!p.exists(".evnx.toml"));
    assert!(p.exists(".env"));

    let out = run(p.path(), &["init", "--yes", "--no-contract", "--dry-run"]);
    out.assert_contains(".evnx.toml: no contract (--no-contract)");
}

#[test]
fn init4_the_next_steps_say_what_the_contract_is() {
    let p = Project::new().git().example("PORT=8080\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains(".evnx.toml is the contract 'evnx validate' checks: safe to delete");
}

#[test]
fn init4_adoption_with_no_contract_still_sets_the_default() {
    let p = Project::new()
        .git()
        .file(".env.prod", "A=1\n")
        .file(".env.prod.example", "A=\n");
    run(p.path(), &["init", "--yes", "--no-contract"]).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains("env_name = \"prod\""), "{toml}");
    assert!(!toml.contains("[vars"), "{toml}");
}

#[test]
fn gen20_no_generated_template_turns_debugging_on() {
    for stack in ["django", "laravel"] {
        let p = Project::new().git();
        run(p.path(), &["init", "--yes", "--with", stack]).assert_code(0);
        let t = p.read(".env.example");
        for line in t.lines().filter(|l| l.contains("DEBUG=")) {
            assert!(
                line.ends_with("=False") || line.ends_with("=false"),
                "{stack}: {line}"
            );
        }
    }
}

#[test]
fn gen7_an_empty_start_reports_the_template_first_and_says_how_to_add() {
    let p = Project::new().git();
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0);
    let all = out.all();
    let template = all.find("Created .env.example").expect(&all);
    let env = all.find("Created .env from .env.example").expect(&all);
    assert!(template < env, "template first:\n{all}");
    out.assert_contains("Add variables: evnx add custom NAME=value");
    out.assert_not_contains("replace placeholder values");
}

#[test]
fn init7_a_refusal_says_nothing_was_changed_not_no_verdict() {
    let p = Project::new().git();
    let out = run(p.path(), &["init"]);
    // Exit 4 since 0.15.0 (WP-9, D3): the menu cannot be asked without a terminal.
    out.assert_code(4);
    out.assert_contains("evnx stopped before making changes (exit 4)");
    out.assert_not_contains("No verdict");
}

#[test]
fn sync5_reverse_sync_creates_env_from_the_template() {
    let p = Project::new().git().example("PORT=8080\n");
    run(p.path(), &["sync", "--direction", "reverse", "--yes"]).assert_code(0);
    assert!(p.exists(".env"));
}

// ── SPEC-3: spec check for machines ─────────────────────────────────────────

fn contract() -> Project {
    Project::new()
        .git()
        .example("PORT=8080\nAPI_KEY=\nEXTRA=\n")
        .toml("[vars.PORT]\nrequired = false\nformat = \"port\"\n[vars.API_KEY]\nrequired = true\n[vars.GONE]\nrequired = true\n")
        .env("PORT=99999\n")
}

#[test]
fn spec3_check_json_lists_every_problem_by_name() {
    let p = contract();
    let out = run(p.path(), &["spec", "check", "--format", "json"]);
    out.assert_code(1);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["not_in_template"], serde_json::json!(["GONE"]));
    assert_eq!(doc["undeclared"], serde_json::json!(["EXTRA"]));
    assert_eq!(
        doc["missing_from_env"],
        serde_json::json!(["API_KEY", "GONE"])
    );
    assert_eq!(doc["format"][0]["name"], "PORT");
    assert!(!out.stdout.contains("99999"), "a value in the JSON");
}

#[test]
fn spec3_skip_without_env_skips_a_named_missing_file() {
    let p = Project::new()
        .git()
        .example("PORT=8080\n")
        .toml("[vars.PORT]\nrequired = false\n");
    run(p.path(), &["spec", "check", "--env-name", "prod"]).assert_code(2);
    let out = run(
        p.path(),
        &[
            "spec",
            "check",
            "--env-name",
            "prod",
            "--skip-without-env",
            "--format",
            "json",
        ],
    );
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["env"], serde_json::Value::Null);
    assert_eq!(doc["ok"], true);
}

#[test]
fn spec3_without_vars_json_says_it_was_skipped() {
    let p = Project::new().git().example("PORT=8080\n");
    let out = run(p.path(), &["spec", "check", "--format", "json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["checked"], false);
}
