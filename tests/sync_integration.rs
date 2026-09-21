//! Integration tests for the sync command.
//! These tests create real temp files and test end-to-end behavior.

use anyhow::Result;
use assert_cmd::cargo::cargo_bin_cmd;
use evnx::cli::{NamingPolicy, SyncDirection};
use evnx::commands::sync;
use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test fixture that manages temp directory with absolute paths
/// ✅ No set_current_dir() = tests can run in parallel safely
struct SyncTestFixture {
    temp_dir: TempDir,
    env_path: PathBuf,
    example_path: PathBuf,
    config_path: PathBuf,
}

impl SyncTestFixture {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let env_path = temp_dir.path().join(".env");
        let example_path = temp_dir.path().join(".env.example");
        let config_path = temp_dir.path().join("placeholders.json");

        Ok(Self {
            temp_dir,
            env_path,
            example_path,
            config_path,
        })
    }

    fn write_env(&self, content: &str) -> Result<()> {
        fs::write(&self.env_path, content)?;
        assert!(self.env_path.exists(), ".env file should exist after write");
        Ok(())
    }

    fn write_example(&self, content: &str) -> Result<()> {
        fs::write(&self.example_path, content)?;
        assert!(
            self.example_path.exists(),
            ".env.example should exist after write"
        );
        Ok(())
    }

    fn write_config(&self, content: &str) -> Result<()> {
        fs::write(&self.config_path, content)?;
        assert!(
            self.config_path.exists(),
            "Config file should exist after write"
        );
        Ok(())
    }

    fn read_example(&self) -> Result<String> {
        Ok(fs::read_to_string(&self.example_path)?)
    }

    fn read_env(&self) -> Result<String> {
        Ok(fs::read_to_string(&self.env_path)?)
    }

    fn env_exists(&self) -> bool {
        self.env_path.exists()
    }

    // fn example_exists(&self) -> bool {
    //     self.example_path.exists()
    // }
}

#[test]
#[serial]
fn test_forward_sync_dry_run_adds_preview() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture.write_env("NEW_VAR=test_value\nEXISTING=keep")?;
    fixture.write_example("EXISTING=placeholder")?;

    // ✅ Save original dir and change to temp dir
    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    // Run sync in temp directory
    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Forward,
        true,
        false,
        true,
        true,
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    // ✅ Restore directory IMMEDIATELY (before fixture drops and deletes temp dir)
    let _ = env::set_current_dir(&original_dir);

    // Now run assertions (still have access to fixture via absolute paths)
    assert!(result.is_ok(), "Dry run should succeed: {:?}", result.err());
    let example_content = fixture.read_example()?;
    assert_eq!(example_content.trim(), "EXISTING=placeholder");

    Ok(())
}

/// `--dry-run` must not need a terminal.
///
/// ⚠️ Note the existing dry-run test above passes `force = true`, which takes the
/// short-circuit that skips every prompt — so it never exercised the path a user
/// actually runs. With `force = false` this reached `Select::interact()` and died
/// with `IO error: not a terminal` before printing anything, which is why every
/// CI recipe in the docs carries `--dry-run --force`: a pairing that reads as a
/// contradiction and was only ever a workaround.
///
/// The test harness has no TTY, so simply completing is the assertion.
#[test]
#[serial]
fn dry_run_needs_no_terminal_forward() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture.write_env("NEW_VAR=test_value\nEXISTING=keep")?;
    fixture.write_example("EXISTING=placeholder")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Forward,
        true,
        false,
        true,  // dry_run
        false, // force — the point of the test
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "`sync --dry-run` must work without a TTY: {:?}",
        result.err()
    );
    assert_eq!(
        fixture.read_example()?.trim(),
        "EXISTING=placeholder",
        "a dry run must not modify anything"
    );

    Ok(())
}

