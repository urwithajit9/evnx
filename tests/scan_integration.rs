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

use assert_cmd::cargo::cargo_bin_cmd;
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

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    assert!(
        stdout.contains("AWS Access Key"),
        "Expected output to contain 'AWS Access Key', got:\n{}",
        stdout
    );
    // The confidence level is reported; `(high confidence)` became a right-aligned
    // `high` so the levels form a column. Asserting the level, not its phrasing.
    assert!(
        stdout.contains("high"),
        "Expected the confidence level in the output, got:\n{}",
        stdout
    );
    // And the finding carries where it is and a masked value, not the secret.
    assert!(
        stdout.contains("AWS_ACCESS_KEY_ID") && !stdout.contains("AKIA4OZRMFJ3VREALKEY"),
        "Expected the variable named and the value masked, got:\n{}",
        stdout
    );
}

/// Test that scan exits with code 1 when secrets are found (default behavior).
#[test]
fn test_scan_without_exit_zero() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    // No --exit-zero: secrets found → must exit 1
    let assert = cargo_bin_cmd!("evnx")
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

    // Safe placeholder values — no real secrets
    fs::write(
        &env_file,
        "DATABASE_URL=postgresql://localhost/dev\nAPI_KEY=changeme\n",
    )
    .expect("Failed to write test .env file");

    let assert = cargo_bin_cmd!("evnx")
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

#[test]
fn test_scan_ignore_placeholders() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let env_file = temp_dir.path().join(".env");

    fs::write(&env_file, "API_KEY=your_api_key_here\nSECRET=example123\n")
        .expect("Failed to write test .env file");

    // ✅ Fix: Use cargo_bin_cmd! macro (returns Command)
    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--ignore-placeholders")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

    // Clear assertions
    assert!(
        stdout.contains("No secrets detected"),
        "Expected 'No secrets detected' with --ignore-placeholders, got:\n{}",
        stdout
    );

    assert!(
        !stdout.contains("your_api_key_here"),
        "Placeholder should not appear in output, got:\n{}",
        stdout
    );
}

/// Test JSON output format.
#[test]
fn test_scan_json_format() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("json")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

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

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--exclude")
        .arg("test.env")
        .arg("--exit-zero")
        .assert();

    let stdout = get_stdout(&assert);
    assert_cmd::assert::Assert::code(assert, 0);

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

    let assert = cargo_bin_cmd!("evnx")
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

    // Pretty format should emit UI header on stderr
    let pretty_assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("pretty")
        .arg("--exit-zero")
        .assert();

    let pretty_stderr = String::from_utf8_lossy(&pretty_assert.get_output().stderr);

    assert!(
        pretty_stderr.contains("Checking for exposed secrets"),
        "Pretty format should show header on stderr. Got stderr:\n{}",
        pretty_stderr
    );
    assert!(
        pretty_stderr.contains("evnx scan"),
        "Header should contain command title. Got stderr:\n{}",
        pretty_stderr
    );

    // JSON format should NOT have UI header on stdout
    let json_assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .arg("scan")
        .arg("--format")
        .arg("json")
        .arg("--exit-zero")
        .assert();

    let json_stdout = get_stdout(&json_assert);
    let trimmed = json_stdout.trim_start();
    assert!(
        trimmed.starts_with('{'),
        "JSON output should start with '{{', got: {}",
        &trimmed[..trimmed.len().min(50)]
    );
}

/// `scan` was the only machine-readable command without a `summary`, and
/// `evnx.dev` documented it as if it had one — `jq -e '.summary.errors == 0'`
/// failed a build that was clean, because `.summary` was `null`.
#[test]
fn json_output_carries_a_summary() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .args(["scan", "--format", "json", "--exit-zero"])
        .assert();

    let json = parse_json_output(&get_stdout(&assert)).expect("valid JSON");
    let summary = &json["summary"];

    assert!(summary.is_object(), "scan JSON must carry a summary object");
    assert_eq!(summary["total"], json["secrets_found"]);
    assert_eq!(summary["high"], json["high_confidence"]);
    assert_eq!(summary["medium"], json["medium_confidence"]);
    assert_eq!(summary["low"], json["low_confidence"]);
    assert_eq!(summary["files_scanned"], json["files_scanned"]);
    assert!(summary["total"].as_u64().unwrap() > 0);
}

