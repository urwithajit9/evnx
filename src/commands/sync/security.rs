//! Security-focused utilities: naming validation, audit logging.
//!
//! The `.env` permission check moved to `core::permissions::warn_if_readable`
//! in 0.10.12 (I-276): it was worded `(0o644)` here and differently in
//! `safe_write`, and a reverse sync onto a readable `.env` printed both.

use crate::cli::NamingPolicy;
use crate::utils::ui;

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

/// One line on stderr naming the keys written to the template with real
/// values. DEC-S1: printed whenever that happens, not only with `--verbose`,
/// and not hidden by `-q` — it records a choice with consequences. It is a
/// line in the run's output, not a log kept anywhere else (D-83).
pub fn log_security_event(keys: &[String], template: &str) {
    if !keys.is_empty() {
        eprintln!(
            "[AUDIT] actual values written to {template} for: {}",
            keys.join(", ")
        );
    }
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
    fn test_log_security_event_no_panic() {
        // Just ensure the function doesn't panic
        log_security_event(&["KEY1".to_string(), "KEY2".to_string()], ".env.example");
        log_security_event(&[], ".env.example");
    }
}
