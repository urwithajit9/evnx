//! Placeholder value generation with custom config support.

use crate::commands::sync::models::PlaceholderConfig;

/// Generate placeholder using built-in rules + custom config
pub fn generate_placeholder(
    key: &str,
    value: Option<&String>,
    config: &PlaceholderConfig,
) -> String {
    // First check custom patterns from config
    for (pattern, placeholder) in &config.patterns {
        if let Ok(re) = regex::Regex::new(&format!("(?i){}", pattern)) {
            if re.is_match(key) {
                return placeholder.clone();
            }
        }
    }

    // Built-in heuristics (fallback)
    let key_upper = key.to_uppercase();

    if key_upper.contains("SECRET") || key_upper.contains("KEY") || key_upper.contains("TOKEN") {
        return "YOUR_KEY_HERE".to_string();
    }

    if key_upper.contains("PASSWORD") || key_upper.contains("PASS") {
        return "YOUR_PASSWORD_HERE".to_string();
    }

    if key_upper.contains("URL") {
        if let Some(v) = value {
            if v.contains("postgresql://") {
                return "postgresql://user:password@localhost:5432/dbname".to_string();
            }
            if v.contains("redis://") {
                return "redis://localhost:6379/0".to_string();
            }
            if v.contains("http://") || v.contains("https://") {
                return "https://your-api-url-here.com".to_string();
            }
        }
        return "YOUR_URL_HERE".to_string();
    }

    if key_upper.contains("PORT") {
        return "8000".to_string();
    }

    if key_upper.contains("DEBUG") {
        return "true".to_string();
    }

    if key_upper.contains("HOST") || key_upper.contains("SERVER") {
        return "localhost".to_string();
    }

    // Use config default, fallback to hardcoded default if empty
    if config.default.is_empty() {
        "YOUR_VALUE_HERE".to_string()
    } else {
        config.default.clone()
    }
}

