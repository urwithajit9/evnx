//! `.evnx.toml` applied end to end.
//!
//! The unit tests cover parsing and precedence in isolation; these check that a
//! config file actually reaches the commands, and that it is announced.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project(config: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), "A=1\nEXTRA_ONLY_HERE=2\n").unwrap();
    fs::write(d.path().join(".env.example"), "A=x\n").unwrap();
    fs::write(d.path().join(".env.production"), "A=prod\n").unwrap();
    if !config.is_empty() {
        fs::write(d.path().join(".evnx.toml"), config).unwrap();
    }
    d
}

#[test]
fn validate_strict_comes_from_config() {
    let without = project("");
    cargo_bin_cmd!("evnx")
        .current_dir(without.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE").not());

    let with = project("[validate]\nstrict = true\n");
    cargo_bin_cmd!("evnx")
        .current_dir(with.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE"));
}

#[test]
fn defaults_env_name_selects_the_file() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("prod"), "{stdout}");
}

/// ⚠️ The precedence bug that `--env` becoming `Option` exists to fix.
#[test]
fn an_explicit_flag_beats_config() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--env", ".env", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("EXTRA_ONLY_HERE"),
        "--env must win over [defaults] env_name:\n{stdout}"
    );
}

/// ⚠️ A committed config can weaken scanning, so every run that loads one says
/// so — and names the settings that changed a security default.
#[test]
fn a_config_that_weakens_scanning_is_announced() {
    let d = project("[scan]\nseverity = \"high\"\nexclude = [\"fixtures/\"]\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml"))
        .stderr(predicate::str::contains("scan.severity=high"))
        .stderr(predicate::str::contains("1 patterns"));
}

#[test]
fn a_config_that_changes_nothing_security_relevant_is_announced_plainly() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml"))
        .stderr(predicate::str::contains("scan.severity").not());
}

/// The banner and warnings are stderr-only, or `evnx convert --to json > out`
/// would stop being machine-readable.
#[test]
fn nothing_about_config_reaches_stdout() {
    let d = project("[scan]\nseverity = \"high\"\n\n[nonesuch]\nx = 1\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(!stdout.contains(".evnx.toml"), "{stdout}");
    assert!(!stdout.contains("unknown key"), "{stdout}");
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout must stay valid JSON");
}

#[test]
fn unknown_keys_are_named_but_do_not_fail_the_run() {
    let d = project("[scan]\nseverity = \"high\"\nentropy_threshold = 4.5\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exit-zero"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "unknown key `scan.entropy_threshold`",
        ));
}

#[test]
fn quiet_suppresses_the_banner() {
    let d = project("[scan]\nseverity = \"high\"\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--quiet", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml").not());
}

/// A malformed config is an error: ignoring one written on purpose would mean
/// running with policy the user believes is in force.
#[test]
fn a_malformed_config_stops_the_run() {
    let d = project("[scan\nseverity = ");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(".evnx.toml"));
}