/// Adding `summary` must not move or drop anything that was already there.
#[test]
fn json_summary_is_purely_additive() {
    let temp_dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(temp_dir.path())
        .args(["scan", "--format", "json", "--exit-zero"])
        .assert();

    let json = parse_json_output(&get_stdout(&assert)).expect("valid JSON");

    for key in [
        "files_scanned",
        "secrets_found",
        "findings",
        "high_confidence",
        "medium_confidence",
        "low_confidence",
    ] {
        assert!(
            !json[key].is_null(),
            "pre-existing key `{key}` must survive"
        );
    }
}

// ─────────────────────────────────────────────────────────────
// `--severity` — the CI threshold
// ─────────────────────────────────────────────────────────────

fn mixed_confidence_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".env"),
        "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n\
         GENERIC_BLOB=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWo=\n",
    )
    .unwrap();
    dir
}

fn summary_of(dir: &TempDir, args: &[&str]) -> serde_json::Value {
    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--format", "json", "--exit-zero"])
        .args(args)
        .assert();
    parse_json_output(&get_stdout(&assert)).expect("valid JSON")["summary"].clone()
}

#[test]
fn severity_high_drops_everything_below_it() {
    let dir = mixed_confidence_project();

    let all = summary_of(&dir, &[]);
    let high = summary_of(&dir, &["--severity", "high"]);

    assert_eq!(high["medium"], 0, "medium must be filtered out");
    assert_eq!(high["low"], 0, "low must be filtered out");
    assert_eq!(high["high"], all["high"], "high findings must survive");
    assert!(
        high["total"].as_u64().unwrap() <= all["total"].as_u64().unwrap(),
        "filtering cannot add findings"
    );
}

#[test]
fn the_default_reports_everything() {
    let dir = mixed_confidence_project();
    assert_eq!(
        summary_of(&dir, &[]),
        summary_of(&dir, &["--severity", "low"])
    );
}

/// ⚠️ The property that makes the flag safe to gate on: what is reported, what is
/// counted and what the process exits with all describe the **same** filtered
/// set. A scanner whose exit code disagreed with its own output would be worse
/// than one with no threshold at all.
#[test]
fn the_exit_code_follows_the_threshold() {
    let dir = TempDir::new().unwrap();
    // A medium-confidence finding and nothing higher.
    fs::write(
        dir.path().join(".env"),
        "GENERIC_BLOB=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWo=\n",
    )
    .unwrap();

    let below = summary_of(&dir, &["--severity", "high"]);
    assert_eq!(below["total"], 0, "nothing at or above high");

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--severity", "high"])
        .assert()
        .success();
}

#[test]
fn an_unknown_severity_is_an_error_not_a_default() {
    let dir = mixed_confidence_project();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--severity", "critical"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown severity"));
}

// ─────────────────────────────────────────────────────────────
// `--format github` — PR annotations
// ─────────────────────────────────────────────────────────────

#[test]
fn github_format_emits_workflow_commands() {
    let dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--format", "github", "--exit-zero"])
        .assert();

    let stdout = get_stdout(&assert);
    assert!(
        stdout.lines().any(|l| l.starts_with("::error file=")),
        "expected a ::error annotation, got:\n{stdout}"
    );
    assert!(
        stdout.contains("line="),
        "annotations must carry a line number"
    );
    assert!(stdout.contains("title="), "annotations must carry a title");
}

