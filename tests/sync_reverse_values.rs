//! What `sync --direction reverse` puts into `.env` (U8d, I-115, DEC-S2).
//!
//! Reverse regenerated every missing value from its name and ignored
//! `--template-config`: `REDIS_URL=redis://localhost:6379` came back as
//! `…/0`, `API_PORT=8080` as `8000`, `LOG_LEVEL=info` as `YOUR_VALUE_HERE`.
//! The template is the project's committed answer for a new teammate; the
//! decision is to copy it, and to generate a placeholder — by the project's
//! config — only where the template has none.

mod support;

use support::{run, Project};

fn env_of(p: &Project) -> String {
    std::fs::read_to_string(p.path().join(".env")).unwrap()
}

#[test]
fn t_i115_reverse_keeps_the_templates_values() {
    let p = Project::new()
        .env("APP=1\n")
        .example("APP=\nREDIS_URL=redis://localhost:6379\nAPI_PORT=8080\nLOG_LEVEL=info\n");
    run(p.path(), &["sync", "--direction", "reverse", "-f"]).assert_code(0);
    let env = env_of(&p);
    for line in [
        "REDIS_URL=redis://localhost:6379",
        "API_PORT=8080",
        "LOG_LEVEL=info",
    ] {
        assert!(env.lines().any(|l| l == line), "{line} missing:\n{env}");
    }
}

/// T-SYNC-35.
#[test]
fn t_i115_reverse_with_allow_actual_keeps_the_value() {
    let p = Project::new()
        .env("APP=1\n")
        .example("APP=\nLOG_LEVEL=info\n")
        .file("tc.json", r#"{"allow_actual":["LOG_LEVEL"]}"#);
    run(
        p.path(),
        &[
            "sync",
            "--direction",
            "reverse",
            "-f",
            "--template-config",
            "tc.json",
        ],
    )
    .assert_code(0);
    assert!(
        env_of(&p).lines().any(|l| l == "LOG_LEVEL=info"),
        "{}",
        env_of(&p)
    );
}

/// Where the template has no value, the project's patterns and default decide.
#[test]
fn t_i115_reverse_honours_the_template_config() {
    let p = Project::new()
        .env("APP=1\n")
        .example("APP=\nCUSTOM_FLAG=\nOTHER=\n")
        .file(
            "tc.json",
            r#"{"patterns": {"CUSTOM_.*": "custom_val"}, "default": "MY_DEFAULT"}"#,
        );
    run(
        p.path(),
        &[
            "sync",
            "--direction",
            "reverse",
            "-f",
            "--template-config",
            "tc.json",
        ],
    )
    .assert_code(0);
    let env = env_of(&p);
    assert!(env.lines().any(|l| l == "CUSTOM_FLAG=custom_val"), "{env}");
    assert!(env.lines().any(|l| l == "OTHER=MY_DEFAULT"), "{env}");
}

/// A placeholder in the template is still a placeholder in `.env`, by the
/// same rule as before: nothing real is invented.
#[test]
fn t_i115_a_template_placeholder_stays_a_placeholder() {
    let p = Project::new()
        .env("APP=1\n")
        .example("APP=\nSTRIPE_SECRET_KEY=YOUR_KEY_HERE\nDB_PASSWORD=\n");
    run(p.path(), &["sync", "--direction", "reverse", "-f"]).assert_code(0);
    let env = env_of(&p);
    assert!(
        env.lines().any(|l| l == "STRIPE_SECRET_KEY=YOUR_KEY_HERE"),
        "{env}"
    );
    assert!(
        env.lines().any(|l| l == "DB_PASSWORD=YOUR_PASSWORD_HERE"),
        "{env}"
    );
}

/// `--dry-run` shows what would be written, so it must show the same values.
#[test]
fn t_i115_the_preview_shows_the_templates_values() {
    let p = Project::new()
        .env("APP=1\n")
        .example("APP=\nAPI_PORT=8080\n");
    let out = run(p.path(), &["sync", "--direction", "reverse", "--dry-run"]);
    assert!(out.all().contains("8080"), "{}", out.all());
    assert!(!out.all().contains("8000"), "{}", out.all());
}
