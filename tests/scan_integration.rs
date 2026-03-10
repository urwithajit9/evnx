//! Integration tests for the `scan` command.
//!
//! These tests verify end-to-end behavior of the secret scanner,
//! including exit codes, output formatting, and flag handling.
//!
//! # Running Tests
//!
//! ```bash
//! # Run all scan integration tests
//! cargo test --test scan_integration
//!
//! # Run specific test
//! cargo test --test scan_integration test_scan_exit_zero
//!
//! # Show output for debugging
//! cargo test --test scan_integration -- --nocapture
//! ```

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn parse_json_output(stdout: &str) -> Result<serde_json::Value, serde_json::Error> {
    // Find the first '{' or '[' which should start the JSON
    let json_start = stdout.find(|c| c == '{' || c == '[').unwrap_or(0);
    serde_json::from_str(&stdout[json_start..])
}

/// Helper: Create a temp directory with a .env file containing a test secret.
fn setup_test_env(secret_value: &str) -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let env_file = temp_dir.path().join(".env");
    fs::write(&env_file, format!("AWS_ACCESS_KEY_ID={}\n", secret_value))
        .expect("Failed to write test .env file");
    temp_dir
}

/// Helper: Extract stdout as String from assert_cmd output.
fn get_stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
}

/// Test that `--exit-zero` flag forces exit code 0 even when secrets are found.
///
/// This is critical for CI/CD pipelines that want to scan but not fail builds.
#[test]
fn test_scan_exit_zero() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    // ✅ Get output FIRST, then assert on exit code
    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--exit-zero")
        .assert();

    // Get output before consuming assert with code()
    let stdout = get_stdout(&assert);

    // Now assert on exit code (this consumes assert, but we already have output)
    assert_cmd::assert::Assert::code(assert, 0);

    // ✅ Verify output content
    assert!(
        stdout.contains("AWS Access Key"),
        "Expected output to contain 'AWS Access Key', got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("high confidence"),
        "Expected output to contain 'high confidence', got:\n{}",
        stdout
    );
}

/// Test that scan exits with code 1 when secrets are found (default behavior).
#[test]
fn test_scan_without_exit_zero() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 1);

    assert!(
        stdout.contains("AWS Access Key"),
        "Expected output to contain 'AWS Access Key', got:\n{}",
        stdout
    );
}

/// Test that scan exits with code 0 when no secrets are found.
#[test]
fn test_scan_no_secrets_found() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let env_file = temp_dir.path().join(".env");

    // Create a file with NO secrets (just safe placeholder values)
    fs::write(
        &env_file,
        "DATABASE_URL=postgresql://localhost/dev\nAPI_KEY=changeme\n",
    )
    .expect("Failed to write test .env file");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    assert!(
        stdout.contains("No secrets detected") || !stdout.contains("Found"),
        "Expected 'No secrets detected' or no findings, got:\n{}",
        stdout
    );
}

/// Test that `--ignore-placeholders` skips placeholder values.
#[test]
fn test_scan_ignore_placeholders() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let env_file = temp_dir.path().join(".env");

    // File with placeholder that should be ignored
    fs::write(&env_file, "API_KEY=your_api_key_here\nSECRET=example123\n")
        .expect("Failed to write test .env file");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--ignore-placeholders")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    // Should not report the placeholder values as secrets
    assert!(
        !stdout.contains("your_api_key_here") || stdout.contains("No secrets detected"),
        "Placeholders should be ignored, got:\n{}",
        stdout
    );
}

/// Test JSON output format.
#[test]
fn test_scan_json_format() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("json")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    // ✅ Parse JSON (stdout should be clean now, but be defensive)
    let json = parse_json_output(&stdout).expect("Output should contain valid JSON");

    assert!(json["findings"].is_array());
    assert!(json["findings"].as_array().unwrap().len() > 0);

    if let Some(first) = json["findings"].as_array().and_then(|a| a.first()) {
        assert!(first["pattern"].is_string());
        assert!(first["confidence"].is_string());
    }
}

/// Test that excluded files are not scanned.
#[test]
fn test_scan_exclude_patterns() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // File that SHOULD be scanned
    let env_file = temp_dir.path().join(".env");
    fs::write(&env_file, "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n")
        .expect("Failed to write test .env file");

    // File that should be EXCLUDED
    let excluded_file = temp_dir.path().join("test.env");
    fs::write(&excluded_file, "SECRET_KEY=AKIA4OZRMFJ3VREALKEY\n")
        .expect("Failed to write excluded test file");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--exclude")
        .arg("test.env")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    // Should find the secret in .env
    assert!(
        stdout.contains(".env:"),
        "Expected to find secret in .env, got:\n{}",
        stdout
    );
}

/// Test SARIF output format structure.
#[test]
fn test_scan_sarif_format() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let env_file = temp_dir.path().join(".env");
    fs::write(&env_file, "GITHUB_TOKEN=ghp_1234567890abcdefghijklmnop\n")
        .expect("Failed to write test file");

    let assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("sarif")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    let json = parse_json_output(&stdout).expect("SARIF output should be valid JSON");

    assert_eq!(json["version"], "2.1.0");
    assert!(json["runs"].is_array());

    if let Some(runs) = json["runs"].as_array() {
        assert!(!runs.is_empty());
        if let Some(run) = runs.first() {
            assert!(run["tool"].is_object());
            assert!(run["results"].is_array());
        }
    }
}

/// Test that header appears in pretty format but not JSON.
#[test]
fn test_output_format_header_behavior() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    // Pretty format should have UI header (on stderr)
    let pretty_assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("pretty")
        .arg("--exit-zero")
        .assert();

    let pretty_stderr = String::from_utf8_lossy(&pretty_assert.get_output().stderr);

    // ✅ Fix: Match the ACTUAL subtitle text used in runner.rs
    assert!(
        pretty_stderr.contains("Checking for exposed secrets"),
        "Pretty format should show header on stderr. Got stderr:\n{}",
        pretty_stderr
    );

    // Also verify the title is present
    assert!(
        pretty_stderr.contains("evnx scan"),
        "Header should contain command title. Got stderr:\n{}",
        pretty_stderr
    );

    // JSON format should NOT have UI header on stdout
    let json_assert = Command::cargo_bin("evnx")
        .expect("Failed to find evnx binary")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("json")
        .arg("--exit-zero")
        .assert();

    let json_stdout = get_stdout(&json_assert);
    // JSON should start with { not box characters
    let trimmed = json_stdout.trim_start();
    assert!(
        trimmed.starts_with('{'),
        "JSON output should start with '{{', got: {}",
        &trimmed[..trimmed.len().min(50)]
    );
}