/// ⚠️ Annotations appear on the pull request and stay in the workflow log, both
/// readable by more people than the branch is. A scanner that published the
/// secret in order to report it would be doing the leaking itself.
#[test]
fn github_annotations_never_carry_the_secret() {
    const SECRET: &str = "AKIA4OZRMFJ3VREALKEY";
    let dir = setup_test_env(SECRET);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--format", "github", "--exit-zero"])
        .assert();

    let stdout = get_stdout(&assert);
    assert!(
        !stdout.contains(SECRET),
        "the secret leaked into an annotation:\n{stdout}"
    );
    assert!(
        stdout.contains("AWS_ACCESS_KEY_ID"),
        "the variable name should still be named:\n{stdout}"
    );
}

#[test]
fn github_format_respects_the_severity_threshold() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".env"),
        "GENERIC_BLOB=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWo=\n",
    )
    .unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args([
            "scan",
            "--format",
            "github",
            "--severity",
            "high",
            "--exit-zero",
        ])
        .assert();

    assert!(
        get_stdout(&assert).trim().is_empty(),
        "nothing at or above high means no annotations"
    );
}

/// ⚠️ `_ => Ok(Self::Pretty)` meant a typo, or a format that was only ever
/// documented, printed human-readable output and exited 0 — so a CI step that
/// looked configured was checking nothing.
#[test]
fn an_unknown_format_is_an_error_not_pretty_output() {
    let dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--format", "bogus", "--exit-zero"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown format 'bogus'"))
        .stderr(predicates::str::contains("sarif"));
}

#[test]
fn validate_also_rejects_an_unknown_format() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    fs::write(dir.path().join(".env.example"), "A=x\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["validate", "--format", "bogus"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown format 'bogus'"));
}

/// scan says `github`, validate says `github-actions`. Both accept both, so
/// nobody has to remember which command wanted which spelling.
#[test]
fn both_spellings_of_the_github_format_work() {
    let dir = setup_test_env("AKIA4OZRMFJ3VREALKEY");

    for spelling in ["github", "github-actions"] {
        cargo_bin_cmd!("evnx")
            .current_dir(dir.path())
            .args(["scan", "--format", spelling, "--exit-zero"])
            .assert()
            .success();
    }
}

// ─────────────────────────────────────────────────────────────
// One value, one finding — and the right label on it
// ─────────────────────────────────────────────────────────────

