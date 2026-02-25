//! Config file support for .evnx.toml
//!
//! # Overview
//!
//! Provides configuration file support allowing users to set defaults and preferences
//! that persist across command invocations. Configuration files are searched in:
//!
//! 1. Current directory (`.evnx.toml` or `evnx.toml`)
//! 2. Parent directories (recursively up to root)
//! 3. Home directory (`~/.evnx.toml` or `~/evnx.toml`)
//!
//! # Configuration Sections
//!
//! - **defaults** - Default file paths and behavior
//! - **validate** - Validation command defaults
//! - **scan** - Secret scanning defaults
//! - **convert** - Format conversion defaults
//! - **aliases** - Custom format aliases
//!
//! # Example Configuration
//!
//! ```toml
//! [defaults]
//! env_file = ".env"
//! example_file = ".env.example"
//! verbose = false
//!
//! [validate]
//! strict = true
//! format = "pretty"
//!
//! [scan]
//! ignore_placeholders = true
//! exclude_patterns = ["*.example", "*.template"]
//!
//! [convert]
//! default_format = "json"
//!
//! [aliases]
//! gh = "github-actions"
//! k8s = "kubernetes"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Main configuration struct
///
/// ✅ CLIPPY FIX: Uses `#[derive(Default)]` instead of manual implementation
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,

    #[serde(default)]
    pub validate: ValidateConfig,

    #[serde(default)]
    pub scan: ScanConfig,

    #[serde(default)]
    pub convert: ConvertConfig,

    #[serde(default)]
    pub aliases: Aliases,
}

/// Default settings for file paths and general behavior
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Defaults {
    #[serde(default = "default_env_file")]
    pub env_file: String,

    #[serde(default = "default_example_file")]
    pub example_file: String,

    #[serde(default)]
    pub verbose: bool,
}

/// Configuration for validation command
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ValidateConfig {
    #[serde(default)]
    pub strict: bool,

    #[serde(default)]
    pub auto_fix: bool,

    #[serde(default = "default_format")]
    pub format: String,
}

/// Configuration for secret scanning command
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ScanConfig {
    #[serde(default)]
    pub ignore_placeholders: bool,

    #[serde(default)]
    pub exclude_patterns: Vec<String>,

    #[serde(default = "default_format")]
    pub format: String,
}

/// Configuration for format conversion command
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConvertConfig {
    #[serde(default = "default_convert_format")]
    pub default_format: String,

    #[serde(default)]
    pub base64: bool,

    pub prefix: Option<String>,
    pub transform: Option<String>,
}

/// Custom format aliases for the convert command
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Aliases {
    #[serde(flatten)]
    pub formats: std::collections::HashMap<String, String>,
}

// ✅ CLIPPY FIX: Removed manual impl Default for Config (using #[derive(Default)])

impl Default for Defaults {
    fn default() -> Self {
        Self {
            env_file: default_env_file(),
            example_file: default_example_file(),
            verbose: false,
        }
    }
}

impl Default for ValidateConfig {
    fn default() -> Self {
        Self {
            strict: false,
            auto_fix: false,
            format: default_format(),
        }
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            ignore_placeholders: true,
            exclude_patterns: vec![
                "*.example".to_string(),
                "*.sample".to_string(),
                "*.template".to_string(),
            ],
            format: default_format(),
        }
    }
}

impl Default for ConvertConfig {
    fn default() -> Self {
        Self {
            default_format: default_convert_format(),
            base64: false,
            prefix: None,
            transform: None,
        }
    }
}

fn default_env_file() -> String {
    ".env".to_string()
}

fn default_example_file() -> String {
    ".env.example".to_string()
}

fn default_format() -> String {
    "pretty".to_string()
}

fn default_convert_format() -> String {
    "json".to_string()
}

impl Config {
    /// Load config from file
    pub fn load() -> Result<Self> {
        let path = Self::find_config_file()?;
        Self::load_from_path(&path)
    }

    /// Load config from specific path
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;

        Ok(config)
    }

    /// Find config file by searching up the directory tree
    pub fn find_config_file() -> Result<PathBuf> {
        let config_names = [".evnx.toml", "evnx.toml"];

        // Start from current directory
        let mut current_dir = std::env::current_dir()?;

        loop {
            for name in &config_names {
                let path = current_dir.join(name);
                if path.exists() {
                    return Ok(path);
                }
            }

            // Move up one directory
            if !current_dir.pop() {
                break;
            }
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            for name in &config_names {
                let path = home.join(name);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        // Return default config if no file found
        Err(anyhow::anyhow!("No config file found"))
    }

    /// Save config to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let toml = toml::to_string_pretty(self)?;
        fs::write(path, toml)?;
        Ok(())
    }

    /// Create example config file
    pub fn create_example(path: &Path) -> Result<()> {
        let example = r#"# evnx configuration file
# Place this in your project root or home directory

[defaults]
env_file = ".env"
example_file = ".env.example"
verbose = false

[validate]
strict = false
auto_fix = false
format = "pretty"  # Options: pretty, json, github-actions

[scan]
ignore_placeholders = true
exclude_patterns = ["*.example", "*.sample", "*.template"]
format = "pretty"  # Options: pretty, json, sarif

[convert]
default_format = "json"
base64 = false
# prefix = "APP_"
# transform = "uppercase"

[aliases]
# Format aliases for convert command
gh = "github-actions"
k8s = "kubernetes"
tf = "terraform"
"#;
        fs::write(path, example)?;
        Ok(())
    }

    /// Merge with CLI arguments (CLI args take precedence)
    pub fn merge_with_args(&self, cli_verbose: bool) -> Self {
        let mut config = self.clone();
        if cli_verbose {
            config.defaults.verbose = true;
        }
        config
    }

    /// Resolve format alias
    pub fn resolve_format_alias(&self, format: &str) -> String {
        self.aliases
            .formats
            .get(format)
            .cloned()
            .unwrap_or_else(|| format.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.defaults.env_file, ".env");
        assert_eq!(config.defaults.example_file, ".env.example");
        assert!(!config.defaults.verbose);
    }

    #[test]
    fn test_load_from_toml() {
        let toml = r#"
[defaults]
env_file = "custom.env"
verbose = true

[validate]
strict = true

[scan]
ignore_placeholders = false

[aliases]
gh = "github-actions"
"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(toml.as_bytes()).unwrap();

        let config = Config::load_from_path(file.path()).unwrap();
        assert_eq!(config.defaults.env_file, "custom.env");
        assert!(config.defaults.verbose);
        assert!(config.validate.strict);
        assert!(!config.scan.ignore_placeholders);
        assert_eq!(
            config.aliases.formats.get("gh"),
            Some(&"github-actions".to_string())
        );
    }

    #[test]
    fn test_resolve_alias() {
        let mut config = Config::default();
        config
            .aliases
            .formats
            .insert("gh".to_string(), "github-actions".to_string());

        assert_eq!(config.resolve_format_alias("gh"), "github-actions");
        assert_eq!(config.resolve_format_alias("json"), "json");
    }

    #[test]
    fn test_merge_with_args() {
        let config = Config::default();
        assert!(!config.defaults.verbose);

        let merged = config.merge_with_args(true);
        assert!(merged.defaults.verbose);
    }

    #[test]
    fn test_create_example() {
        let file = NamedTempFile::new().unwrap();
        Config::create_example(file.path()).unwrap();

        let content = fs::read_to_string(file.path()).unwrap();
        assert!(content.contains("[defaults]"));
        assert!(content.contains("[validate]"));
        assert!(content.contains("[scan]"));
    }
}
