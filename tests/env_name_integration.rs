//! `--env-name` across the commands that take an env file.
//!
//! The behaviour that matters is the same everywhere: a name resolves to
//! `.env.<name>`, and a name that resolves to nothing is an **error** rather
//! than a quiet retreat to `.env`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".env.example"), "A=\nB=\n").unwrap();
    fs::write(dir.path().join(".env"), "A=dev\nB=dev\n").unwrap();
    fs::write(dir.path().join(".env.production"), "A=prod\nB=prod\n").unwrap();
    fs::write(dir.path().join(".env.staging"), "A=stg\nB=stg\n").unwrap();
    dir
}

#[test]
fn convert_reads_the_named_environment() {
    let dir = project();

    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["convert", "--env-name", "production", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("\"A\": \"prod\""), "{stdout}");
    assert!(!stdout.contains("dev"), "it must not read .env: {stdout}");
}

#[test]
fn diff_can_compare_two_real_environments() {
    let dir = project();
    fs::write(
        dir.path().join(".env.production"),
        "A=prod\nB=prod\nONLY_PROD=1\n",
    )
    .unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["diff", "--env-name", "production", "--against", "staging"])
        .assert()
        .stdout(predicate::str::contains(".env.production"))
        .stdout(predicate::str::contains(".env.staging"))
        .stdout(predicate::str::contains("ONLY_PROD"));
}

/// ⚠️ The property the whole resolver exists for.
#[test]
fn a_missing_environment_never_falls_back_to_dot_env() {
    let dir = project();

    for args in [
        vec!["convert", "--env-name", "nope", "--to", "json"],
        vec!["validate", "--env-name", "nope"],
        vec![
            "template",
            "--env-name",
            "nope",
            "--input",
            "x",
            "--output",
            "y",
        ],
    ] {
        cargo_bin_cmd!("evnx")
            .current_dir(dir.path())
            .args(&args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("does not exist"));
    }
}

#[test]
fn an_abbreviated_name_is_guessed() {
    let dir = project();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["convert", "--env-name", "prod", "--to", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("did you mean 'production'"));
}

/// `--env` and `--env-name` answer the same question, so clap refuses both.
#[test]
fn env_and_env_name_conflict() {
    let dir = project();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args([
            "convert",
            "--env",
            ".env",
            "--env-name",
            "production",
            "--to",
            "json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

/// `--pattern` was a weaker duplicate of `--env`: anything not starting `.env.`
/// printed a notice and silently validated `.env` instead, exit 0.
#[test]
fn validate_pattern_is_gone() {
    let dir = project();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["validate", "--pattern", ".env.production"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn defaults_are_unchanged_without_the_flag() {
    let dir = project();

    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["convert", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("dev"),
        "the default is still .env: {stdout}"
    );
}

// ── A name that resolves to nothing is "could not run", not a finding ────────

/// ⚠️ This exited **1** before v0.5.0, in every command below.
///
/// `main` called `env_name::select(...)?`, and `?` surfaces the error through
/// `main`'s `Result`, which Rust's default `Termination` numbers 1. But 1 is
/// already spoken for: `validate` uses it for *invalid*, `diff` for *differences
/// found*, `sync --check` for *drifted*. So a typo in a CI flag reported a
/// finding rather than a broken setup, and the pipeline branched on an answer
/// that was never computed.
///
/// `scan` is absent from this list only because it has no `--env-name` to
/// mistype — it exits 2 from clap's own argument error.
#[test]
fn an_env_name_that_resolves_to_nothing_exits_2_everywhere() {
    let dir = project();

    for args in [
        vec!["validate", "--env-name", "nosuch"],
        vec!["diff", "--env-name", "nosuch"],
        vec!["sync", "--check", "--env-name", "nosuch"],
        vec!["convert", "--to", "json", "--env-name", "nosuch"],
        vec![
            "template",
            "--input",
            ".env.example",
            "--output",
            "out.txt",
            "--env-name",
            "nosuch",
        ],
    ] {
        let assert = cargo_bin_cmd!("evnx")
            .current_dir(dir.path())
            .args(&args)
            .assert()
            .code(2);
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
        assert!(
            stderr.contains("No verdict"),
            "`evnx {}` must not read as a result: {stderr}",
            args.join(" ")
        );
        assert!(
            stderr.contains("nosuch"),
            "the error should name what could not be resolved: {stderr}"
        );
    }
}

/// The complement: a name that *does* resolve must still reach the command, so
/// the guard above cannot be satisfied by refusing everything.
#[test]
fn a_name_that_resolves_still_runs_the_command() {
    let dir = project();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["convert", "--to", "json", "--env-name", "production"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("prod"));
}

/// ⚠️ A file evnx cannot read is "could not run", not "validation failed".
///
/// `validate` exits 1 when it finds errors. Before v0.5.0 it also exited 1 when
/// the file did not exist, because `main` returned the command's `Result` and
/// Rust's default `Termination` numbers any error 1 — so a CI gate reported a
/// *finding* for a path it never opened. `diff`, `sync`, `scan` and `doctor`
/// were each wrapped individually and already returned 2; this is now the
/// default for every command rather than something each one has to remember.
#[test]
fn a_file_that_cannot_be_read_exits_2_not_1() {
    let dir = project();

    for args in [
        vec!["validate", "--env", "./nope.env"],
        vec!["convert", "--to", "json", "--env", "./nope.env"],
        vec!["template", "--input", "./nope.tpl", "--output", "out.txt"],
    ] {
        cargo_bin_cmd!("evnx")
            .current_dir(dir.path())
            .args(&args)
            .assert()
            .code(2);
    }
}

/// The complement, and the one that matters most: a real finding must still be
/// 1, or the guard above would have turned every gate into a pass.
#[test]
fn a_real_validation_error_is_still_1() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".env.example"), "A=\nB=\nC=\n").unwrap();
    fs::write(dir.path().join(".env"), "A=1\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .arg("validate")
        .assert()
        .code(1);
}
