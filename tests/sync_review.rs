//! Found reviewing R-sync (#206) before the 0.10.12 release (I-279, I-281,
//! I-282, I-283). The terminal cases (I-277, I-278, I-280) are in
//! `sync_pty.rs`.

mod support;

use support::{run, Project};

/// I-279: with no `.env`, `--check` returned before loading
/// `--template-config`, so a misspelt key or a missing file passed in CI
/// (exit 0) and failed locally (exit 2) — the same command, two answers.
#[test]
fn t_i279_template_config_is_checked_without_env() {
    let p = Project::new()
        .example("A=\n")
        .toml("[vars.A]\n")
        .file("tc.json", r#"{"allow_actaul":[]}"#);
    run(
        p.path(),
        &["sync", "--check", "--template-config", "tc.json"],
    )
    .assert_code(2)
    .assert_contains("allow_actaul");
    run(
        p.path(),
        &["sync", "--check", "--template-config", "missing.json"],
    )
    .assert_code(2);
    run(
        p.path(),
        &[
            "sync",
            "--check",
            "--skip-without-env",
            "--template-config",
            "missing.json",
        ],
    )
    .assert_code(2);
}

/// I-281: a preview does not say it is creating anything.
#[test]
fn t_i281_check_and_dry_run_never_say_creating() {
    let p = Project::new().env("A=1\n");
    for args in [&["sync", "--check"][..], &["sync", "-n"][..]] {
        let out = run(p.path(), args);
        assert!(!out.all().contains("creating"), "{args:?}:\n{}", out.all());
    }
    assert!(!p.exists(".env.example"));
    let p = Project::new().example("A=\n");
    let out = run(p.path(), &["sync", "-n", "--direction", "reverse"]);
    assert!(!out.all().contains("creating"), "{}", out.all());
    assert!(!p.exists(".env"));
}

/// I-282: every command a message suggests names the files in play.
#[test]
fn t_i282_hints_carry_the_files_in_play() {
    // The empty-source warning.
    let p = Project::new().file("e.env", "").example("A=\n");
    run(p.path(), &["sync", "--env", "e.env"])
        .assert_code(0)
        .assert_contains("evnx sync --direction reverse --env e.env");
    // No `.env` and no spec under --check.
    let p = Project::new().example("A=\n");
    run(p.path(), &["sync", "--check", "--env", "e.env"])
        .assert_code(2)
        .assert_contains("evnx sync --direction reverse --env e.env");
}

/// I-283: only the two lines sync writes are replaced in a new `.env`; a
/// first line of the user's own that starts the same way is kept.
#[test]
fn t_i283_own_generated_from_line_is_kept() {
    let p = Project::new().example("# Generated from terraform output\nA=1\n");
    run(p.path(), &["sync", "-f", "--direction", "reverse"]).assert_code(0);
    let env = p.read(".env");
    assert!(
        env.starts_with("# Generated from terraform output\n"),
        "{env}"
    );
}