/// Same guarantee for `--direction reverse`.
#[test]
#[serial]
fn dry_run_needs_no_terminal_reverse() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture.write_env("EXISTING=keep")?;
    fixture.write_example("EXISTING=placeholder\nMISSING_VAR=example")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Reverse,
        true,
        false,
        true,  // dry_run
        false, // force
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "`sync --direction reverse --dry-run` must work without a TTY: {:?}",
        result.err()
    );
    assert_eq!(
        fixture.read_env()?.trim(),
        "EXISTING=keep",
        "a dry run must not modify anything"
    );

    Ok(())
}

#[test]
#[serial]
fn test_reverse_sync_creates_env_with_placeholders() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture.write_example("DB_URL=YOUR_URL_HERE\nAPI_KEY=YOUR_KEY_HERE")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Reverse,
        true,
        false,
        false,
        true,
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    // Restore BEFORE fixture drops
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Reverse sync should succeed: {:?}",
        result.err()
    );
    assert!(fixture.env_exists(), ".env should be created");
    let env_content = fixture.read_env()?;
    assert!(env_content.contains("DB_URL="));

    Ok(())
}

#[test]
#[serial]
fn test_forward_sync_security_warning_with_actual_values() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture.write_env("SECRET_KEY=sk_live_abc123")?;
    fixture.write_example("")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Forward,
        false,
        false,
        false,
        true,
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Forward sync should succeed: {:?}",
        result.err()
    );
    let example_content = fixture.read_example()?;
    assert!(
        example_content.contains("YOUR_KEY_HERE") || example_content.contains("YOUR_VALUE_HERE")
    );
    assert!(!example_content.contains("sk_live_abc123"));

    Ok(())
}

#[test]
#[serial]
fn test_sync_with_custom_placeholder_config() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    fixture
        .write_config(r#"{"patterns": {"CUSTOM_.*": "custom_val"}, "default": "MY_DEFAULT"}"#)?;
    fixture.write_env("CUSTOM_VAR=secret\nOTHER_VAR=value")?;
    fixture.write_example("")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    // ✅ Use absolute path for config
    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Forward,
        true,
        false,
        false,
        true,
        false, // check
        "pretty".to_string(),
        Some(fixture.config_path.clone()), // Absolute path
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Sync with config should succeed: {:?}",
        result.err()
    );
    let example_content = fixture.read_example()?;
    assert!(example_content.contains("CUSTOM_VAR=custom_val"));
    assert!(example_content.contains("OTHER_VAR=MY_DEFAULT"));

    Ok(())
}

#[test]
#[serial] // Add this if using Option 2
fn test_forward_sync_missing_env_file() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    // Don't create .env file

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Forward,
        true,
        false,
        false,
        true,
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains(".env") || err.contains("init"));

    Ok(())
}

