// tests/common/mod.rs

//! Shared utilities for integration tests.

use anyhow::Result;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// Submodules
pub mod assertions;
pub mod fixtures;

/// Create a temporary test directory
pub fn test_dir() -> Result<TempDir> {
    tempfile::tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))
}

/// Write initial .env.example content
pub fn write_env_example(dir: &Path, content: &str) -> Result<()> {
    fs::write(dir.join(".env.example"), content)
        .map_err(|e| anyhow::anyhow!("Failed to write .env.example: {}", e))
}

/// Write initial .env content
pub fn write_env(dir: &Path, content: &str) -> Result<()> {
    fs::write(dir.join(".env"), content).map_err(|e| anyhow::anyhow!("Failed to write .env: {}", e))
}

/// Read .env.example content
pub fn read_env_example(dir: &Path) -> Result<String> {
    fs::read_to_string(dir.join(".env.example"))
        .map_err(|e| anyhow::anyhow!("Failed to read .env.example: {}", e))
}

/// Read .env content
pub fn read_env(dir: &Path) -> Result<String> {
    fs::read_to_string(dir.join(".env")).map_err(|e| anyhow::anyhow!("Failed to read .env: {}", e))
}

/// Count non-comment, non-empty lines with '='
pub fn count_env_vars(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed.contains('=')
        })
        .count()
}

/// Parse KEY=value pairs from content
pub fn parse_env_vars(content: &str) -> std::collections::HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            trimmed
                .split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}
