//! Data models for the sync command.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Configuration for custom placeholder templates.
///
/// Loaded from the JSON file named by `--template-config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderConfig {
    /// Declared for editor tooling; evnx does not read it.
    ///
    /// Present as a field rather than ignored silently because the published
    /// example config carries it, and serde drops unknown keys without a word —
    /// so an operator has no way to tell "accepted and unused" from "misspelled
    /// and dropped". Round-tripping it also means `Serialize` does not delete it
    /// from a file evnx rewrites.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// A human-readable note. Same reasoning as [`schema`](Self::schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Pattern-to-placeholder mappings (regex pattern → placeholder string).
    ///
    /// ⚠️ **An `IndexMap`, not a `HashMap`, and that is load-bearing.** More than
    /// one pattern can match one key, and a `HashMap` iterates in an order that
    /// Rust randomises per process — so the winner changed between runs of the
    /// same command, and the winner is written into `.env.example`, a committed
    /// file. Eight identical runs produced four different answers.
    ///
    /// Declaration order is the last tie-break in
    /// [`generate_placeholder`](super::placeholder::generate_placeholder), which
    /// needs the order the file was written in.
    #[serde(default)]
    pub patterns: IndexMap<String, String>,

    /// Default placeholder for unmatched keys
    #[serde(default = "default_placeholder")]
    pub default: String,

    /// Keys whose **real value** is written to the template instead of a
    /// placeholder.
    ///
    /// For the part of a `.env` that is configuration rather than credentials —
    /// `APP_NAME`, `LOG_LEVEL`, `ENVIRONMENT` — where the actual value is the
    /// useful example and `YOUR_VALUE_HERE` is strictly worse. It also makes
    /// `sync --reverse` deliver working defaults to a teammate's `.env`.
    ///
    /// ⚠️ **Guarded, not trusted.** A listed key whose value looks like a
    /// credential fails the sync by name rather than being copied — see
    /// [`check_allow_actual`](super::executor::check_allow_actual). Without that
    /// this field is a documented way to commit a live secret.
    #[serde(default)]
    pub allow_actual: Vec<String>,

    /// Documented in `sync-configuration.mdx`, and **not implemented here.**
    ///
    /// Naming policy is real, but it is set by `--naming-policy` or `[sync]
    /// naming_policy` in `.evnx.toml`, not by this file. Kept as a field so
    /// [`PlaceholderConfig::from_path`] can say so, instead of serde discarding
    /// it and the operator believing a policy is in force that is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming_convention: Option<serde_json::Value>,
}

fn default_placeholder() -> String {
    String::from("YOUR_VALUE_HERE")
}

impl PlaceholderConfig {
    /// Load configuration from a JSON file.
    ///
    /// # Errors
    ///
    /// The file cannot be read or is not valid JSON, or a key of `patterns` is
    /// not a valid regular expression.
    ///
    /// ⚠️ That last one used to be silent. `generate_placeholder` compiled each
    /// pattern with `if let Ok(re)`, so an expression with an unbalanced
    /// parenthesis was **skipped**, the key fell through to `default`, and the
    /// operator got a template that looked fine and did not follow their rules.
    /// `evnx scan` already refuses to run an uncompilable `--pattern` for the
    /// same reason; this brings the two into line.
    pub fn from_path<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read placeholder config file: {}", e))?;
        let config: Self = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse placeholder config as JSON: {}", e))?;
        config.validate(path)?;
        Ok(config)
    }

    /// Reject what would otherwise fail silently, and say what is ignored.
    fn validate(&self, path: &std::path::Path) -> anyhow::Result<()> {
        for pattern in self.patterns.keys() {
            regex::Regex::new(&format!("(?i){}", pattern)).map_err(|e| {
                anyhow::anyhow!(
                    "{} declares a placeholder pattern that is not a valid regular \
                     expression: {pattern:?}\n  {e}\n  \
                     Nothing was written. Fix the pattern, or remove it.",
                    path.display()
                )
            })?;
        }

        // A warning rather than an error: the field is in the published example,
        // so refusing it would fail every config copied from the docs.
        if self.naming_convention.is_some() {
            crate::utils::ui::warning(format!(
                "{} sets `naming_convention`, which evnx does not read from this \
                 file. Use --naming-policy, or [sync] naming_policy in .evnx.toml.",
                path.display()
            ));
        }

        Ok(())
    }
}

impl Default for PlaceholderConfig {
    fn default() -> Self {
        Self {
            schema: None,
            description: None,
            patterns: IndexMap::new(),
            default: String::from("YOUR_VALUE_HERE"),
            allow_actual: Vec::new(),
            naming_convention: None,
        }
    }
}

/// Result of a sync operation (for dry-run preview)
#[derive(Debug, Clone)]
pub struct SyncPreview {
    pub target_file: String,
    pub action: SyncAction,
    pub variables: Vec<VarChange>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncAction {
    Add,
    Update,
    Remove,
    NoChange,
}

#[derive(Debug, Clone)]
pub struct VarChange {
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: String,
    pub is_placeholder: bool,
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_config_default() {
        let config = PlaceholderConfig::default();
        assert_eq!(config.default, "YOUR_VALUE_HERE");
        assert!(config.patterns.is_empty());
        assert!(config.allow_actual.is_empty());
    }

    #[test]
    fn test_placeholder_config_serialization() {
        let config = PlaceholderConfig {
            patterns: IndexMap::from([("API_.*".to_string(), "api-key".to_string())]),
            default: "custom".to_string(),
            allow_actual: vec!["PUBLIC_KEY".to_string()],
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: PlaceholderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.default, "custom");
        assert!(parsed.patterns.contains_key("API_.*"));
        assert_eq!(parsed.allow_actual, vec!["PUBLIC_KEY"]);
    }

    #[test]
    fn test_placeholder_config_from_path_success() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"default": "TEST_DEFAULT", "patterns": {{"KEY_.*": "test"}}}}"#
        )
        .unwrap();

        let config = PlaceholderConfig::from_path(file.path()).unwrap();
        assert_eq!(config.default, "TEST_DEFAULT");
        assert!(config.patterns.contains_key("KEY_.*"));
    }

    #[test]
    fn test_placeholder_config_from_path_invalid_json() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        // writeln!(file, r#"{"invalid": json}"#).unwrap();
        writeln!(file, r#"{{"invalid": json}}"#).unwrap();

        let result = PlaceholderConfig::from_path(file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("JSON"));
    }

    #[test]
    fn test_placeholder_config_from_path_missing_file() {
        let result = PlaceholderConfig::from_path("/nonexistent/path.json");
        assert!(result.is_err());
    }

    // SyncPreview and VarChange are simple data structs - minimal tests needed
    #[test]
    fn test_sync_action_equality() {
        assert_eq!(SyncAction::Add, SyncAction::Add);
        assert_ne!(SyncAction::Add, SyncAction::Remove);
    }

    #[test]
    fn test_var_change_clone() {
        let change = VarChange {
            key: "TEST".to_string(),
            old_value: Some("old".to_string()),
            new_value: "new".to_string(),
            is_placeholder: true,
        };
        let cloned = change.clone();
        assert_eq!(cloned.key, change.key);
        assert_eq!(cloned.is_placeholder, change.is_placeholder);
    }
}
