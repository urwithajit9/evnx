//! Placeholder value generation with custom config support.

use crate::commands::sync::models::PlaceholderConfig;

/// Generate placeholder using built-in rules + custom config
pub fn generate_placeholder(
    key: &str,
    value: Option<&String>,
    config: &PlaceholderConfig,
) -> String {
    // The project asked for this key's real value to go into the template.
    //
    // ⚠️ Safe only because `executor::check_allow_actual` has already refused the
    // whole sync if any listed value looks like a credential. Reaching here means
    // the value was examined and is configuration, not a secret.
    if config.allow_actual.iter().any(|k| k == key) {
        if let Some(v) = value {
            return v.clone();
        }
    }

    // Then the project's own patterns, most specific first.
    if let Some(placeholder) = best_pattern(key, config) {
        return placeholder;
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

/// The placeholder of the most specific pattern matching `key`.
///
/// # Why this is not "the first one that matches"
///
/// It was, over a `HashMap`, and more than one pattern routinely matches one key
/// — `STRIPE_SECRET_KEY` matches `SECRET`, `_KEY$` and `^STRIPE_` at once. Rust
/// randomises `HashMap` iteration per process, so the winner changed from run to
/// run, and the winner is written into `.env.example`, **a committed file**.
/// Eight identical runs produced four different answers: every one of the four
/// patterns won at least once. So `evnx sync` produced a spurious `git diff` on
/// a project whose `.env` had not changed at all.
///
/// # The order
///
/// 1. **An exact key match**, which is the project naming the variable outright.
/// 2. **The longest pattern.** A crude specificity measure, but the right way
///    round for the expressions people actually write: `.*_SECRET_KEY$` beats
///    `SECRET`, and anything beats `.*`. It also settles the anchored spelling of
///    an exact match without special-casing it — `^STRIPE_SECRET_KEY$` is longer
///    than every pattern that could match the same key by accident.
/// 3. **Declaration order**, which is why `patterns` is an `IndexMap`. Two
///    equally long patterns matching one key is a genuine ambiguity, and the
///    file is the only place that expresses which the author meant.
///
/// Every rung is total, so the result depends on the config file and the key and
/// nothing else.
fn best_pattern(key: &str, config: &PlaceholderConfig) -> Option<String> {
    config
        .patterns
        .iter()
        .enumerate()
        .filter(|(_, (pattern, _))| {
            // An expression that does not compile cannot match. It is also
            // refused at load time by `PlaceholderConfig::from_path`, so this is
            // only reachable for a config built in memory.
            regex::Regex::new(&format!("(?i){}", pattern))
                .map(|re| re.is_match(key))
                .unwrap_or(false)
        })
        .max_by_key(|(index, (pattern, _))| {
            (
                pattern.eq_ignore_ascii_case(key),
                pattern.len(),
                // `max_by_key` keeps the last maximum, so declaration order has
                // to be reversed to make the *first* declaration win.
                std::cmp::Reverse(*index),
            )
        })
        .map(|(_, (_, placeholder))| placeholder.clone())
}

/// Check if a value looks like a placeholder
///
/// ⚠️ This consulted `config.default` and a list of hardcoded strings, and
/// **ignored `config.patterns` entirely** — so a project whose convention is
/// `<set-me>` had every one of its own placeholders reported as a real value.
/// Display only, in the `--dry-run` preview, which is why it went unnoticed; but
/// the preview is exactly where someone decides whether a template is ready.
pub fn is_placeholder_value(value: &str, config: &PlaceholderConfig) -> bool {
    config.patterns.values().any(|p| p == value)
        || value == config.default
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

    /// Four patterns, all matching `STRIPE_SECRET_KEY`. Which one wins used to
    /// depend on `HashMap` iteration order, and the answer is written into a
    /// committed file. These pin the order instead.
    fn overlapping() -> PlaceholderConfig {
        let mut config = PlaceholderConfig::default();
        for (pattern, placeholder) in [
            ("SECRET", "<any-secret>"),
            ("_KEY$", "<any-key>"),
            ("^STRIPE_", "<stripe-thing>"),
            ("STRIPE_SECRET_KEY", "<the-exact-key>"),
        ] {
            config
                .patterns
                .insert(pattern.to_string(), placeholder.to_string());
        }
        config
    }

    #[test]
    fn an_exact_key_match_beats_every_regex() {
        let config = overlapping();
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<the-exact-key>"
        );
        // Declaration order alone would have picked `<any-secret>`, so this is
        // not the first rung passing by accident.
        assert_eq!(
            config.patterns.keys().next().map(String::as_str),
            Some("SECRET")
        );
    }

    #[test]
    fn the_longest_pattern_wins_when_none_is_exact() {
        let mut config = overlapping();
        config.patterns.shift_remove("STRIPE_SECRET_KEY");
        // `^STRIPE_` (8) over `SECRET` (6) and `_KEY$` (5).
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<stripe-thing>"
        );
    }

    #[test]
    fn declaration_order_settles_equally_long_patterns() {
        let mut config = PlaceholderConfig::default();
        // Both six characters, both match, so only the file can say which.
        config
            .patterns
            .insert("SECRET".to_string(), "<first>".to_string());
        config
            .patterns
            .insert("^STRIP".to_string(), "<second>".to_string());
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<first>"
        );

        // Reversed in the file, reversed in the answer — proving it reads the
        // order rather than happening to agree with it.
        let mut reversed = PlaceholderConfig::default();
        reversed
            .patterns
            .insert("^STRIP".to_string(), "<second>".to_string());
        reversed
            .patterns
            .insert("SECRET".to_string(), "<first>".to_string());
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &reversed),
            "<second>"
        );
    }

    #[test]
    fn a_pattern_matching_nothing_is_ignored_rather_than_winning_on_length() {
        let mut config = PlaceholderConfig::default();
        config.patterns.insert(
            "A_VERY_LONG_PATTERN_THAT_MATCHES_NOTHING".to_string(),
            "<no>".to_string(),
        );
        config
            .patterns
            .insert("KEY".to_string(), "<yes>".to_string());
        assert_eq!(generate_placeholder("API_KEY", None, &config), "<yes>");
    }

    #[test]
    fn allow_actual_writes_the_real_value_and_outranks_a_matching_pattern() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("LOG_LEVEL".to_string(), "<a-level>".to_string());
        config.allow_actual.push("LOG_LEVEL".to_string());

        let value = "debug".to_string();
        assert_eq!(
            generate_placeholder("LOG_LEVEL", Some(&value), &config),
            "debug"
        );
    }

    #[test]
    fn allow_actual_with_no_value_falls_back_to_a_placeholder() {
        // The reverse direction has no real value to copy. It must not produce an
        // empty string, which would read as "this variable is blank".
        let mut config = PlaceholderConfig::default();
        config.allow_actual.push("APP_NAME".to_string());
        assert_eq!(
            generate_placeholder("APP_NAME", None, &config),
            "YOUR_VALUE_HERE"
        );
    }

    #[test]
    fn a_projects_own_placeholder_is_recognised_as_a_placeholder() {
        // ⚠️ `is_placeholder_value` ignored `patterns` entirely, so a project
        // whose convention is `<set-me>` had all of its own placeholders reported
        // as real values in the dry-run preview.
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("API_.*".to_string(), "<set-me>".to_string());

        assert!(is_placeholder_value("<set-me>", &config));
        assert!(!is_placeholder_value("sk_live_abc123", &config));
    }

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
        let config = PlaceholderConfig {
            default: "CUSTOM_PLACEHOLDER".to_string(),
            ..Default::default()
        };

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
