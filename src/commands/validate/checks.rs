//! Pure validation check functions
//!
//! All functions in this module are side-effect free and return
//! collections of issues. They do not modify files or print output.

use lazy_static::lazy_static;
use regex::Regex;
// use serde_json::value::Index;
use indexmap::IndexMap;
use std::collections::HashSet;

use super::types::{Issue, IssueType};

// ─────────────────────────────────────────────────────────────
// Regex Patterns for Format Validation
// ─────────────────────────────────────────────────────────────

lazy_static! {
    pub static ref URL_REGEX: Regex =
        Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").expect("URL regex is valid");
    pub static ref PORT_REGEX: Regex = Regex::new(r"^\d{1,5}$").expect("Port regex is valid");
    pub static ref EMAIL_REGEX: Regex =
        Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
            .expect("Email regex is valid");
}

// ─────────────────────────────────────────────────────────────
// Helper Predicates
// ─────────────────────────────────────────────────────────────

pub fn is_placeholder(value: &str) -> bool {
    let lower = value.to_lowercase();
    let placeholders = [
        "your_key_here",
        "your_secret_here",
        "your_token_here",
        "change_me",
        "changeme",
        "replace_me",
        "example",
        "xxx",
        "todo",
        "generate-with",
        "placeholder",
        "<",
        ">",
    ];
    placeholders.iter().any(|p| lower.contains(p)) || value.is_empty()
}

pub fn is_weak_secret_key(key: &str) -> bool {
    if key.len() < 32 {
        return true;
    }
    let weak = [
        "secret", "password", "dev", "test", "1234", "abcd", "changeme", "example",
    ];
    let lower = key.to_lowercase();
    weak.iter().any(|w| lower.contains(w))
}

pub fn validate_url(value: &str) -> bool {
    URL_REGEX.is_match(value)
}

/// Validate that a value is a valid port number (1-65535)
pub fn validate_port(value: &str) -> bool {
    // Check regex format first (1-5 digits)
    if !PORT_REGEX.is_match(value) {
        return false;
    }

    // Parse and validate range: ports must be 1-65535 (0 is invalid)
    // value.parse::<u16>().map_or(false, |port| port >= 1)
    value.parse::<u16>().is_ok_and(|port| port >= 1)
}

pub fn validate_email(value: &str) -> bool {
    EMAIL_REGEX.is_match(value)
}

// ─────────────────────────────────────────────────────────────
// Validation Check Functions
// Each returns Vec<Issue> for the specific check
// ─────────────────────────────────────────────────────────────

/// Check 1: Missing required variables
pub fn check_missing_variables(
    env_vars: &IndexMap<String, String>,
    example_vars: &IndexMap<String, String>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::MissingVariable.as_str()) {
        return Vec::new();
    }

    let example_keys: HashSet<_> = example_vars.keys().collect();
    let env_keys: HashSet<_> = env_vars.keys().collect();

    example_keys
        .difference(&env_keys)
        .map(|key| Issue {
            severity: "error".to_string(),
            issue_type: IssueType::MissingVariable.as_str().to_string(),
            variable: key.to_string(),
            message: format!("Missing required variable: {}", key),
            location: format!("{}:?", env_path),
            suggestion: Some(format!("Add {}=<value> to {}", key, env_path)),
            auto_fixable: true,
        })
        .collect()
}

/// Check 2: Extra variables (strict mode only)
pub fn check_extra_variables(
    env_vars: &IndexMap<String, String>,
    example_vars: &IndexMap<String, String>,
    env_path: &str,
    strict: bool,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if !strict || ignore.contains(IssueType::ExtraVariable.as_str()) {
        return Vec::new();
    }

    let example_keys: HashSet<_> = example_vars.keys().collect();
    let env_keys: HashSet<_> = env_vars.keys().collect();

    env_keys
        .difference(&example_keys)
        .map(|key| Issue {
            severity: "warning".to_string(),
            issue_type: IssueType::ExtraVariable.as_str().to_string(),
            variable: key.to_string(),
            message: format!("Extra variable not in .env.example: {}", key),
            location: format!("{}:?", env_path),
            suggestion: Some(format!(
                "Add {} to .env.example or remove from {}",
                key, env_path
            )),
            auto_fixable: false,
        })
        .collect()
}

