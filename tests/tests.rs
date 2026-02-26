/// Integration tests for evnx CLI
///
/// Tests actual command execution with real files

// use assert_cmd::Command;
use assert_cmd::prelude::*;
use std::process::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create a test environment
fn setup_test_env() -> TempDir {
    TempDir::new().unwrap()
}

/// Helper to create a basic .env.example
fn create_env_example(dir: &TempDir) -> std::path::PathBuf {
    let example_path = dir.path().join(".env.example");
    fs::write(
        &example_path,
        r#"# Database
DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=change-me
DEBUG=True

# AWS
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#,
    )
    .unwrap();
    example_path
}

/// Helper to create a basic .env
fn create_env(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let env_path = dir.path().join(".env");
    fs::write(&env_path, content).unwrap();
    env_path
}

// ============================================================================
// INIT COMMAND TESTS
// ============================================================================

#[test]
fn test_init_help() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Interactive project setup"));
}

#[test]
fn test_init_non_interactive() {
    let dir = setup_test_env();

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["init", "--stack", "python", "--services", "postgres,redis", "--yes"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created .env.example"));

    // Verify files were created
    assert!(dir.path().join(".env.example").exists());
    assert!(dir.path().join(".env").exists());

    // Verify content
    let content = fs::read_to_string(dir.path().join(".env.example")).unwrap();
    assert!(content.contains("DATABASE_URL"));
    assert!(content.contains("REDIS_URL"));
}

// ============================================================================
// VALIDATE COMMAND TESTS
// ============================================================================

#[test]
fn test_validate_help() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Check .env against .env.example"));
}

#[test]
fn test_validate_missing_file() {
    let dir = setup_test_env();

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("validate")
        .current_dir(dir.path())
        .assert()
        .failure();
}

#[test]
fn test_validate_finds_placeholder() {
    let dir = setup_test_env();
    create_env_example(&dir);
    create_env(
        &dir,
        r#"DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=YOUR_KEY_HERE
DEBUG=True
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#,
    );

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("validate")
        .current_dir(dir.path())
        .assert()
        .failure() // Should fail due to placeholder
        .stdout(predicate::str::contains("placeholder"));
}

#[test]
fn test_validate_json_output() {
    let dir = setup_test_env();
    create_env_example(&dir);
    create_env(
        &dir,
        r#"DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=good-secret-key-that-is-long-enough-32chars
DEBUG=True
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#,
    );

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["validate", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("status").is_some());
    assert!(json.get("issues").is_some());
}

#[test]
fn test_validate_boolean_trap() {
    let dir = setup_test_env();
    create_env_example(&dir);
    create_env(
        &dir,
        r#"DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=good-secret-key-that-is-long-enough-32chars
DEBUG=False
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#,
    );

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("validate")
        .current_dir(dir.path())
        .assert()
        .stdout(predicate::str::contains("truthy in Python"));
}

// ============================================================================
// SCAN COMMAND TESTS
// ============================================================================

#[test]
fn test_scan_help() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["scan", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Detect secrets"));
}

#[test]
fn test_scan_detects_aws_key() {
    let dir = setup_test_env();
    create_env(
        &dir,
        r#"AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY
AWS_SECRET_ACCESS_KEY=realSecretKeyWith40CharactersHere12345
"#,
    );

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("scan")
        .current_dir(dir.path())
        .assert()
        .failure() // Should fail when secrets found
        .stdout(predicate::str::contains("AWS Access Key"));
}

#[test]
fn test_scan_json_output() {
    let dir = setup_test_env();
    create_env(
        &dir,
        r#"AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY"#,
    );

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["scan", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("secrets_found").is_some());
    assert!(json.get("findings").is_some());
}

