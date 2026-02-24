/// Secret pattern detection for scanning .env files
///
/// This module contains regex patterns and entropy calculation for detecting
/// accidentally committed secrets. Patterns are based on real-world secret formats
/// from AWS, Stripe, GitHub, OpenAI, and other major services.
use lazy_static::lazy_static;
use regex::Regex;

/// A detected secret pattern
#[derive(Debug, Clone)]
pub struct SecretPattern {
    pub name: String,
    pub pattern: String,
    pub confidence: Confidence,
    pub action_url: Option<String>,
}

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
    /// AWS Access Key ID pattern
    pub static ref AWS_ACCESS_KEY: Regex = Regex::new(r"AKIA[0-9A-Z]{16}").unwrap();

    /// AWS Secret Access Key pattern (40 chars base64-like)
    pub static ref AWS_SECRET_KEY: Regex = Regex::new(r"[0-9a-zA-Z/+=]{40}").unwrap();

    /// Stripe Secret Key (live)
    pub static ref STRIPE_SECRET_LIVE: Regex = Regex::new(r"sk_live_[0-9a-zA-Z]{24,}").unwrap();

    /// Stripe Secret Key (test)
    pub static ref STRIPE_SECRET_TEST: Regex = Regex::new(r"sk_test_[0-9a-zA-Z]{24,}").unwrap();

    /// GitHub Personal Access Token
    pub static ref GITHUB_PAT: Regex = Regex::new(r"\bghp_[A-Za-z0-9]{36,40}\b").unwrap();

    /// GitHub OAuth Token
    pub static ref GITHUB_OAUTH: Regex = Regex::new(r"\bgho_[A-Za-z0-9]{36,40}\b").unwrap();

    /// GitHub App Token
    pub static ref GITHUB_APP: Regex = Regex::new(r"\b(ghu|ghs)_[A-Za-z0-9]{36,40}\b").unwrap();

    /// OpenAI API Key
    pub static ref OPENAI_API_KEY: Regex = Regex::new(r"sk-[0-9a-zA-Z]{48}").unwrap();

    /// Anthropic API Key
    pub static ref ANTHROPIC_API_KEY: Regex = Regex::new(r"sk-ant-api[0-9]{2}-[0-9a-zA-Z\-_]{95}").unwrap();

    /// Generic API key pattern (high entropy)
    pub static ref GENERIC_API_KEY: Regex = Regex::new(r#"api[_-]?key['\"]?\s*[:=]\s*['\"]?([0-9a-zA-Z_\-]{32,})['\"]?"#).unwrap();

    /// Private Key Header
    pub static ref PRIVATE_KEY: Regex = Regex::new(r"-----BEGIN [A-Z ]+ PRIVATE KEY-----").unwrap();
}

/// Get all secret patterns
pub fn get_patterns() -> Vec<SecretPattern> {
    vec![
        SecretPattern {
            name: "AWS Access Key".to_string(),
            pattern: r"AKIA[0-9A-Z]{16}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://console.aws.amazon.com/iam".to_string()),
        },
        SecretPattern {
            name: "Stripe Secret Key (Live)".to_string(),
            pattern: r"sk_live_[0-9a-zA-Z]{24,}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://dashboard.stripe.com/apikeys".to_string()),
        },
        SecretPattern {
            name: "Stripe Secret Key (Test)".to_string(),
            pattern: r"sk_test_[0-9a-zA-Z]{24,}".to_string(),
            confidence: Confidence::Medium,
            action_url: Some("https://dashboard.stripe.com/apikeys".to_string()),
        },
        SecretPattern {
            name: "GitHub Personal Access Token".to_string(),
            pattern: r"ghp_[0-9a-zA-Z]{36}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://github.com/settings/tokens".to_string()),
        },
        SecretPattern {
            name: "GitHub OAuth Token".to_string(),
            pattern: r"gho_[0-9a-zA-Z]{36}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://github.com/settings/tokens".to_string()),
        },
        SecretPattern {
            name: "OpenAI API Key".to_string(),
            pattern: r"sk-[0-9a-zA-Z]{48}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://platform.openai.com/api-keys".to_string()),
        },
        SecretPattern {
            name: "Anthropic API Key".to_string(),
            pattern: r"sk-ant-api[0-9]{2}-[0-9a-zA-Z\-_]{95}".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://console.anthropic.com/settings/keys".to_string()),
        },
        SecretPattern {
            name: "Private Key".to_string(),
            pattern: r"-----BEGIN [A-Z ]+ PRIVATE KEY-----".to_string(),
            confidence: Confidence::High,
            action_url: None,
        },
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
        "your_secret_here",
        "your_token_here",
        "change_me",
        "changeme",
        "replace_me",
        "xxx",
        "todo",
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

    false
}

