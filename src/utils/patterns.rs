use crate::core::config::PatternRule;
/// Secret pattern detection for scanning .env files
///
/// This module contains regex patterns and entropy calculation for detecting
/// accidentally committed secrets. Patterns are based on real-world secret formats
/// from AWS, Stripe, GitHub, OpenAI, and other major services.
use lazy_static::lazy_static;
use regex::Regex;

/// Confidence level for secret detection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => write!(f, "high"),
            Confidence::Medium => write!(f, "medium"),
            Confidence::Low => write!(f, "low"),
        }
    }
}

lazy_static! {
    /// AWS Secret Access Key — the one regex that is **not** a rule.
    ///
    /// ⚠️ Every other provider pattern moved into [`builtin_rules`]. This one
    /// cannot: its shape is just 40 base64 characters, so on its own it matches
    /// any 40-character token. It is only a finding when the variable's *name*
    /// says AWS and the value's entropy is high, which a `PatternRule` has no way
    /// to express — see [`detect_heuristic`].
    ///
    /// The others were deleted rather than kept beside the rules. Keeping both
    /// would have rebuilt the duplication this change removed: two copies of one
    /// pattern, with unit tests asserting the copy production no longer reads.
    pub static ref AWS_SECRET_KEY: Regex = Regex::new(r"[0-9a-zA-Z/+=]{40}").unwrap();
}

/// The detectors evnx ships, as ordinary [`PatternRule`]s.
///
/// ⚠️ These were a hand-written `if` chain of `Regex::is_match` calls, and a
/// duplicate `SecretPattern` table that nothing ever read. Two consequences:
///
/// * **Built-ins were weaker than user rules.** `[[scan.patterns]]` goes through
///   `PatternSet` and is matched against whole lines; the chain only ever saw
///   tokens longer than 20 characters. `-----BEGIN RSA PRIVATE KEY-----` is a
///   phrase, never one token, so the private-key detector could not fire in any
///   file — including the `.pem` files private keys live in.
/// * **Collisions were resolved by source order.** Anthropic was checked after
///   OpenAI, so a permissive OpenAI pattern silently claimed `sk-ant-…` keys and
///   named the wrong provider to revoke at.
///
/// As data they share one engine, one precedence rule, and the config surface
/// that lets a project disable or re-rate any of them.
///
/// Two checks are deliberately **not** here, because neither is a pattern:
/// the AWS secret key (gated on the variable's *name* plus entropy, since its
/// shape is just 40 base64 characters) and the generic high-entropy fallback.
/// Both live in [`detect_heuristic`] and stay token-scoped — running an entropy
/// threshold over whole lines would flag every minified bundle.
pub fn builtin_rules() -> Vec<PatternRule> {
    vec![
        PatternRule::builtin(
            "AWS Access Key",
            r"AKIA[0-9A-Z]{16}",
            "high",
            Some("https://console.aws.amazon.com/iam"),
        ),
        PatternRule::builtin(
            "Stripe Secret Key (LIVE)",
            r"sk_live_[0-9a-zA-Z]{24,}",
            "high",
            Some("https://dashboard.stripe.com/apikeys"),
        ),
        PatternRule::builtin(
            "Stripe Secret Key (test)",
            r"sk_test_[0-9a-zA-Z]{24,}",
            "medium",
            Some("https://dashboard.stripe.com/apikeys"),
        ),
        // The three GitHub forms were three regexes behind one `||`; as data
        // they are one alternation with one name, which is what the output said
        // all along.
        PatternRule::builtin(
            "GitHub Token",
            r"\b(?:ghp|gho|ghu|ghs)_[A-Za-z0-9]{36,40}\b",
            "high",
            Some("https://github.com/settings/tokens"),
        ),
        // ⚠️ Two exclusive branches, not `(?:proj-)?`. Anthropic keys also begin
        // `sk-`, and a permissive form swallowed them.
        PatternRule::builtin(
            "OpenAI API Key",
            r"sk-(?:proj-[0-9a-zA-Z_-]{40,}|[0-9a-zA-Z]{48})",
            "high",
            Some("https://platform.openai.com/api-keys"),
        ),
        PatternRule::builtin(
            "Anthropic API Key",
            r"sk-ant-api[0-9]{2}-[0-9a-zA-Z\-_]{95}",
            "high",
            Some("https://console.anthropic.com/settings/keys"),
        ),
        // ⚠️ `[A-Z ]*`, not `[A-Z ]+ `. PKCS#8 writes a bare
        // `-----BEGIN PRIVATE KEY-----` with no algorithm — OpenSSL's default
        // since 3.0 — which the old form could not match.
        PatternRule::builtin(
            "Private Key",
            r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----",
            "high",
            None,
        ),
    ]
}