/// Check 3: Placeholder values
pub fn check_placeholders(
    env_vars: &IndexMap<String, String>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::PlaceholderValue.as_str()) {
        return Vec::new();
    }

    env_vars
        .iter()
        .filter(|(_, v)| is_placeholder(v))
        .map(|(key, _value)| {
            let suggestion = match key.as_str() {
                "SECRET_KEY" => Some("Run: openssl rand -hex 32".to_string()),
                k if k.contains("AWS") => Some("Get from AWS Console".to_string()),
                k if k.contains("STRIPE") => Some("Get from Stripe Dashboard".to_string()),
                _ => None,
            };

            Issue {
                severity: "error".to_string(),
                issue_type: IssueType::PlaceholderValue.as_str().to_string(),
                variable: key.clone(),
                message: format!("{} looks like a placeholder", key),
                location: format!("{}:?", env_path),
                suggestion,
                auto_fixable: true,
            }
        })
        .collect()
}

/// Check 4: Boolean string trap
pub fn check_boolean_trap(
    env_vars: &IndexMap<String, String>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::BooleanTrap.as_str()) {
        return Vec::new();
    }

    env_vars
        .iter()
        .filter(|(_, v)| *v == "False" || *v == "True")
        .map(|(key, value)| Issue {
            severity: "warning".to_string(),
            issue_type: IssueType::BooleanTrap.as_str().to_string(),
            variable: key.clone(),
            message: format!("{} is set to \"{}\" (string, not boolean)", key, value),
            location: format!("{}:?", env_path),
            suggestion: Some(format!(
                "Use {} or 0 for proper boolean handling",
                if value == "False" { "false" } else { "true" }
            )),
            auto_fixable: true,
        })
        .collect()
}

/// Check 5: Weak SECRET_KEY
pub fn check_weak_secret(
    env_vars: &IndexMap<String, String>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::WeakSecret.as_str()) {
        return Vec::new();
    }

    env_vars
        .get("SECRET_KEY")
        .filter(|key| is_weak_secret_key(key))
        .map(|_| Issue {
            severity: "error".to_string(),
            issue_type: IssueType::WeakSecret.as_str().to_string(),
            variable: "SECRET_KEY".to_string(),
            message: "SECRET_KEY is too weak or predictable".to_string(),
            location: format!("{}:?", env_path),
            suggestion: Some("Run: openssl rand -hex 32".to_string()),
            auto_fixable: true,
        })
        .into_iter()
        .collect()
}

/// Check 6: localhost in Docker context
pub fn check_localhost_docker(
    env_vars: &IndexMap<String, String>,
    env_path: &str,
    has_docker: bool,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if !has_docker || ignore.contains(IssueType::LocalhostInDocker.as_str()) {
        return Vec::new();
    }

    env_vars
        .iter()
        .filter(|(k, v)| {
            (v.contains("localhost") || v.contains("127.0.0.1"))
                && (k.contains("URL") || k.contains("HOST") || k.contains("ADDR"))
        })
        .map(|(key, _)| Issue {
            severity: "warning".to_string(),
            issue_type: IssueType::LocalhostInDocker.as_str().to_string(),
            variable: key.clone(),
            message: format!("{} uses localhost/127.0.0.1", key),
            location: format!("{}:?", env_path),
            suggestion: Some("In Docker, use service name instead (e.g., db:5432)".to_string()),
            auto_fixable: false,
        })
        .collect()
}