fn findings_for(env: &str) -> Vec<serde_json::Value> {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".env"), env).unwrap();
    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--format", "json", "--exit-zero"])
        .assert();
    parse_json_output(&get_stdout(&assert)).expect("valid JSON")["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A synthetic Stripe live key, assembled at runtime rather than written as one
/// literal.
///
/// ⚠️ GitHub push protection scans file contents, and any string convincingly
/// shaped like `sk_live_…` blocks the push exactly as a real key would — which
/// is correct behaviour, and it happened to this file. Splitting the prefix
/// keeps the fixture out of the detector's reach without weakening it.
///
/// It must satisfy **both** patterns for the test below to mean anything:
/// Stripe's `sk_live_[0-9a-zA-Z]{24,}`, and the shapeless
/// `[0-9a-zA-Z/+=]{40}` that the AWS secret check uses — 40 characters after the
/// prefix, with entropy above 4.5. Shorten it and the AWS branch stops matching,
/// at which point the test passes whether or not the bug is present.
fn synthetic_stripe_key() -> String {
    format!(
        "sk_{}_{}",
        "live", "abcdefghijklmnopqrstuvwxyz0123456789abcd"
    )
}

/// Both detectors fire on `STRIPE_SECRET_KEY`, so one key was counted as two
/// secrets, printed twice, and produced two GitHub annotations.
#[test]
fn one_variable_yields_one_finding() {
    let f = findings_for(&format!("STRIPE_SECRET_KEY={}\n", synthetic_stripe_key()));
    assert_eq!(f.len(), 1, "expected a single finding, got: {f:#?}");
}

/// ⚠️ A Stripe key is ~40 base64-ish characters and `STRIPE_SECRET_KEY` contains
/// SECRET, so the AWS gate — `contains("AWS") || contains("SECRET")` — claimed it
/// and told the user to revoke it in the AWS IAM console.
#[test]
fn a_stripe_key_is_not_reported_as_an_aws_key() {
    let f = findings_for(&format!("STRIPE_SECRET_KEY={}\n", synthetic_stripe_key()));
    let first = &f[0];

    assert!(
        first["pattern"].as_str().unwrap().contains("Stripe"),
        "expected a Stripe label, got {}",
        first["pattern"]
    );
    assert!(
        first["action_url"].as_str().unwrap().contains("stripe.com"),
        "remediation must point at Stripe, got {}",
        first["action_url"]
    );
}

/// The second half of the AWS fix, and the half reordering does not cover.
///
/// `MY_SECRET_TOKEN` is not a Stripe or GitHub key, so no specific pattern claims
/// it first — only the shapeless 40-character regex matches. With the old gate
/// (`contains("AWS") || contains("SECRET")`) the name contained SECRET, so it was
/// labelled an AWS Secret Access Key and pointed at the IAM console. An honest
/// "this key name looks sensitive" is the right answer.
#[test]
fn a_non_aws_secret_is_not_attributed_to_aws() {
    let f = findings_for("MY_SECRET_TOKEN=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYab\n");
    assert_eq!(f.len(), 1);

    let pattern = f[0]["pattern"].as_str().unwrap();
    assert!(
        !pattern.contains("AWS"),
        "a token that is not AWS must not be labelled AWS, got {pattern}"
    );
    assert!(
        f[0]["action_url"].is_null(),
        "and must not send the user to the IAM console: {}",
        f[0]["action_url"]
    );
}

/// Deduplication must not cost the specific label. The sensitive-key heuristic
/// scores High on any long value and would otherwise outrank the real AWS match,
/// leaving the user with "Sensitive config key" and no link to IAM.
#[test]
fn a_real_aws_secret_keeps_its_label_and_its_remediation_url() {
    let f = findings_for("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYab\n");
    assert_eq!(f.len(), 1);

    assert_eq!(f[0]["pattern"], "AWS Secret Access Key");
    assert!(f[0]["action_url"]
        .as_str()
        .unwrap()
        .contains("aws.amazon.com"));
}

/// ⚠️ And deduplication must not narrow `--severity high`. The AWS pattern is
/// Medium on purpose; the key name settles what the pattern cannot, so two
/// detectors agreeing raises the confidence rather than discarding one of them.
#[test]
fn corroborated_findings_keep_the_higher_confidence() {
    let f = findings_for("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYab\n");
    assert_eq!(
        f[0]["confidence"], "high",
        "a high-confidence corroboration must not be thrown away with the duplicate"
    );

    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".env"),
        "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYab\n",
    )
    .unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "--severity", "high"])
        .assert()
        .failure();
}

// ─────────────────────────────────────────────────────────────
// Exit code 2 — "I could not look" is not "I found nothing"
// ─────────────────────────────────────────────────────────────

/// ⚠️ The fail-open this fixes: `evnx scan ./typo` printed
/// "✓ No secrets detected" and exited 0. A renamed directory or an empty
/// variable in a CI `working-directory` turned the gate into a no-op that
/// reported success.
#[test]
fn a_path_that_does_not_exist_is_trouble_not_a_clean_result() {
    let dir = TempDir::new().unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "./definitely-not-here"])
        .assert()
        .code(2);

    let stdout = get_stdout(&assert);
    assert!(
        !stdout.contains("No secrets detected"),
        "a scan that never happened must not claim to be clean: {stdout}"
    );

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("does not exist"), "{stderr}");
    assert!(stderr.contains("No verdict"), "{stderr}");
}

/// `--exit-zero` means "do not fail my build over findings", not "never tell me
/// the scan was impossible". It suppresses 1 and leaves 2 alone.
#[test]
fn exit_zero_suppresses_findings_but_not_trouble() {
    let found = setup_test_env("AKIA4OZRMFJ3VREALKEY");
    cargo_bin_cmd!("evnx")
        .current_dir(found.path())
        .args(["scan", ".", "--exit-zero"])
        .assert()
        .code(0);

    let dir = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "./definitely-not-here", "--exit-zero"])
        .assert()
        .code(2);
}

