// tests/doctor_integration.rs

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_doctor_json_output() {
    let dir = TempDir::new().unwrap();

    // Setup minimal valid project
    fs::write(dir.path().join(".env.example"), "FOO=bar\n").unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("doctor")
        .arg(dir.path())              // ← Positional path argument
        .env("EVNX_OUTPUT_JSON", "1") // ← JSON output via env var
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""summary""#))
        .stdout(predicate::str::contains(r#""checks""#));
}

#[test]
fn test_doctor_verbose_mode() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("doctor")
        .arg(dir.path())
        .arg("--verbose")            // ← Verbose flag works
        .assert()
        .success()
        .stdout(predicate::str::contains("Project path:"));
}

#[test]
fn test_doctor_missing_env_warning() {
    let dir = TempDir::new().unwrap();
    // No .env file created → should warn

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .success()  // Warnings don't cause exit(1), only errors do
        .stdout(predicate::str::contains("Warning").or(predicate::str::contains("⚠️")));
}