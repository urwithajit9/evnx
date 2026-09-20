//! Integration tests for the sync command.
//! These tests create real temp files and test end-to-end behavior.

use anyhow::Result;
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
        SyncDirection::Forward,
        true,
        false,
        true,
        true,
        false, // check
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
        SyncDirection::Forward,
        true,
        false,
        true,  // dry_run
        false, // force — the point of the test
        false, // check
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
        SyncDirection::Reverse,
        true,
        false,
        true,  // dry_run
        false, // force
        false, // check
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
        SyncDirection::Reverse,
        true,
        false,
        false,
        true,
        false, // check
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
        SyncDirection::Forward,
        false,
        false,
        false,
        true,
        false, // check
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
        SyncDirection::Forward,
        true,
        false,
        false,
        true,
        false,                             // check
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
        SyncDirection::Forward,
        true,
        false,
        false,
        true,
        false, // check
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
        SyncDirection::Reverse,
        true,
        false,
        false,
        true,
        false, // check
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