/// Check 7: Format validation (URL, port, email)
pub fn check_formats(
    env_vars: &IndexMap<String, String>,
    env_path: &str,
    validate: bool,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if !validate {
        return Vec::new();
    }

    let mut issues = Vec::new();

    for (key, value) in env_vars.iter() {
        let key_upper = key.to_uppercase();

        // URL validation
        if (key_upper.contains("URL")
            || key_upper.contains("URI")
            || key_upper.contains("ENDPOINT"))
            && !value.is_empty()
            && !ignore.contains(IssueType::InvalidUrl.as_str())
            && !validate_url(value)
        {
            issues.push(Issue {
                severity: "warning".to_string(),
                issue_type: IssueType::InvalidUrl.as_str().to_string(),
                variable: key.clone(),
                message: format!("{} does not appear to be a valid URL", key),
                location: format!("{}:?", env_path),
                suggestion: Some("Expected format: https://example.com/path".to_string()),
                auto_fixable: false,
            });
        }

        // Port validation
        if key_upper.contains("PORT")
            && !value.is_empty()
            && !ignore.contains(IssueType::InvalidPort.as_str())
            && !validate_port(value)
        {
            issues.push(Issue {
                severity: "error".to_string(),
                issue_type: IssueType::InvalidPort.as_str().to_string(),
                variable: key.clone(),
                message: format!("{} is not a valid port number (1-65535)", key),
                location: format!("{}:?", env_path),
                suggestion: Some("Expected format: 8080".to_string()),
                auto_fixable: false,
            });
        }

        // Email validation
        if (key_upper.contains("EMAIL") || key_upper.contains("MAIL"))
            && !value.is_empty()
            && !ignore.contains(IssueType::InvalidEmail.as_str())
            && !validate_email(value)
        {
            issues.push(Issue {
                severity: "warning".to_string(),
                issue_type: IssueType::InvalidEmail.as_str().to_string(),
                variable: key.clone(),
                message: format!("{} does not appear to be a valid email", key),
                location: format!("{}:?", env_path),
                suggestion: Some("Expected format: user@example.com".to_string()),
                auto_fixable: false,
            });
        }
    }

    issues
}

// ─────────────────────────────────────────────────────────────
// Tests for Pure Functions
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_placeholder() {
        assert!(is_placeholder("YOUR_KEY_HERE"));
        assert!(is_placeholder("changeme"));
        assert!(!is_placeholder("sk_live_abc123"));
        assert!(is_placeholder("")); // empty is placeholder
    }

    #[test]
    fn test_is_weak_secret_key() {
        assert!(is_weak_secret_key("short"));
        assert!(is_weak_secret_key("this-is-a-test-secret-key"));
        assert!(!is_weak_secret_key(
            "a7b9c4d1e8f2g5h3i6j0k9l8m7n6o5p4q3r2s1t0"
        ));
    }

    #[test]
    fn test_validate_url() {
        assert!(validate_url("https://example.com"));
        assert!(validate_url("http://localhost:8080/path"));
        assert!(!validate_url("not-a-url"));
        assert!(!validate_url("ftp://example.com"));
    }

    #[test]
    fn test_validate_port() {
        // Valid ports
        assert!(validate_port("1"));
        assert!(validate_port("80"));
        assert!(validate_port("443"));
        assert!(validate_port("8080"));
        assert!(validate_port("65535"));

        // Invalid ports
        assert!(!validate_port("0")); // port 0 is reserved
        assert!(!validate_port("65536")); // exceeds u16::MAX
        assert!(!validate_port("abc")); // non-numeric
        assert!(!validate_port("")); // empty
        assert!(!validate_port("12.34")); // decimal
        assert!(!validate_port("-1")); // negative (regex fails)
        assert!(!validate_port("999999")); // too many digits (regex fails)
    }

    #[test]
    fn test_check_missing_variables() {
        let mut env = IndexMap::new();
        env.insert("DB_URL".to_string(), "postgres://localhost".to_string());

        let mut example = IndexMap::new();
        example.insert("DB_URL".to_string(), "".to_string());
        example.insert("API_KEY".to_string(), "".to_string());

        let issues = check_missing_variables(&env, &example, ".env", &HashSet::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].variable, "API_KEY");
        assert_eq!(issues[0].severity, "error");
    }

    #[test]
    fn test_check_boolean_trap() {
        let mut env = IndexMap::new();
        env.insert("DEBUG".to_string(), "True".to_string());

        let issues = check_boolean_trap(&env, ".env", &HashSet::new());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("string, not boolean"));
    }
}
