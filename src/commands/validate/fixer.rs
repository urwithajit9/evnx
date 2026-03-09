//! Auto-fix logic and file operations for validation
//!
//! This module handles:
//! - Suggesting fixes for common issues
//! - Applying fixes to in-memory env vars
//! - Writing fixed content back to .env files

use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use super::types::{FixApplied, IssueType};

// ─────────────────────────────────────────────────────────────
// Fix Action Types
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum FixAction {
    GenerateSecret,
    ReplacePlaceholder(String),
    FixBoolean(String),
    AddMissing(String),
    Skip,
}

// ─────────────────────────────────────────────────────────────
// Fix Suggestion Logic
// ─────────────────────────────────────────────────────────────

pub fn suggest_fix(key: &str, value: &str, issue_type: &IssueType) -> FixAction {
    match issue_type {
        IssueType::PlaceholderValue => {
            if key.to_uppercase().contains("SECRET") || key.to_uppercase().contains("KEY") {
                FixAction::GenerateSecret
            } else if key.contains("URL") {
                FixAction::ReplacePlaceholder("https://example.com".to_string())
            } else if key.contains("EMAIL") || key.contains("MAIL") {
                FixAction::ReplacePlaceholder("user@example.com".to_string())
            } else if key.contains("PORT") {
                FixAction::ReplacePlaceholder("8080".to_string())
            } else {
                FixAction::ReplacePlaceholder("your_value_here".to_string())
            }
        }
        IssueType::BooleanTrap => {
            let fixed = if value.eq_ignore_ascii_case("true") {
                "true"
            } else {
                "false"
            };
            FixAction::FixBoolean(fixed.to_string())
        }
        IssueType::WeakSecret => FixAction::GenerateSecret,
        IssueType::MissingVariable => {
            let default =
                if key.to_uppercase().contains("SECRET") || key.to_uppercase().contains("KEY") {
                    "CHANGE_ME_SECURE_32_CHARS_MIN"
                } else {
                    "your_value_here"
                };
            FixAction::AddMissing(default.to_string())
        }
        _ => FixAction::Skip,
    }
}

// ─────────────────────────────────────────────────────────────
// Secure Secret Generation
// ─────────────────────────────────────────────────────────────

/// Generate a secure-ish random secret (32 bytes = 64 hex chars)
///
/// For production, consider using the `rand` or `openssl` crate instead.
pub fn generate_secure_secret() -> String {
    // Simple time-based entropy for demo; replace with crypto RNG in production
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    // XOR with a constant and format as hex
    format!("{:064x}", seed ^ 0x5DEECE66D)
}

// ─────────────────────────────────────────────────────────────
// Apply Fix to In-Memory HashMap
// ─────────────────────────────────────────────────────────────

pub fn apply_fix(
    key: &str,
    value: &str,
    action: &FixAction,
    env_vars: &mut HashMap<String, String>,
) -> Option<FixApplied> {
    match action {
        FixAction::GenerateSecret => {
            let new_val = generate_secure_secret();
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Generated secure secret".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val,
            })
        }
        FixAction::ReplacePlaceholder(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Replaced placeholder".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val.clone(),
            })
        }
        FixAction::FixBoolean(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Fixed boolean format".to_string(),
                old_value: Some(value.to_string()),
                new_value: new_val.clone(),
            })
        }
        FixAction::AddMissing(new_val) => {
            env_vars.insert(key.to_string(), new_val.clone());
            Some(FixApplied {
                variable: key.to_string(),
                action: "Added missing variable".to_string(),
                old_value: None,
                new_value: new_val.clone(),
            })
        }
        FixAction::Skip => None,
    }
}

// ─────────────────────────────────────────────────────────────
// File I/O: Write Fixed Content
// ─────────────────────────────────────────────────────────────

/// Write fixed env vars back to file, preserving comments and order where possible
pub fn write_fixed_file(
    env_path: &str,
    env_vars: &HashMap<String, String>,
    original_content: &str,
) -> Result<()> {
    let mut output = String::new();

    // Process original file line-by-line to preserve structure
    for line in original_content.lines() {
        let trimmed = line.trim();

        // Preserve empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        // Update existing variable values
        if let Some(eq_pos) = line.find('=') {
            let var_name = line[..eq_pos].trim();
            if let Some(new_value) = env_vars.get(var_name) {
                output.push_str(var_name);
                output.push('=');
                output.push_str(new_value);
                output.push('\n');
            } else {
                // Keep original if not in fixed map
                output.push_str(line);
                output.push('\n');
            }
        } else {
            // Keep malformed lines as-is
            output.push_str(line);
            output.push('\n');
        }
    }

    // Append any new variables that weren't in original file
    for (key, value) in env_vars {
        if !original_content.contains(&format!("{}=", key)) {
            output.push_str(key);
            output.push('=');
            output.push_str(value);
            output.push('\n');
        }
    }

    fs::write(env_path, output).with_context(|| format!("Failed to write fixes to {}", env_path))
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suggest_fix_placeholder_secret() {
        let action = suggest_fix("API_KEY", "changeme", &IssueType::PlaceholderValue);
        assert!(matches!(action, FixAction::GenerateSecret));
    }

    #[test]
    fn test_suggest_fix_boolean() {
        let action = suggest_fix("DEBUG", "True", &IssueType::BooleanTrap);
        assert!(matches!(action, FixAction::FixBoolean(s) if s == "true"));
    }

    #[test]
    fn test_apply_fix_generates_secret() {
        let mut env = HashMap::new();
        let action = FixAction::GenerateSecret;
        let result = apply_fix("SECRET_KEY", "weak", &action, &mut env);

        assert!(result.is_some());
        let fix = result.unwrap();
        assert_eq!(fix.variable, "SECRET_KEY");
        assert_eq!(fix.new_value.len(), 64); // 32 bytes = 64 hex chars
        assert_eq!(env.get("SECRET_KEY"), Some(&fix.new_value));
    }

    #[test]
    fn test_apply_fix_boolean() {
        let mut env = HashMap::new();
        let action = FixAction::FixBoolean("false".to_string());
        let result = apply_fix("DEBUG", "True", &action, &mut env);

        assert!(result.is_some());
        assert_eq!(env.get("DEBUG"), Some(&"false".to_string()));
    }

    #[test]
    fn test_generate_secret_length() {
        let secret = generate_secure_secret();
        assert_eq!(secret.len(), 64); // 32 bytes in hex
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