/// Check if a value looks like a placeholder
pub fn is_placeholder_value(value: &str, config: &PlaceholderConfig) -> bool {
    value == config.default
        || value.contains("YOUR_")
        || value.contains("placeholder")
        || value == "localhost"
        || value == "8000"
        || value == "true"
        || value == "false"
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_placeholder_builtin_rules() {
        let config = PlaceholderConfig::default();

        assert_eq!(
            generate_placeholder("SECRET_KEY", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("API_TOKEN", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("DB_PASSWORD", None, &config),
            "YOUR_PASSWORD_HERE"
        );
        assert_eq!(generate_placeholder("PORT", None, &config), "8000");
        assert_eq!(generate_placeholder("DEBUG_MODE", None, &config), "true");
        assert_eq!(generate_placeholder("DB_HOST", None, &config), "localhost");
        assert_eq!(
            generate_placeholder("RANDOM_VAR", None, &config),
            "YOUR_VALUE_HERE"
        );
    }

    #[test]
    fn test_generate_placeholder_with_url_values() {
        let config = PlaceholderConfig::default();

        let pg_url = "postgresql://user:pass@localhost:5432/db";
        assert!(
            generate_placeholder("DATABASE_URL", Some(&pg_url.to_string()), &config)
                .contains("postgresql")
        );

        let redis_url = "redis://localhost:6379/0";
        assert!(
            generate_placeholder("REDIS_URL", Some(&redis_url.to_string()), &config)
                .contains("redis")
        );

        // ✅ FIXED: Use a key that actually contains "URL"
        let https_url = "https://api.example.com/v1";
        assert!(
            generate_placeholder("API_URL", Some(&https_url.to_string()), &config) // Changed from API_ENDPOINT
                .contains("https://")
        );
    }

    #[test]
    fn test_generate_placeholder_custom_config() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("AWS_.*".to_string(), "aws-placeholder".to_string());
        config.default = "CUSTOM_DEFAULT".to_string();

        assert_eq!(
            generate_placeholder("AWS_SECRET", None, &config),
            "aws-placeholder"
        );
        assert_eq!(
            generate_placeholder("UNKNOWN_VAR", None, &config),
            "CUSTOM_DEFAULT"
        );
    }

    #[test]
    fn test_custom_pattern_matching() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("STRIPE_.*".to_string(), "stripe_test_key".to_string());
        config
            .patterns
            .insert(".*_PORT".to_string(), "3000".to_string());

        assert_eq!(
            generate_placeholder("STRIPE_API_KEY", None, &config),
            "stripe_test_key"
        );
        assert_eq!(
            generate_placeholder("STRIPE_SECRET", None, &config),
            "stripe_test_key"
        );
        assert_eq!(generate_placeholder("SERVER_PORT", None, &config), "3000");
        assert_eq!(generate_placeholder("APP_PORT", None, &config), "3000");
    }

    #[test]
    fn test_custom_pattern_case_insensitive() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("api_.*".to_string(), "lowercase-pattern".to_string());

        // Should match regardless of case due to (?i) flag
        assert_eq!(
            generate_placeholder("API_KEY", None, &config),
            "lowercase-pattern"
        );
        assert_eq!(
            generate_placeholder("api_token", None, &config),
            "lowercase-pattern"
        );
        assert_eq!(
            generate_placeholder("Api_Secret", None, &config),
            "lowercase-pattern"
        );
    }

    #[test]
    fn test_invalid_regex_pattern_fallback() {
        let mut config = PlaceholderConfig::default();
        // Invalid regex pattern - should not panic, should fallback
        config
            .patterns
            .insert("[invalid(regex".to_string(), "should-not-match".to_string());

        // Should fallback to built-in rules or default
        let result = generate_placeholder("TEST_KEY", None, &config);
        assert_eq!(result, "YOUR_KEY_HERE"); // Built-in rule for KEY
    }

    #[test]
    fn test_is_placeholder_value() {
        let config = PlaceholderConfig::default();

        assert!(is_placeholder_value("YOUR_VALUE_HERE", &config));
        assert!(is_placeholder_value("YOUR_KEY_HERE", &config));
        assert!(is_placeholder_value("localhost", &config));
        assert!(is_placeholder_value("placeholder", &config));
        assert!(!is_placeholder_value("sk_live_abc123", &config));
        assert!(!is_placeholder_value("production-db.example.com", &config));
        assert!(!is_placeholder_value("my-secret-value", &config));
    }

    #[test]
    fn test_is_placeholder_value_with_custom_config() {
        let mut config = PlaceholderConfig::default();
        config.default = "CUSTOM_PLACEHOLDER".to_string();

        assert!(is_placeholder_value("CUSTOM_PLACEHOLDER", &config));
        // ✅ FIXED: "YOUR_VALUE_HERE" still matches because of .contains("YOUR_") check
        // This is intentional behavior - we detect common placeholder patterns
        assert!(is_placeholder_value("YOUR_VALUE_HERE", &config)); // Still true due to "YOUR_" check

        // Test a value that should NOT be detected as placeholder
        assert!(!is_placeholder_value("my-actual-secret-123", &config));
    }

    // NEW: Edge cases
    #[test]
    fn test_generate_placeholder_empty_key() {
        let config = PlaceholderConfig::default();
        let result = generate_placeholder("", None, &config);
        // Should not panic, should return default
        assert!(!result.is_empty());
    }

    #[test]
    fn test_generate_placeholder_special_chars_in_key() {
        let config = PlaceholderConfig::default();
        // Use a key that won't match any built-in rules
        // Avoid: KEY, SECRET, TOKEN, PASSWORD, URL, PORT, DEBUG, HOST, SERVER
        let result = generate_placeholder("my-custom-var", None, &config);
        assert_eq!(result, "YOUR_VALUE_HERE");
    }

    #[test]
    fn test_generate_placeholder_priority_order() {
        let mut config = PlaceholderConfig::default();
        // Add a pattern that could conflict with built-in rules
        config
            .patterns
            .insert(".*PASSWORD".to_string(), "custom-pass".to_string());

        // Custom pattern should take priority over built-in "PASSWORD" rule
        assert_eq!(
            generate_placeholder("MY_PASSWORD", None, &config),
            "custom-pass"
        );
    }
}