/// Every bad path is named in one run, rather than one per invocation.
#[test]
fn all_unscannable_paths_are_reported_together() {
    let dir = TempDir::new().unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["scan", "./nope-a", "./nope-b"])
        .assert()
        .code(2);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("./nope-a"), "{stderr}");
    assert!(stderr.contains("./nope-b"), "{stderr}");
    assert!(stderr.contains("2 of the 2"), "{stderr}");
}

/// The three codes stay distinct: a real directory that is simply clean is still
/// 0, and findings are still 1.
#[test]
fn the_three_exit_codes_are_distinct() {
    let clean = TempDir::new().unwrap();
    fs::write(clean.path().join(".env"), "DEBUG=true\n").unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(clean.path())
        .args(["scan", "."])
        .assert()
        .code(0);

    let found = setup_test_env("AKIA4OZRMFJ3VREALKEY");
    cargo_bin_cmd!("evnx")
        .current_dir(found.path())
        .args(["scan", "."])
        .assert()
        .code(1);
}

// ─────────────────────────────────────────────────────────────
// --exclude globs match the way they are written
// ─────────────────────────────────────────────────────────────

/// ⚠️ `glob::Pattern` anchors at both ends, and paths are walked with a `./`
/// prefix — so `"fixtures/**"` and `"*.log"`, the two forms people actually
/// write, matched nothing at all and did so silently.
#[test]
fn exclude_globs_match_the_forms_people_write() {
    fn project() -> TempDir {
        let d = TempDir::new().unwrap();
        fs::create_dir_all(d.path().join("fixtures")).unwrap();
        fs::write(d.path().join(".env"), "AWS=AKIA4OZRMFJ3VREALKEY\n").unwrap();
        fs::write(d.path().join("fixtures/.env"), "AWS=AKIA4OZRMFJ3VREALKEY\n").unwrap();
        d
    }

    // Each of these must exclude fixtures/.env, leaving only the root finding.
    for pattern in ["fixtures/**", "./fixtures/**", "*fixtures*", "fixtures"] {
        let d = project();
        let assert = cargo_bin_cmd!("evnx")
            .current_dir(d.path())
            .args(["scan", ".", "--exclude", pattern, "--exit-zero"])
            .assert()
            .success();

        let stdout = get_stdout(&assert);
        assert!(
            !stdout.contains("fixtures"),
            "{pattern:?} must exclude fixtures/.env: {stdout}"
        );
    }

    // A filename glob reaches both files, so nothing is left to find.
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exclude", "*.env"])
        .assert()
        .code(0);
}

// ── Custom patterns: --pattern and [[scan.patterns]] ─────────────────────────
//
// Every test below was first run by hand against a real build. They exist so
// the behaviour cannot regress silently — `--pattern` spent a release being
// accepted by the parser and doing nothing, which is exactly the failure a
// suite that only tests the built-in detectors cannot see.

/// An internal credential with a name that gives nothing away.
///
/// Deliberately **not** a real provider format: the point is that no built-in
/// detector can recognise it, and that a project can declare it and have it
/// found anyway.
const ACME_KEY: &str = "ACME-7F3A9C21B85E4D0FA62C1D8B04E7A539";
const ACME_REGEX: &str = "ACME-[A-Z0-9]{32}";

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, body).expect("write");
    }
    dir
}

/// The gap custom patterns exist to close: a live credential whose name is
/// `TENANT_A` matches no built-in pattern and trips no name heuristic, so before
/// this it scanned clean.
#[test]
fn a_custom_pattern_finds_what_no_builtin_detector_can() {
    let d = project(&[(".env", &format!("TENANT_A={ACME_KEY}\n"))]);

    // Without the rule: nothing.
    let clean = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "."])
        .assert()
        .code(0);
    assert!(
        !get_stdout(&clean).contains("TENANT_A"),
        "the built-in detectors are not expected to know this format"
    );

    // With it: found.
    let found = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--pattern", ACME_REGEX])
        .assert()
        .code(1);
    assert!(get_stdout(&found).contains("TENANT_A"));
}

