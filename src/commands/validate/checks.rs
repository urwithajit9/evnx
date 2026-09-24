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
    pub static ref PORT_REGEX: Regex = Regex::new(r"^\d{1,5}$").expect("Port regex is valid");
    pub static ref EMAIL_REGEX: Regex =
        Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
            .expect("Email regex is valid");
}

// ─────────────────────────────────────────────────────────────
// Helper Predicates
// ─────────────────────────────────────────────────────────────

/// ⚠️ This is **not** [`crate::utils::patterns::is_placeholder`], and the
/// difference is deliberate: the two have opposite failure modes. There, a
/// placeholder verdict *suppresses* a scan finding, so over-matching hides a
/// real secret. Here it *raises* an error, so over-matching invents one — which
/// is why this list stays conservative and omits `test`, `dev` and `sample`.
/// `ENVIRONMENT=development` is a real value, not a placeholder.
///
/// ⚠️ Two definitions of one idea is still the shape that produced F1's URL bug.
/// Reconciling them needs the **key** as well as the value (`API_KEY=dev` is a
/// placeholder; `ENVIRONMENT=dev` is not), which is a design change, not a list
/// merge. Tracked separately rather than rushed in before a tag.
pub fn is_placeholder(value: &str) -> bool {
    let lower = value.to_lowercase();
    let placeholders = [
        // ⚠️ `your_` as a prefix, not the three exact spellings that used to be
        // here. `evnx init` generates `your_<name>_value` — so evnx wrote a
        // placeholder its own validator did not recognise, and a fresh
        // `init --with nextjs,postgresql` produced a DB_PASSWORD and a
        // NEXTAUTH_SECRET that `validate` passed without a word (N1, 2026-09-24).
        "your_",
        "your-",
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

/// `name` chooses the length floor; `value` is what gets judged.
///
/// ⚠️ The floor is **not** one number. 32 characters is the right bar for a
/// framework signing key — Django's `SECRET_KEY`, Flask's, NextAuth's — and it
/// is what this check was written for. Applied to every credential-shaped
/// variable it is simply wrong: an AWS access key **ID** is 20 characters by
/// specification, so `AWS_KEY=AKIA4OZRMFJ3VREALKEY` came back "too weak or
/// predictable", which is both false and noisy. Below 16 characters nothing is
/// a serious credential, so that is the general bar.
pub fn is_weak_secret_key(name: &str, key: &str) -> bool {
    let floor = if is_signing_key(name) { 32 } else { 16 };
    if key.len() < floor {
        return true;
    }

    // ⚠️ A long, uniformly hex/base64 value is a generated token, and the word
    // list below must not be applied to it. `1234` and `abcd` are ordinary hex,
    // so a random 64-character secret contains one roughly once in 500 — which
    // made `validate --fix` occasionally generate a secret that `validate` then
    // called weak. Rare, silent, and exactly the "evnx must pass its own
    // output" class. `password1234password1234password1234` is not uniform, so
    // it stays weak.
    if looks_generated(key) {
        return false;
    }

    let weak = [
        "secret", "password", "dev", "test", "1234", "abcd", "changeme", "example",
    ];
    let lower = key.to_lowercase();
    weak.iter().any(|w| lower.contains(w))
}

/// Is this a framework signing key, where 32 characters is the documented bar?
fn is_signing_key(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper == "SECRET_KEY" || upper.ends_with("_SECRET_KEY") || upper == "NEXTAUTH_SECRET"
}

/// Is this the shape of a machine-generated token rather than a typed one?
fn looks_generated(value: &str) -> bool {
    let all_hex = value.chars().all(|c| c.is_ascii_hexdigit());
    let all_b64 = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_' | '='));
    // Base64 of anything real mixes cases; requiring that keeps lowercase words
    // like `supersecretpassword` out.
    let mixed_case = value.chars().any(|c| c.is_ascii_uppercase())
        && value.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = value.chars().any(|c| c.is_ascii_digit());
    all_hex && has_digit || (all_b64 && mixed_case && has_digit)
}

pub fn validate_url(value: &str) -> bool {
    // One definition, shared with `[vars] format = "url"`. See the note on
    // `core::spec::is_url` for why this is not a regex here any more.
    crate::core::spec::is_url(value)
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

    // ⚠️ Iterate the **IndexMap**, not a HashSet difference.
    //
    // `HashSet::difference` yields hash order, and Rust seeds its hasher
    // randomly per process — so this reported the same missing variables in a
    // different order on every run. Five consecutive runs over the same file
    // produced five orderings. That makes the output undiffable, breaks any
    // golden-file test that covers more than one finding, and makes a human ask
    // what changed when nothing did.
    //
    // `example_vars` is an IndexMap and already holds the order the variables
    // appear in `.env.example` — which is also the order the reader is looking
    // at while they fix them.
    let env_keys: HashSet<_> = env_vars.keys().collect();

    example_vars
        .keys()
        .filter(|key| !env_keys.contains(key))
        .map(|key| Issue {
            severity: "error".to_string(),
            issue_type: IssueType::MissingVariable.as_str().to_string(),
            variable: key.to_string(),
            message: format!("Missing required variable: {}", key),
            location: env_path.to_string(),
            suggestion: Some(format!("Add {}=<value> to {}", key, env_path)),
            auto_fixable: true,
        })
        .collect()
}

/// Check: the same key assigned twice in one file.
///
/// ⚠️ **The parser cannot report this and never could.** It builds an
/// `IndexMap`, and `insert` overwrites — so `API_KEY=first` followed by
/// `API_KEY=second` silently becomes `second`, and the first value is gone
/// before any check sees the file. Neither `validate` nor `doctor` said a word.
///
/// Last-wins is a defensible parsing rule and is what dotenv implementations
/// generally do; evnx keeps it. What is not defensible is silence. A key written
/// twice in a `.env` is almost always a merge artefact or a copy-paste, and the
/// value the author expects is usually the first one.
///
/// This reads the file rather than the parsed map, because by the time there is
/// a map the evidence is gone.
pub fn check_duplicate_keys(content: &str, env_path: &str, ignore: &HashSet<String>) -> Vec<Issue> {
    if ignore.contains(IssueType::DuplicateKey.as_str()) {
        return Vec::new();
    }

    // key -> the line numbers it is assigned on, in file order.
    let mut seen: IndexMap<String, Vec<usize>> = IndexMap::new();

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let without_export = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((key, _)) = without_export.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                seen.entry(key.to_string()).or_default().push(idx + 1);
            }
        }
    }

    seen.into_iter()
        .filter(|(_, lines)| lines.len() > 1)
        .map(|(key, lines)| {
            let last = *lines.last().expect("filtered on len > 1");
            let places = lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Issue {
                severity: "warning".to_string(),
                issue_type: IssueType::DuplicateKey.as_str().to_string(),
                variable: key.clone(),
                message: format!(
                    "{key} is assigned {} times (lines {places}) — only line {last} takes effect",
                    lines.len()
                ),
                location: env_path.to_string(),
                suggestion: Some(format!(
                    "Remove the earlier assignments of {key}, or rename them if they were meant to differ"
                )),
                auto_fixable: false,
            }
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

    // Ordered for the same reason as `check_missing_variables` above.
    let example_keys: HashSet<_> = example_vars.keys().collect();

    env_vars
        .keys()
        .filter(|key| !example_keys.contains(key))
        .map(|key| Issue {
            severity: "warning".to_string(),
            issue_type: IssueType::ExtraVariable.as_str().to_string(),
            variable: key.to_string(),
            message: format!("Extra variable not in .env.example: {}", key),
            location: env_path.to_string(),
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
        .map(|(key, _value): (&String, &String)| {
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
                location: env_path.to_string(),
                suggestion,
                // ⚠️ Ask the fixer rather than asserting. A placeholder in
                // `DB_NAME` has no value evnx can supply, and claiming
                // "fixable with --fix" for it sends the user to a command that
                // will leave the line as it found it.
                auto_fixable: crate::commands::validate::fixer::suggest_fix(
                    key,
                    _value,
                    &IssueType::PlaceholderValue,
                )
                .is_actionable(),
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
            location: env_path.to_string(),
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

    // ⚠️ Every secret-shaped key, not the literal `SECRET_KEY`. This was
    // `env_vars.get("SECRET_KEY")` — one hardcoded lookup — so `JWT_SECRET=123`,
    // `SESSION_SECRET=weak` and `DB_PASSWORD=dev` all validated clean while the
    // identically-worthless `SECRET_KEY=123` was an error (N2, 2026-09-24).
    // `is_weak_secret_key` always took any key; only the call site was narrow.
    env_vars
        .iter()
        .filter(|(name, _)| is_secret_shaped(name))
        .filter(|(name, value)| is_weak_secret_key(name, value))
        .map(|(name, _)| Issue {
            severity: "error".to_string(),
            issue_type: IssueType::WeakSecret.as_str().to_string(),
            variable: name.clone(),
            message: format!("{name} is too weak or predictable"),
            location: env_path.to_string(),
            suggestion: Some("Run: openssl rand -hex 32".to_string()),
            auto_fixable: true,
        })
        .collect()
}

/// Does this variable's **name** say it holds a credential?
///
/// Suffix-anchored rather than `contains`, so `SECRET_KEY` and `JWT_SECRET`
/// match while `SECRET_KEY_ROTATION_DAYS` — a number, not a secret — does not.
pub fn is_secret_shaped(name: &str) -> bool {
    const SUFFIXES: &[&str] = &["_SECRET", "_KEY", "_PASSWORD", "_TOKEN", "_PASSWD"];
    let upper = name.to_uppercase();
    upper == "SECRET_KEY"
        || upper == "PASSWORD"
        || upper == "SECRET"
        || SUFFIXES.iter().any(|s| upper.ends_with(s))
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
            location: env_path.to_string(),
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
                location: env_path.to_string(),
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
                location: env_path.to_string(),
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
                location: env_path.to_string(),
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
        assert!(is_weak_secret_key("SECRET_KEY", "short"));
        assert!(is_weak_secret_key(
            "SECRET_KEY",
            "this-is-a-test-secret-key"
        ));
        // A signing key holds itself to 32; every other credential to 16, so an
        // AWS access key ID at its specified 20 characters is not weak.
        assert!(!is_weak_secret_key("AWS_KEY", "AKIA4OZRMFJ3VREALKEY"));
        assert!(is_weak_secret_key("SECRET_KEY", "AKIA4OZRMFJ3VREALKEY"));
        assert!(is_weak_secret_key("AWS_KEY", "short"));
        assert!(!is_weak_secret_key(
            "SECRET_KEY",
            "a7b9c4d1e8f2g5h3i6j0k9l8m7n6o5p4q3r2s1t0"
        ));
    }

    #[test]
    fn test_validate_url() {
        assert!(validate_url("https://example.com"));
        assert!(validate_url("http://localhost:8080/path"));

        // ⚠️ These are the reason this changed. Every one is a value evnx
        // itself writes or a user will certainly have, and every one was
        // rejected as "not a valid URL" before 2026-09-24.
        assert!(validate_url("postgresql://user:pw@localhost:5432/db"));
        assert!(validate_url("redis://localhost:6379"));
        assert!(validate_url("amqp://guest@localhost"));
        assert!(validate_url("mongodb+srv://cluster.example.net/db"));
        assert!(validate_url("s3://bucket/key"));
        // ⚠️ This line used to assert `!validate_url(...)`, pinning the bug.
        assert!(validate_url("ftp://example.com"));

        assert!(!validate_url("not-a-url"));
        assert!(!validate_url("://no-scheme"));
        assert!(!validate_url("http://"), "an empty authority is not a URL");
        assert!(!validate_url("1http://x"), "a scheme starts with a letter");
        assert!(!validate_url("http://has space/x"));
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

// ─────────────────────────────────────────────────────────────
// Spec-aware checks — slice 3 of Proposal D
//
// ⚠️ These run **instead of** `check_missing_variables` when the project
// declares a `[vars]` contract, not alongside it. Both answer "what is
// required", and the template's answer is the one the spec exists to replace:
// `.env.example` cannot say optional, so every line in it counts.
// ─────────────────────────────────────────────────────────────

use crate::core::spec::Spec;

/// Variables the spec declares required, applicable to this environment, and
/// absent from the file being checked.
pub fn check_spec_required(
    env_vars: &IndexMap<String, String>,
    spec: &Spec,
    env_name: Option<&str>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::MissingVariable.as_str()) {
        return Vec::new();
    }

    crate::core::spec::missing_required(spec, |k| env_vars.contains_key(k), env_name)
        .into_iter()
        .map(|key| Issue {
            severity: "error".to_string(),
            issue_type: IssueType::MissingVariable.as_str().to_string(),
            variable: key.to_string(),
            message: format!("Missing required variable: {key}"),
            location: env_path.to_string(),
            suggestion: Some(match spec.get(key).and_then(|v| v.description.as_deref()) {
                // The spec already says what the variable is for, so the
                // suggestion can say it too rather than repeating the name.
                Some(d) => format!("Add {key}=<value> to {env_path} — {d}"),
                None => format!("Add {key}=<value> to {env_path}"),
            }),
            auto_fixable: true,
        })
        .collect()
}

/// Values that do not match the format their `[vars]` entry declares.
///
/// ⚠️ An empty value is not reported here. It is either a missing variable —
/// which `check_spec_required` already covers — or a deliberate blank, and
/// reporting the same fact twice under two names makes a report harder to act
/// on, not more thorough.
pub fn check_spec_format(
    env_vars: &IndexMap<String, String>,
    spec: &Spec,
    env_name: Option<&str>,
    env_path: &str,
    ignore: &HashSet<String>,
) -> Vec<Issue> {
    if ignore.contains(IssueType::FormatMismatch.as_str()) {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (key, value) in env_vars {
        let Some(declared) = spec.get(key) else {
            continue;
        };
        if !declared.applies_to(env_name) {
            continue;
        }
        let Some(format) = &declared.format else {
            continue;
        };
        if value.trim().is_empty() || format.matches(value.trim()) {
            continue;
        }

        out.push(Issue {
            severity: "error".to_string(),
            issue_type: IssueType::FormatMismatch.as_str().to_string(),
            variable: key.clone(),
            message: format!("{key} is not {}", format.describe()),
            location: env_path.to_string(),
            suggestion: Some(format!("Fix the value, or relax `format` in [vars.{key}]")),
            // ⚠️ Not auto-fixable, and must not become so. evnx knows the value
            // is wrong; it has no idea what the right one is.
            auto_fixable: false,
        });
    }
    out
}

#[cfg(test)]
mod spec_check_tests {
    use super::*;
    use crate::core::spec::Spec;

    fn spec(src: &str) -> Spec {
        #[derive(serde::Deserialize)]
        struct W {
            #[serde(default)]
            vars: Spec,
        }
        toml::from_str::<W>(src).expect("valid spec").vars
    }

    fn env(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_optional_variable_is_not_reported_missing() {
        let s = spec("[vars.NEEDED]\n[vars.OPTIONAL]\nrequired = false\n");
        let issues = check_spec_required(&env(&[]), &s, None, ".env", &HashSet::new());
        let names: Vec<&str> = issues.iter().map(|i| i.variable.as_str()).collect();
        assert_eq!(names, ["NEEDED"]);
    }

    #[test]
    fn a_production_only_variable_is_ignored_in_staging() {
        let s = spec("[vars.PROD_KEY]\nenvironments = [\"production\"]\n");
        assert!(check_spec_required(
            &env(&[]),
            &s,
            Some("staging"),
            ".env.staging",
            &HashSet::new()
        )
        .is_empty());
        assert_eq!(
            check_spec_required(
                &env(&[]),
                &s,
                Some("production"),
                ".env.production",
                &HashSet::new()
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_description_reaches_the_suggestion() {
        let s = spec("[vars.DATABASE_URL]\ndescription = \"Primary Postgres\"\n");
        let issues = check_spec_required(&env(&[]), &s, None, ".env", &HashSet::new());
        assert!(issues[0]
            .suggestion
            .as_ref()
            .unwrap()
            .contains("Primary Postgres"));
    }

    #[test]
    fn a_value_that_does_not_match_its_format_is_reported() {
        let s = spec("[vars.PORT]\nformat = \"port\"\n[vars.DATABASE_URL]\nformat = \"url\"\n");
        let issues = check_spec_format(
            &env(&[("PORT", "not-a-port"), ("DATABASE_URL", "postgres://h/d")]),
            &s,
            None,
            ".env",
            &HashSet::new(),
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].variable, "PORT");
        assert!(
            issues[0].message.contains("port between"),
            "{}",
            issues[0].message
        );
    }

    /// Already reported as missing; saying it twice under a second name makes
    /// the report harder to act on.
    #[test]
    fn an_empty_value_is_not_also_a_format_error() {
        let s = spec("[vars.PORT]\nformat = \"port\"\n");
        assert!(
            check_spec_format(&env(&[("PORT", "")]), &s, None, ".env", &HashSet::new()).is_empty()
        );
        assert!(
            check_spec_format(&env(&[("PORT", "   ")]), &s, None, ".env", &HashSet::new())
                .is_empty()
        );
    }

    /// evnx knows the value is wrong and has no idea what the right one is.
    #[test]
    fn a_format_mismatch_is_never_auto_fixable() {
        let s = spec("[vars.PORT]\nformat = \"port\"\n");
        let issues = check_spec_format(&env(&[("PORT", "x")]), &s, None, ".env", &HashSet::new());
        assert!(!issues[0].auto_fixable);
    }

    #[test]
    fn a_variable_absent_from_the_spec_is_left_alone() {
        let s = spec("[vars.KNOWN]\nformat = \"int\"\n");
        assert!(check_spec_format(
            &env(&[("UNDECLARED", "anything at all")]),
            &s,
            None,
            ".env",
            &HashSet::new()
        )
        .is_empty());
    }

    #[test]
    fn both_checks_honour_ignore() {
        let s = spec("[vars.PORT]\nformat = \"port\"\n");
        let ig: HashSet<String> = ["missing_variable", "format_mismatch"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(check_spec_required(&env(&[]), &s, None, ".env", &ig).is_empty());
        assert!(check_spec_format(&env(&[("PORT", "x")]), &s, None, ".env", &ig).is_empty());
    }
}