/// Calculate Shannon entropy of a string (for detecting high-entropy secrets)
///
/// Returns a value between 0.0 and ~6.0
/// - < 3.0: Low entropy (probably not a secret)
/// - 3.0-4.5: Medium entropy (could be a secret)
/// - > 4.5: High entropy (likely a secret)
pub fn calculate_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }

    let mut char_counts = std::collections::HashMap::new();
    for c in s.chars() {
        *char_counts.entry(c).or_insert(0) += 1;
    }

    let len = s.len() as f64;
    let mut entropy = 0.0;

    for count in char_counts.values() {
        let probability = (*count as f64) / len;
        entropy -= probability * probability.log2();
    }

    entropy
}

/// Check if a value looks like a placeholder (not a real secret)
pub fn is_placeholder(value: &str) -> bool {
    let v = value.trim();
    let lower = v.to_lowercase();

    // Exact known fake/example secrets
    const EXACT: &[&str] = &[
        "akiaiosfodnn7example",
        "wjalrxutnfemi/k7mdeng/bpxrficyexamplekey",
        "your_key_here",
        "your_api_key",
        "your_api_key_here",
        "your_secret_here",
        "your_token_here",
        "change_me",
        "changeme",
        "replace_me",
        "xxx",
        "xxxx",
        "xxxxx",
        "todo",
        "placeholder",
        "test",
        "testing",
        "dev",
        "development",
        "dummy",
        "fake",
        "sample",
    ];

    if EXACT.iter().any(|p| lower == *p) {
        return true;
    }

    // Structured placeholder patterns (safe substrings)
    const SUBSTRINGS: &[&str] = &[
        "change_me",
        "changeme",
        "your_key_here",
        "your_secret_here",
        "your_token_here",
        "replace_me",
        "generate-with",
    ];

    if SUBSTRINGS.iter().any(|p| lower.contains(p)) {
        return true;
    }

    // ✅ NEW: Pattern-based checks for common placeholder formats
    if is_placeholder_pattern(&lower) {
        return true;
    }

    false
}

/// Helper: Check for common placeholder patterns like example123, test456, dev_key
fn is_placeholder_pattern(value: &str) -> bool {
    const BASE_WORDS: &[&str] = &[
        "example",
        "test",
        "testing",
        "dev",
        "development",
        "dummy",
        "fake",
        "sample",
        "placeholder",
        "todo",
    ];

    // ✅ Use strip_prefix instead of starts_with + manual slicing
    for base in BASE_WORDS {
        if let Some(remainder) = value.strip_prefix(base) {
            // Allow: empty, digits only, or underscore + anything
            if remainder.is_empty()
                || remainder.chars().all(|c| c.is_ascii_digit())
                || remainder.starts_with('_')
            {
                return true;
            }
        }
    }

    // Pattern: your_* variations
    if value.starts_with("your_") || value.starts_with("your-") {
        return true;
    }

    // Pattern: all same character (xxx, *****, ####)
    if !value.is_empty() {
        let first = value.chars().next().unwrap();
        if value.chars().all(|c| c == first) && value.len() >= 3 {
            return true;
        }
    }

    // Pattern: simple alphanumeric placeholder like abc123, def456
    if value.len() == 6 && value.chars().all(|c| c.is_ascii_alphanumeric()) {
        let letters: usize = value.chars().filter(|c| c.is_ascii_alphabetic()).count();
        let digits: usize = value.chars().filter(|c| c.is_ascii_digit()).count();
        if letters == 3 && digits == 3 {
            return true;
        }
    }

    false
}
#[must_use]
pub fn is_sensitive_key(key: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "TOKEN",
        "API_KEY",
        "PRIVATE_KEY",
        "AUTH",
        "CREDENTIAL",
    ];
    let upper = key.to_uppercase();
    PATTERNS.iter().any(|p| upper.contains(p))
}

