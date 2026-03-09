// Add this import at the top of your test file
use assert_cmd::cargo::cargo_bin_cmd; // ← NEW IMPORT

// use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_doctor_json_output() {
    let dir = TempDir::new().unwrap();

    fs::write(dir.path().join(".env.example"), "FOO=bar\n").unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();

    // Replace Command::cargo_bin() with cargo_bin_cmd!()
    cargo_bin_cmd!("evnx")  // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .env("EVNX_OUTPUT_JSON", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""summary""#))
        .stdout(predicate::str::contains(r#""checks""#));
}

#[test]
fn test_doctor_verbose_mode() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();

    cargo_bin_cmd!("evnx")  // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .arg("--verbose")
        .assert()
        .success()
        .stdout(predicate::str::contains("Project path:"));
}

#[test]
fn test_doctor_missing_env_warning() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx")  // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Warning").or(predicate::str::contains("⚠️")));
}