#[test]
#[serial]
fn test_reverse_sync_missing_example_file() -> Result<()> {
    use std::env;

    let fixture = SyncTestFixture::new()?;
    // Don't create .env.example

    let original_dir = env::current_dir()?;
    env::set_current_dir(fixture.temp_dir.path())?;

    let result = sync::run(
        ".env".to_string(),
        ".env.example".to_string(),
        SyncDirection::Reverse,
        true,
        false,
        false,
        true,
        false, // check
        "pretty".to_string(),
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains(".env.example") || err.contains("init"));

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// `--check` — the CI gate
// ─────────────────────────────────────────────────────────────
//
// These drive the binary rather than `sync::run`, because `--check` reports its
// verdict with `std::process::exit`, which would take the test harness with it.
// Each uses its own temp dir and never touches the process cwd, so they need no
// `#[serial]`.

mod check_flag {
    use assert_cmd::Command;
    use predicates::prelude::*;
    use tempfile::TempDir;

    fn project(env: &str, example: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), env).unwrap();
        std::fs::write(dir.path().join(".env.example"), example).unwrap();
        dir
    }

    #[test]
    fn check_fails_when_the_template_is_missing_a_variable() {
        let dir = project("A=1\nNEW=2\n", "A=placeholder\n");

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--check"])
            .current_dir(dir.path())
            .assert()
            .failure()
            .stderr(predicate::str::contains("Out of sync"));

        assert_eq!(
            std::fs::read_to_string(dir.path().join(".env.example")).unwrap(),
            "A=placeholder\n",
            "--check must not write anything"
        );
    }

    #[test]
    fn check_passes_when_in_sync() {
        let dir = project("A=1\nNEW=2\n", "A=placeholder\nNEW=placeholder\n");

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--check"])
            .current_dir(dir.path())
            .assert()
            .success();
    }

    #[test]
    fn check_works_in_the_reverse_direction() {
        let dir = project("A=1\n", "A=placeholder\nONLY_IN_TEMPLATE=x\n");

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--direction", "reverse", "--check"])
            .current_dir(dir.path())
            .assert()
            .failure()
            .stderr(predicate::str::contains("Out of sync"));

        assert_eq!(
            std::fs::read_to_string(dir.path().join(".env")).unwrap(),
            "A=1\n",
            "--check must not write anything"
        );
    }

    /// `--check` must not need a terminal — that is the whole point of it.
    #[test]
    fn check_needs_no_terminal_and_no_force() {
        let dir = project("A=1\nNEW=2\n", "A=placeholder\n");

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--check"])
            .current_dir(dir.path())
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Out of sync")
                    .and(predicate::str::contains("not a terminal").not()),
            );
    }

    /// Plain `--dry-run` keeps its contract: preview, always exit 0.
    ///
    /// Changing that would break anyone already calling it under `set -e`, which
    /// is why `--check` is a separate flag rather than new behaviour on --dry-run.
    #[test]
    fn plain_dry_run_still_exits_zero_when_out_of_sync() {
        let dir = project("A=1\nNEW=2\n", "A=placeholder\n");

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--dry-run"])
            .current_dir(dir.path())
            .assert()
            .success();
    }
}

// ─────────────────────────────────────────────────────────────
// `--check` exit codes — 0 in sync / 1 out of sync / 2 error
// ─────────────────────────────────────────────────────────────

mod check_exit_codes {
    use assert_cmd::Command;
    use tempfile::TempDir;

    const IN_SYNC: i32 = 0;
    const OUT_OF_SYNC: i32 = 1;
    const ERROR: i32 = 2;

    /// Writes only the files named; a `None` means "this file does not exist".
    fn dir(env: Option<&[u8]>, example: Option<&[u8]>) -> TempDir {
        let d = TempDir::new().unwrap();
        if let Some(b) = env {
            std::fs::write(d.path().join(".env"), b).unwrap();
        }
        if let Some(b) = example {
            std::fs::write(d.path().join(".env.example"), b).unwrap();
        }
        d
    }

    fn check_code(d: &TempDir, extra: &[&str]) -> i32 {
        let mut cmd = Command::cargo_bin("evnx").unwrap();
        cmd.arg("sync")
            .arg("--check")
            .args(extra)
            .current_dir(d.path());
        cmd.assert().get_output().status.code().unwrap()
    }

    #[test]
    fn zero_when_in_sync() {
        let d = dir(Some(b"A=1\n"), Some(b"A=x\n"));
        assert_eq!(check_code(&d, &[]), IN_SYNC);
    }

    #[test]
    fn one_when_a_variable_is_missing_from_the_template() {
        let d = dir(Some(b"A=1\nNEW=2\n"), Some(b"A=x\n"));
        assert_eq!(check_code(&d, &[]), OUT_OF_SYNC);
    }

    /// An absent *target* is not an error — running sync would create it.
    #[test]
    fn one_when_the_target_does_not_exist_yet() {
        let fwd = dir(Some(b"A=1\n"), None);
        assert_eq!(check_code(&fwd, &[]), OUT_OF_SYNC);

        let rev = dir(None, Some(b"A=x\n"));
        assert_eq!(check_code(&rev, &["--direction", "reverse"]), OUT_OF_SYNC);
    }

