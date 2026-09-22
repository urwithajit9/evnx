//! `evnx spec init` end to end.
//!
//! Slice 2 of the spec-first work: generating a first contract, so nobody has to
//! write TOML by hand to start. The unit tests cover inference in isolation;
//! these check the command writes a file that evnx can read back.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "DATABASE_URL=postgres://u:pw@h/app\nPORT=8080\nSTRIPE_SECRET_KEY=sk_live_abc\nAPP_NAME=demo\n",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "DATABASE_URL=\nPORT=\nSTRIPE_SECRET_KEY=\nAPP_NAME=\n",
    )
    .unwrap();
    d
}

#[test]
fn stdout_writes_nothing_to_disk() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--stdout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[vars.DATABASE_URL]"));

    assert!(
        !d.path().join(".evnx.toml").exists(),
        "--stdout must not write the config"
    );
}

/// The round trip that matters: what `spec init` writes, `.evnx.toml` reads.
#[test]
fn the_generated_spec_is_read_back_by_the_next_command() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    let written = fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
    assert!(written.contains("[vars.DATABASE_URL]"), "{written}");
    assert!(written.contains("format = \"url\""), "{written}");
    assert!(written.contains("format = \"port\""), "{written}");
    assert!(written.contains("secret = true"), "{written}");

    // If the file did not parse, the config loader would fail the run.
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .success();
}

/// ⚠️ The whole point of appending rather than rewriting: a project's existing
/// config, and its comments, must survive untouched.
#[test]
fn an_existing_config_survives_byte_for_byte() {
    let d = project();
    let before = "# hand-written, do not lose me\n[scan]\nseverity = \"high\"\n";
    fs::write(d.path().join(".evnx.toml"), before).unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    let after = fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
    assert!(
        after.starts_with(before),
        "existing content changed:\n{after}"
    );
}

/// TOML refuses a file that declares the same table twice, so appending a second
/// `[vars]` would produce a config nothing can read. Refuse instead.
#[test]
fn a_second_run_refuses_rather_than_corrupting_the_file() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already has a [vars] section"));

    // And the file is still readable after the refusal.
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .success();
}

#[test]
fn components_carry_their_descriptions_and_attribution() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--with", "stripe", "--stdout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[vars.STRIPE_SECRET_KEY]"))
        .stdout(predicate::str::contains("# from Stripe"))
        .stdout(predicate::str::contains("description ="));
}

#[test]
fn an_unknown_component_is_refused_with_a_suggestion() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--with", "postgres", "--stdout"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("postgresql"));
}

#[test]
fn a_project_with_nothing_to_read_says_so() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--stdout"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("evnx init"));
}