/// A rule the project wrote itself outranks a heuristic that only read the
/// variable's name, so the finding says what the value *is*.
#[test]
fn a_custom_pattern_outranks_the_name_based_heuristic() {
    let d = project(&[(".env", &format!("ACME_TOKEN={ACME_KEY}\n"))]);

    let named = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "scan",
            ".",
            "--format",
            "json",
            "--pattern",
            ACME_REGEX,
            "--exit-zero",
        ])
        .assert()
        .code(0);
    let json = parse_json_output(&get_stdout(&named)).expect("json");
    let patterns: Vec<&str> = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["pattern"].as_str().unwrap())
        .collect();

    assert_eq!(patterns, vec!["Custom pattern 1"]);
    assert!(
        !patterns.iter().any(|p| p.contains("Sensitive config key")),
        "the name heuristic should not answer in the custom rule's place: {patterns:?}"
    );
}

/// ⚠️ The other half of that precedence. A custom rule must **not** displace a
/// built-in provider match, because only the built-ins know where to revoke a
/// key — and a finding that can say "revoke it here" is worth more than one
/// that cannot.
#[test]
fn a_builtin_with_a_remediation_url_still_wins() {
    let d = project(&[(".env", "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n")]);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "scan",
            ".",
            "--format",
            "json",
            "--pattern",
            "AKIA[0-9A-Z]{16}",
            "--exit-zero",
        ])
        .assert()
        .code(0);
    let json = parse_json_output(&get_stdout(&assert)).expect("json");
    let finding = &json["findings"][0];

    assert_eq!(finding["pattern"], "AWS Access Key");
    assert_eq!(
        finding["action_url"], "https://console.aws.amazon.com/iam",
        "the IAM link must survive a custom rule matching the same value"
    );
}

/// `secret = false` retracts a guess made from a variable's **name**. A custom
/// pattern read the value, so it stands — no line in a committed `.evnx.toml`
/// can declare a matching credential to be something else.
#[test]
fn secret_false_cannot_retract_a_custom_pattern() {
    let d = project(&[
        (".env", &format!("INTERNAL_HANDLE={ACME_KEY}\n")),
        (
            ".evnx.toml",
            &format!(
                "[[scan.patterns]]\nname = \"Acme API key\"\nregex = \"{ACME_REGEX}\"\n\n\
                 [vars]\nINTERNAL_HANDLE = {{ secret = false }}\n"
            ),
        ),
    ]);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".env", "--format", "json", "--exit-zero"])
        .assert()
        .code(0);
    let json = parse_json_output(&get_stdout(&assert)).expect("json");
    assert_eq!(json["findings"][0]["pattern"], "Acme API key");
}

/// ⚠️ The reason `scan_line` exists.
///
/// `extract_tokens` keeps only tokens longer than 20 characters, so in a `.ts`
/// file a declared 9-character format would match nothing and the scan would
/// report clean. Custom patterns see the whole line instead.
#[test]
fn a_custom_pattern_shorter_than_the_token_floor_is_still_found() {
    let d = project(&[("src/client.ts", "const ring = \"RING-4821\";\n")]);

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "./src"])
        .assert()
        .code(0); // nothing built in knows it

    let found = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "./src", "--pattern", "RING-[0-9]{4}"])
        .assert()
        .code(1);
    assert!(get_stdout(&found).contains("client.ts"));
}