/// Detect if a value matches any secret pattern
///
/// Returns (pattern_name, confidence) if a match is found
pub fn detect_secret(value: &str, key: &str) -> Option<(String, Confidence, Option<String>)> {
    // Skip obvious placeholders
    if is_placeholder(value) {
        return None;
    }

    // Check specific patterns
    if AWS_ACCESS_KEY.is_match(value) {
        return Some((
            "AWS Access Key".to_string(),
            Confidence::High,
            Some("https://console.aws.amazon.com/iam".to_string()),
        ));
    }

    // AWS Secret Key is tricky - high false positive rate
    // Only flag if key name suggests it's AWS-related
    if AWS_SECRET_KEY.is_match(value)
        && (key.to_uppercase().contains("AWS") || key.to_uppercase().contains("SECRET"))
    {
        let entropy = calculate_entropy(value);
        if entropy > 4.5 {
            return Some((
                "AWS Secret Access Key".to_string(),
                Confidence::Medium,
                Some("https://console.aws.amazon.com/iam".to_string()),
            ));
        }
    }

    if STRIPE_SECRET_LIVE.is_match(value) {
        return Some((
            "Stripe Secret Key (LIVE)".to_string(),
            Confidence::High,
            Some("https://dashboard.stripe.com/apikeys".to_string()),
        ));
    }

    if STRIPE_SECRET_TEST.is_match(value) {
        return Some((
            "Stripe Secret Key (test)".to_string(),
            Confidence::Medium,
            Some("https://dashboard.stripe.com/apikeys".to_string()),
        ));
    }

    if GITHUB_PAT.is_match(value) || GITHUB_OAUTH.is_match(value) || GITHUB_APP.is_match(value) {
        return Some((
            "GitHub Token".to_string(),
            Confidence::High,
            Some("https://github.com/settings/tokens".to_string()),
        ));
    }

    if OPENAI_API_KEY.is_match(value) {
        return Some((
            "OpenAI API Key".to_string(),
            Confidence::High,
            Some("https://platform.openai.com/api-keys".to_string()),
        ));
    }

    if ANTHROPIC_API_KEY.is_match(value) {
        return Some((
            "Anthropic API Key".to_string(),
            Confidence::High,
            Some("https://console.anthropic.com/settings/keys".to_string()),
        ));
    }

    if PRIVATE_KEY.is_match(value) {
        return Some(("Private Key".to_string(), Confidence::High, None));
    }

    // Generic high-entropy check as fallback
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
    fn test_aws_access_key() {
        assert!(AWS_ACCESS_KEY.is_match("AKIAIOSFODNN7EXAMPLE"));
        assert!(!AWS_ACCESS_KEY.is_match("not-an-aws-key"));
    }

    #[test]
    fn test_stripe_keys() {
        assert!(STRIPE_SECRET_LIVE.is_match("sk_live_51Habcdefghijklmnopqrstuvwxyz123456"));
        assert!(STRIPE_SECRET_TEST.is_match("sk_test_51Habcdefghijklmnopqrstuvwxyz123456"));
        assert!(!STRIPE_SECRET_LIVE.is_match("sk_test_something"));
    }

    #[test]
    fn test_github_tokens() {
        assert!(GITHUB_PAT.is_match("ghp_1234567890abcdefghijklmnopqrstuvwxyzABCD"));

        assert!(GITHUB_OAUTH.is_match("gho_1234567890abcdefghijklmnopqrstuvwxyzABCD"));

        assert!(GITHUB_APP.is_match("ghu_1234567890abcdefghijklmnopqrstuvwxyzABCD"));

        assert!(!GITHUB_PAT.is_match("ghp_short"));
        assert!(!GITHUB_PAT.is_match("not_a_token"));
    }

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

    #[test]
    fn test_detect_secret() {
        // AWS Access Key
        let result = detect_secret("AKIAIOSFODNN7EXAMPLE", "AWS_ACCESS_KEY_ID");
        assert!(result.is_none()); // It's a placeholder

        let result = detect_secret("AKIA4OZRMFJ3VEXAMPLE", "AWS_ACCESS_KEY_ID");
        assert!(result.is_some());
        if let Some((name, conf, _)) = result {
            assert_eq!(name, "AWS Access Key");
            assert_eq!(conf, Confidence::High);
        }

        // Stripe Live Key
        let result = detect_secret("sk_live_51H1234567890abcdefghijk", "STRIPE_SECRET_KEY");
        assert!(result.is_some());
        if let Some((name, conf, _)) = result {
            assert_eq!(name, "Stripe Secret Key (LIVE)");
            assert_eq!(conf, Confidence::High);
        }

        // Not a secret
        let result = detect_secret("localhost", "DATABASE_HOST");
        assert!(result.is_none());
    }
}