#[test]
fn test_scan_sarif_output() {
    let dir = setup_test_env();
    create_env(&dir, r#"AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY"#);

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["scan", "--format", "sarif"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    assert!(sarif.get("runs").is_some());
}

#[test]
fn test_scan_exit_zero() {
    let dir = setup_test_env();
    create_env(&dir, r#"AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY"#);

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["scan", "--exit-zero"])
        .current_dir(dir.path())
        .assert()
        .success(); // Should succeed even with secrets due to --exit-zero
}

// ============================================================================
// DIFF COMMAND TESTS
// ============================================================================

#[test]
fn test_diff_help() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compare .env vs .env.example"));
}

#[test]
fn test_diff_shows_missing() {
    let dir = setup_test_env();
    create_env_example(&dir);
    create_env(
        &dir,
        r#"DATABASE_URL=postgresql://localhost:5432/db
SECRET_KEY=test
"#,
    );

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("diff")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Missing from .env"));
}

#[test]
fn test_diff_json_output() {
    let dir = setup_test_env();
    create_env_example(&dir);
    create_env(&dir, r#"DATABASE_URL=postgresql://localhost:5432/db"#);

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["diff", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("missing").is_some());
    assert!(json.get("extra").is_some());
}

// ============================================================================
// CONVERT COMMAND TESTS
// ============================================================================

#[test]
fn test_convert_help() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Transform to different formats"));
}

#[test]
fn test_convert_to_json() {
    let dir = setup_test_env();
    create_env(&dir, r#"KEY1=value1
KEY2=value2
"#);

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["KEY1"], "value1");
    assert_eq!(json["KEY2"], "value2");
}

#[test]
fn test_convert_to_github_actions() {
    let dir = setup_test_env();
    create_env(&dir, r#"SECRET_KEY=abc123"#);

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "github-actions"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: SECRET_KEY"))
        .stdout(predicate::str::contains("Value: abc123"));
}

#[test]
fn test_convert_with_filter() {
    let dir = setup_test_env();
    create_env(
        &dir,
        r#"AWS_KEY=val1
DB_URL=val2
AWS_SECRET=val3
"#,
    );

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "json", "--include", "AWS_*"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("AWS_KEY").is_some());
    assert!(json.get("AWS_SECRET").is_some());
    assert!(json.get("DB_URL").is_none()); // Should be filtered out
}

#[test]
fn test_convert_with_transform() {
    let dir = setup_test_env();
    create_env(&dir, r#"DATABASE_URL=test"#);

    let output = assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "json", "--transform", "lowercase"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("database_url").is_some());
}

// ============================================================================
// END-TO-END WORKFLOW TESTS
// ============================================================================

#[test]
fn test_workflow_init_validate_scan() {
    let dir = setup_test_env();

    // 1. Init
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["init", "--stack", "python", "--yes"])
        .current_dir(dir.path())
        .assert()
        .success();

    // 2. Validate (should find placeholders)
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("validate")
        .current_dir(dir.path())
        .assert()
        .failure();

    // 3. Scan (should find example keys)
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("scan")
        .current_dir(dir.path())
        .assert()
        .success(); // Example keys are placeholders, should pass
}

#[test]
fn test_workflow_convert_multiple_formats() {
    let dir = setup_test_env();
    create_env(&dir, r#"KEY=value"#);

    // Convert to JSON
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "json"])
        .current_dir(dir.path())
        .assert()
        .success();

    // Convert to YAML
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "yaml"])
        .current_dir(dir.path())
        .assert()
        .success();

    // Convert to shell
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "shell"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("export"));
}

// ============================================================================
// GLOBAL FLAGS TESTS
// ============================================================================

#[test]
fn test_version_flag() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_help_flag() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("evnx"))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("validate"));
}

#[test]
fn test_verbose_flag() {
    let dir = setup_test_env();
    create_env(&dir, r#"KEY=value"#);

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "json", "--verbose"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("verbose"));
}

// ============================================================================
// ERROR HANDLING TESTS
// ============================================================================

#[test]
fn test_invalid_command() {
    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .arg("invalid-command")
        .assert()
        .failure();
}

#[test]
fn test_convert_invalid_format() {
    let dir = setup_test_env();
    create_env(&dir, r#"KEY=value"#);

    assert_cmd::Command::cargo_bin("evnx")
        .unwrap()
        .args(&["convert", "--to", "invalid-format"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown format"));
}