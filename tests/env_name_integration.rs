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