    /// An absent *source* is an error — there is nothing to sync from, so the
    /// command cannot reach a verdict at all.
    #[test]
    fn two_when_the_source_does_not_exist() {
        let fwd = dir(None, Some(b"A=x\n"));
        assert_eq!(check_code(&fwd, &[]), ERROR);

        let rev = dir(Some(b"A=1\n"), None);
        assert_eq!(check_code(&rev, &["--direction", "reverse"]), ERROR);
    }

    /// ⚠️ The distinction this whole contract turns on.
    ///
    /// A template that exists but will not parse used to be reported as
    /// ".env.example not found" — the `Err(_)` arm swallowed the difference — so
    /// `--check` called it *out of sync* (1) and a plain `evnx sync` **overwrote
    /// it**. It is an error (2), and the file is left alone.
    #[test]
    fn two_when_the_template_exists_but_will_not_parse() {
        let d = dir(Some(b"A=1\n"), Some(b"\x00\x01 not = valid\n"));
        assert_eq!(check_code(&d, &[]), ERROR);
    }

    #[test]
    fn a_corrupt_template_is_never_overwritten() {
        const CORRUPT: &[u8] = b"\x00\x01 not = valid\n";
        let d = dir(Some(b"A=1\n"), Some(CORRUPT));

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--force"])
            .current_dir(d.path())
            .assert()
            .failure();

        assert_eq!(
            std::fs::read(d.path().join(".env.example")).unwrap(),
            CORRUPT,
            "an unreadable .env.example must be reported, never replaced"
        );
    }

    /// The contract is opt-in: without `--check`, exit codes are untouched.
    #[test]
    fn without_check_nothing_changed() {
        let d = dir(Some(b"A=1\nNEW=2\n"), Some(b"A=x\n"));

        let code = |args: &[&str]| {
            Command::cargo_bin("evnx")
                .unwrap()
                .arg("sync")
                .args(args)
                .current_dir(d.path())
                .assert()
                .get_output()
                .status
                .code()
                .unwrap()
        };

        assert_eq!(code(&["--dry-run"]), 0, "--dry-run previews and exits 0");

        // An error without --check still exits 1 via main, not 2.
        let broken = dir(None, Some(b"A=x\n"));
        let c = Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--dry-run"])
            .current_dir(broken.path())
            .assert()
            .get_output()
            .status
            .code()
            .unwrap();
        assert_eq!(c, 1, "errors keep their old code unless --check opts in");
    }
}

// ─────────────────────────────────────────────────────────────
// `--env-name` / `--env` / `--example` — sync could address nothing
// ─────────────────────────────────────────────────────────────

mod file_selection {
    use assert_cmd::Command;
    use predicates::prelude::*;
    use tempfile::TempDir;