/// Detect if a value matches any secret pattern
///
/// Returns (pattern_name, confidence) if a match is found
/// The two checks that are **not** patterns.
///
/// ⚠️ Everything recognisable from the value alone now lives in
/// [`builtin_rules`] and runs through `PatternSet`. What is left cannot:
///
/// * **AWS secret access key** has no distinctive prefix — its shape is just 40
///   base64 characters — so it is gated on the variable's *name* containing AWS
///   and on entropy. As a bare pattern it matched any 40-character token.
///
///   The gate used to read `contains("AWS") || contains("SECRET")`, and a Stripe
///   key is 40-ish base64 characters inside `STRIPE_SECRET_KEY`, so it was
///   reported as an AWS key with a link to the IAM console. The `||` was the bug;
///   the other half of the fix is that this runs *after* every rule that can
///   name the provider, which `ScanRunner::best` now enforces by ranking a
///   remediation URL above one without.
///
/// * **Generic high entropy** is a last resort with no URL and low confidence.
///
/// ⚠️ Neither is offered to `scan_line`. An entropy threshold over whole lines
/// would flag every minified bundle and every base64 blob in a lockfile.
pub fn detect_heuristic(value: &str, key: &str) -> Option<(String, Confidence, Option<String>)> {
    if is_placeholder(value) {
        return None;
    }

    if AWS_SECRET_KEY.is_match(value) && key.to_uppercase().contains("AWS") {
        let entropy = calculate_entropy(value);
        if entropy > 4.5 {
            return Some((
                "AWS Secret Access Key".to_string(),
                Confidence::Medium,
                Some("https://console.aws.amazon.com/iam".to_string()),
            ));
        }
    }

    if value.len() >= 32 {
        let entropy = calculate_entropy(value);
        if entropy > 4.8 {
            return Some((
                "High-entropy string (possible secret)".to_string(),
                Confidence::Low,
                None,
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy() {
        // Low entropy
        assert!(calculate_entropy("aaaaaaa") < 1.0);

        // High entropy
        assert!(calculate_entropy("aB3$xY9!zQ2#mK7") > 3.5);

        // Random base64-like
        let random = "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0";
        assert!(calculate_entropy(random) > 3.0);
    }

    #[test]
    fn test_is_placeholder() {
        assert!(is_placeholder("YOUR_KEY_HERE"));
        assert!(is_placeholder("sk_test_CHANGE_ME"));
        assert!(is_placeholder("AKIAIOSFODNN7EXAMPLE"));
        assert!(!is_placeholder("sk_live_51HrealkeystuffABC123"));
        assert!(!is_placeholder("postgresql://localhost:5432/db"));
    }

    /// ⚠️ Rewritten when the provider patterns became [`builtin_rules`]. It used
    /// to call `detect_secret` directly, which no longer knows about them — and
    /// asserting against a function production does not use is how a suite keeps
    /// passing while the product breaks. This compiles the built-ins the same way
    /// `evnx scan` does.
    #[test]
    fn builtin_rules_recognise_the_providers_they_name() {
        use crate::commands::scan::patternset::PatternSet;
        let set = PatternSet::compile(&builtin_rules()).expect("built-ins must compile");

        for (value, expected) in [
            ("AKIA4OZRMFJ3VREALKEY", "AWS Access Key"),
            (
                "sk_live_51H1234567890abcdefghijk",
                "Stripe Secret Key (LIVE)",
            ),
            (
                "sk_test_51H1234567890abcdefghijk",
                "Stripe Secret Key (test)",
            ),
            ("ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8", "GitHub Token"),
            ("-----BEGIN PRIVATE KEY-----", "Private Key"),
            ("-----BEGIN RSA PRIVATE KEY-----", "Private Key"),
        ] {
            let m = set
                .strongest(value)
                .unwrap_or_else(|| panic!("{value} must match a built-in rule"));
            assert_eq!(m.name, expected, "for {value}");
        }

        // A placeholder and an ordinary value must match nothing.
        for value in ["localhost", "postgresql://localhost:5432/db"] {
            assert!(set.strongest(value).is_none(), "{value} is not a secret");
        }
    }

    /// The heuristics keep their own test, because they are what is left.
    #[test]
    fn heuristics_need_the_variable_name() {
        // 40 base64 characters is only an AWS secret when the name says AWS.
        // ⚠️ Not AWS's documented sample key: it contains "EXAMPLE", which
        // `is_placeholder` rejects before any check runs, so a test using it
        // would pass for the wrong reason.
        let aws = "wJ9lrXUtnFEMI/K7MDzNG/bPxRfiCY4tQm8vHs2K";
        assert!(detect_heuristic(aws, "AWS_SECRET_ACCESS_KEY").is_some());
        let under_other_name = detect_heuristic(aws, "SOME_BLOB");
        assert!(
            under_other_name.is_none()
                || under_other_name
                    .as_ref()
                    .unwrap()
                    .0
                    .starts_with("High-entropy"),
            "without an AWS name it is at most a high-entropy string"
        );

        assert!(detect_heuristic("localhost", "DATABASE_HOST").is_none());
    }

    #[test]
    fn test_sensitive_detection() {
        assert!(is_sensitive_key("DB_PASSWORD"));
        assert!(is_sensitive_key("api_key"));
        assert!(!is_sensitive_key("APP_NAME"));
        assert!(!is_sensitive_key("DEBUG_MODE"));
    }

    // Named for what it covers rather than `tests` again — a `mod tests` inside
    // `mod tests` says nothing about its contents and reads as a mistake.
    mod placeholder_patterns {
        use super::*;

        #[test]
        fn test_is_placeholder_patterns() {
            // Base word + digits
            assert!(is_placeholder("example"));
            assert!(is_placeholder("example1"));
            assert!(is_placeholder("example123")); // ✅ This was failing
            assert!(is_placeholder("example_value"));

            assert!(is_placeholder("test"));
            assert!(is_placeholder("test123"));
            assert!(is_placeholder("test_key"));

            assert!(is_placeholder("dev"));
            assert!(is_placeholder("dev123"));
            assert!(is_placeholder("dev_key"));

            // your_* patterns
            assert!(is_placeholder("your_key"));
            assert!(is_placeholder("your_api_key_here"));
            assert!(is_placeholder("your-secret"));

            // Repeated characters
            assert!(is_placeholder("xxx"));
            assert!(is_placeholder("*****"));
            assert!(is_placeholder("####"));

            // Simple alphanumeric placeholders
            assert!(is_placeholder("abc123"));
            assert!(is_placeholder("xyz789"));

            // NOT placeholders (real secrets should not match)
            assert!(!is_placeholder("AKIA1234567890EXAMPLE"));
            assert!(!is_placeholder("ghp_xxxxxxxxxxxxxxxxxxxx"));
            assert!(!is_placeholder("sk_live_abc123def456ghi789"));
            assert!(!is_placeholder("My$ecureP@ssw0rd!"));
            assert!(!is_placeholder("production_api_key_2024"));
        }
    }
}