/// ⚠️ A rule that does not compile is trouble, not a clean scan.
///
/// Exit 0 here would mean CI reads a search that never ran as a pass.
#[test]
fn a_pattern_that_does_not_compile_exits_2_not_0() {
    let d = project(&[(".env", &format!("TENANT_A={ACME_KEY}\n"))]);

    for bad in ["(", "[z-a]"] {
        let assert = cargo_bin_cmd!("evnx")
            .current_dir(d.path())
            .args(["scan", ".", "--pattern", bad])
            .assert()
            .code(2);
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
        assert!(
            stderr.contains("No verdict"),
            "a broken rule must not read as a result: {stderr}"
        );
    }
}

/// `--exit-zero` means "do not fail my build over findings", not "never tell me
/// the scan was impossible".
#[test]
fn exit_zero_does_not_suppress_a_broken_pattern() {
    let d = project(&[(".env", &format!("TENANT_A={ACME_KEY}\n"))]);

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exit-zero", "--pattern", "("])
        .assert()
        .code(2);
}

/// A pattern matching the empty string matches everywhere, which would report
/// every line in the project as a secret.
#[test]
fn a_pattern_matching_everything_is_refused() {
    let d = project(&[(".env", &format!("TENANT_A={ACME_KEY}\n"))]);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--pattern", ".*"])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("empty string"));
}

/// The durable form: rules in `.evnx.toml` apply with nothing on the command
/// line, carry the name the project chose, and honour a declared confidence.
#[test]
fn project_rules_apply_with_no_flags() {
    let d = project(&[
        (".env", &format!("TENANT_A={ACME_KEY}\n")),
        ("src/client.ts", "const ring = \"RING-4821\";\n"),
        (
            ".evnx.toml",
            &format!(
                "[[scan.patterns]]\nname = \"Acme API key\"\n\
                 regex = \"{ACME_REGEX}\"\nurl = \"https://acme.example/keys\"\n\n\
                 [[scan.patterns]]\nname = \"Ring handle\"\n\
                 regex = \"RING-[0-9]{{4}}\"\nconfidence = \"medium\"\n"
            ),
        ),
    ]);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".env", "./src", "--format", "json", "--exit-zero"])
        .assert()
        .code(0);
    let json = parse_json_output(&get_stdout(&assert)).expect("json");

    assert_eq!(json["summary"]["high"], 1);
    assert_eq!(json["summary"]["medium"], 1);

    let acme = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["pattern"] == "Acme API key")
        .expect("the declared rule should report under its own name");
    assert_eq!(acme["action_url"], "https://acme.example/keys");
}

/// A declared `confidence` must reach `--severity`, or the flag would silently
/// report on a set the summary and exit code disagree with.
#[test]
fn a_declared_confidence_reaches_the_severity_gate() {
    let d = project(&[
        ("src/client.ts", "const ring = \"RING-4821\";\n"),
        (
            ".evnx.toml",
            "[[scan.patterns]]\nname = \"Ring handle\"\n\
             regex = \"RING-[0-9]{4}\"\nconfidence = \"medium\"\n",
        ),
    ]);

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "./src"])
        .assert()
        .code(1);

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "./src", "--severity", "high"])
        .assert()
        .code(0);
}

/// Additive, like `--exclude`: a flag adds this run's rule without dropping the
/// project's standing set.
#[test]
fn a_flag_and_the_project_rules_apply_together() {
    let d = project(&[
        (
            ".env",
            &format!("TENANT_A={ACME_KEY}\nAWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n"),
        ),
        (
            ".evnx.toml",
            &format!("[[scan.patterns]]\nname = \"Acme API key\"\nregex = \"{ACME_REGEX}\"\n"),
        ),
    ]);

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "scan",
            ".env",
            "--format",
            "json",
            "--pattern",
            "AKIA[0-9A-Z]{16}",
            "--exit-zero",
        ])
        .assert()
        .code(0);
    let json = parse_json_output(&get_stdout(&assert)).expect("json");
    let mut patterns: Vec<&str> = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["pattern"].as_str().unwrap())
        .collect();
    patterns.sort_unstable();

    assert_eq!(patterns, vec!["AWS Access Key", "Acme API key"]);
}
