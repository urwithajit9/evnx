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

    fn example_exists(&self) -> bool {
        self.example_path.exists()
    }
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
        None,
        NamingPolicy::Ignore,
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains(".env.example") || err.contains("init"));

    Ok(())
}