    fn project() -> TempDir {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join(".env.example"), "A=x\n").unwrap();
        std::fs::write(d.path().join(".env"), "A=dev\n").unwrap();
        std::fs::write(d.path().join(".env.production"), "A=prod\nPROD_ONLY=1\n").unwrap();
        d
    }

    fn code(d: &TempDir, args: &[&str]) -> i32 {
        Command::cargo_bin("evnx")
            .unwrap()
            .arg("sync")
            .args(args)
            .current_dir(d.path())
            .assert()
            .get_output()
            .status
            .code()
            .unwrap()
    }

    /// `.env` matches the template, `.env.production` does not. Before this,
    /// both invocations read `.env` and there was no way to say otherwise.
    #[test]
    fn env_name_selects_the_file_that_is_checked() {
        let d = project();
        assert_eq!(code(&d, &["--check"]), 0, ".env is in sync");
        assert_eq!(
            code(&d, &["--env-name", "production", "--check"]),
            1,
            ".env.production has PROD_ONLY, which the template lacks"
        );
    }

    #[test]
    fn a_custom_template_path_is_honoured() {
        let d = project();
        std::fs::write(d.path().join("tpl.env"), "A=x\nPROD_ONLY=y\n").unwrap();

        assert_eq!(
            code(
                &d,
                &[
                    "--env-name",
                    "production",
                    "--example",
                    "tpl.env",
                    "--check"
                ]
            ),
            0,
            "tpl.env already covers .env.production"
        );
    }

    #[test]
    fn the_named_environment_is_what_gets_written() {
        let d = project();

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--env-name", "production", "--force"])
            .current_dir(d.path())
            .assert()
            .success();

        let tpl = std::fs::read_to_string(d.path().join(".env.example")).unwrap();
        assert!(tpl.contains("PROD_ONLY"), "{tpl}");
        assert!(
            tpl.contains(".env.production"),
            "the provenance comment must name the real source, not .env:\n{tpl}"
        );
        // .env was not the source and must be untouched.
        assert_eq!(
            std::fs::read_to_string(d.path().join(".env")).unwrap(),
            "A=dev\n"
        );
    }

    #[test]
    fn a_missing_environment_is_an_error() {
        let d = project();

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--env-name", "nope", "--check"])
            .current_dir(d.path())
            .assert()
            .failure()
            .stderr(predicate::str::contains("does not exist"));
    }

    #[test]
    fn the_header_names_the_files_in_play() {
        let d = project();

        Command::cargo_bin("evnx")
            .unwrap()
            .args(["sync", "--env-name", "production", "--dry-run"])
            .current_dir(d.path())
            .assert()
            .stdout(predicate::str::contains(".env.production"));
    }

    #[test]
    fn defaults_are_unchanged() {
        let d = project();
        assert_eq!(code(&d, &["--dry-run"]), 0);
        // Still reads .env against .env.example with no flags.
        assert_eq!(code(&d, &["--check"]), 0);
    }
}

// ─────────────────────────────────────────────────────────────
// --check --format json
// ─────────────────────────────────────────────────────────────

/// ⚠️ The gap this closes: until now a pipeline could learn *that* the template
/// was stale, from the exit code, and nothing about *what* was stale.
#[test]
fn check_json_reports_which_keys_drifted() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\nB=2\nC=3\n").unwrap();
    std::fs::write(dir.path().join(".env.example"), "A=\n").unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["sync", "--check", "--format", "json"])
        .assert()
        .code(1);

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(report["in_sync"], false);
    assert_eq!(report["summary"]["missing"], 2);
    let missing: Vec<&str> = report["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(missing, vec!["B", "C"], "sorted, so output is stable");
}

#[test]
fn check_json_says_so_when_in_sync() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    std::fs::write(dir.path().join(".env.example"), "A=\n").unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["sync", "--check", "--format", "json"])
        .assert()
        .code(0);

    let report: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid JSON");
    assert_eq!(report["in_sync"], true);
    assert_eq!(report["summary"]["missing"], 0);
}

/// ⚠️ An unreadable template is exit 2, not 1. Under `--check` an error is a
/// different answer from "out of sync", and a pipeline branching on the code
/// would otherwise treat a broken file as drift.
#[test]
fn check_json_distinguishes_trouble_from_drift() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    std::fs::write(dir.path().join(".env.example"), "this is not = valid\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["sync", "--check", "--format", "json"])
        .assert()
        .code(2);
}

#[test]
fn check_json_follows_the_direction() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    std::fs::write(dir.path().join(".env.example"), "A=\nONLY_IN_TEMPLATE=\n").unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args([
            "sync",
            "--check",
            "--format",
            "json",
            "--direction",
            "reverse",
        ])
        .assert()
        .code(1);

    let report: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid JSON");
    assert_eq!(report["direction"], "reverse");
    assert_eq!(report["missing"][0], "ONLY_IN_TEMPLATE");
}

/// `--format json` without `--check` is refused: `sync` edits files, and a
/// machine-readable report of an edit already made is worth less than the edit.
#[test]
fn json_requires_check() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["sync", "--format", "json"])
        .assert()
        .failure()
        .stderr(predicates::prelude::predicate::str::contains("--check"));
}
