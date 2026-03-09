//! Data models for the sync command.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for custom placeholder templates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderConfig {
    /// Pattern-to-placeholder mappings (regex pattern → placeholder string)
    #[serde(default)]
    pub patterns: HashMap<String, String>,

    /// Default placeholder for unmatched keys
    #[serde(default = "default_placeholder")]
    pub default: String,

    /// Keys that should always use actual values (use with extreme caution)
    #[serde(default)]
    pub allow_actual: Vec<String>,
}

fn default_placeholder() -> String {
    String::from("YOUR_VALUE_HERE")
}

impl PlaceholderConfig {
    /// Load configuration from a JSON file
    pub fn from_path<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read placeholder config file: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse placeholder config as JSON: {}", e))
    }
}

impl Default for PlaceholderConfig {
    fn default() -> Self {
        Self {
            patterns: HashMap::new(),
            default: String::from("YOUR_VALUE_HERE"),
            allow_actual: Vec::new(),
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
            patterns: HashMap::from([("API_.*".to_string(), "api-key".to_string())]),
            default: "custom".to_string(),
            allow_actual: vec!["PUBLIC_KEY".to_string()],
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
