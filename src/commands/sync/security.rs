//! Security-focused utilities: permissions, naming validation, audit logging.

use anyhow::Result;
// use colored::*;

use crate::cli::NamingPolicy;
use crate::utils::ui;

/// Check if .env file has appropriate permissions (0o600 recommended)
pub fn check_env_permissions(path: &str) -> Result<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()), // File doesn't exist yet, skip check
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = metadata.permissions().mode();
        // Check if group or others have any permissions
        if perms & 0o077 != 0 {
            ui::warning(format!(
                "{} has permissive permissions (0o{:o}). Consider: chmod 600 {}",
                path,
                perms & 0o777,
                path
            ));
            ui::info("Overly permissive .env files may expose secrets to other users");
        }
    }

    // On Windows, we could check ACLs, but that's complex; skip for now
    Ok(())
}

/// Validate environment variable naming convention
pub fn validate_var_name(key: &str, policy: NamingPolicy) -> Result<(), String> {
    // Standard convention: UPPERCASE with underscores, optional prefix
    let is_standard = regex::Regex::new(r"^[A-Z][A-Z0-9_]*$")
        .ok()
        .map(|re| re.is_match(key))
        .unwrap_or(true); // If regex fails, be permissive

    if !is_standard {
        match policy {
            NamingPolicy::Error => {
                return Err(format!(
                    "Non-standard variable name '{}'. Expected: UPPERCASE_WITH_UNDERSCORES",
                    key
                ));
            }
            NamingPolicy::Warn => {
                ui::warning(format!(
                    "Variable '{}' doesn't follow convention (UPPERCASE_WITH_UNDERSCORES)",
                    key
                ));
            }
            NamingPolicy::Ignore => {} // Silent
        }
    }
    Ok(())
}

/// Log a security audit event (for actual-values mode)
pub fn log_security_event(keys: &[String], verbose: bool) {
    if verbose {
        eprintln!(
            "[AUDIT] User added actual values to .env.example for keys: {:?}",
            keys
        );
    }
    // In production: send to structured audit log
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_var_name_valid_names() {
        assert!(validate_var_name("DATABASE_URL", NamingPolicy::Warn).is_ok());
        assert!(validate_var_name("API_KEY_123", NamingPolicy::Warn).is_ok());
        assert!(validate_var_name("MY_APP_DEBUG", NamingPolicy::Warn).is_ok());
        assert!(validate_var_name("A", NamingPolicy::Warn).is_ok()); // Minimal valid
    }

    #[test]
    fn test_validate_var_name_invalid_names() {
        // camelCase
        assert!(validate_var_name("camelCase", NamingPolicy::Ignore).is_ok());
        assert!(validate_var_name("camelCase", NamingPolicy::Warn).is_ok()); // warns but ok
        assert!(validate_var_name("camelCase", NamingPolicy::Error).is_err());

        // lowercase
        assert!(validate_var_name("lowercase", NamingPolicy::Error).is_err());

        // starts with number
        assert!(validate_var_name("123_VAR", NamingPolicy::Error).is_err());

        // contains hyphen
        assert!(validate_var_name("VAR-NAME", NamingPolicy::Error).is_err());
    }

    #[test]
    fn test_validate_var_name_edge_cases() {
        // Empty string
        assert!(validate_var_name("", NamingPolicy::Error).is_err());

        // Underscore only
        assert!(validate_var_name("_", NamingPolicy::Warn).is_ok()); // Matches regex ^[A-Z]

        // Leading underscore (non-standard but common)
        assert!(validate_var_name("_PRIVATE", NamingPolicy::Warn).is_ok()); // warns
    }

    #[test]
    fn test_check_env_permissions_nonexistent_file() {
        // Should not error if file doesn't exist
        let result = check_env_permissions("/nonexistent/.env");
        assert!(result.is_ok());
    }

    // Note: Testing actual permission checks requires creating files with specific modes
    // This is complex and platform-specific; integration tests are better suited

    #[test]
    fn test_log_security_event_no_panic() {
        // Just ensure the function doesn't panic
        log_security_event(&["KEY1".to_string(), "KEY2".to_string()], true);
        log_security_event(&[], false);
    }
}
