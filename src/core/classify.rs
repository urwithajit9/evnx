//! What a value *is*: one answer for every command (C-05, A-10a, U3).
//!
//! Before this module, "is this a secret" was answered from the variable's
//! **name** alone in `diff`, `convert` and `spec`, so
//! `DATABASE_URL=postgres://admin:s3cr3t@db/app` passed as harmless wherever the
//! name did not look secret (I-076, I-184). `scan` knew better — it reads the
//! value — but its knowledge lived in the scan command.
//!
//! This module answers from the value, using the same built-in rules `scan`
//! uses (`utils::patterns::builtin_rules`), and from the name, using the one
//! name list (`utils::patterns::is_sensitive_key`). `core` never imports a
//! command (T-SHARE-01).
//!
//! The placeholder side (with a `Strictness`, DEC-G8) joins in U3 part 2.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::utils::patterns;

/// The built-in provider rules, compiled once.
fn rules() -> &'static [Regex] {
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    RULES.get_or_init(|| {
        patterns::builtin_rules()
            .iter()
            .filter_map(|r| Regex::new(&r.regex).ok())
            .collect()
    })
}

/// Whether the **value** looks like a live credential: a provider token
/// (`sk_live_…`, `ghp_…`, `AKIA…`), a private key, a connection string with a
/// password, or a long high-entropy string. A placeholder is not.
pub fn is_secret_shaped(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() || patterns::is_placeholder(v) {
        return false;
    }
    rules().iter().any(|r| r.is_match(v)) || patterns::detect_heuristic(v, "").is_some()
}

/// Whether the **name** says the value is a credential (`*_PASSWORD`,
/// `*_SECRET`, `*_TOKEN`, `*_KEY`, …).
pub fn is_sensitive_key(key: &str) -> bool {
    patterns::is_sensitive_key(key)
}

/// Whether `key=value` holds a secret, by either the name or the value.
pub fn holds_secret(key: &str, value: &str) -> bool {
    is_sensitive_key(key) || is_secret_shaped(value) || url_password(value).is_some()
}

/// The byte range of the password in a URL's userinfo
/// (`scheme://user:PASSWORD@host`), when there is a non-empty one.
///
/// A userinfo **without** a colon (`https://TOKEN@host`) is a credential too —
/// that is how tokens ride in git and API URLs — and its range is returned.
pub fn url_password(value: &str) -> Option<Range<usize>> {
    let scheme_end = value.find("://")?;
    let scheme = &value[..scheme_end];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    {
        return None;
    }
    let auth_start = scheme_end + 3;
    let rest = &value[auth_start..];
    let auth_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_len];
    let at = authority.rfind('@')?;
    let userinfo = &authority[..at];
    let (start, end) = match userinfo.find(':') {
        Some(colon) => (auth_start + colon + 1, auth_start + at),
        None => (auth_start, auth_start + at),
    };
    (end > start).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_shaped_values() {
        for v in [
            "sk_live_51H1234567890abcdefghijk",
            "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
            "AKIA4OZRMFJ3VREALKEY",
            "-----BEGIN PRIVATE KEY-----",
            "postgres://admin:s3cr3t@db.internal:5432/app",
        ] {
            assert!(is_secret_shaped(v), "{v}");
        }
        for v in [
            "",
            "localhost",
            "8080",
            "true",
            "your_api_key_here",
            "postgres://localhost/app",
        ] {
            assert!(!is_secret_shaped(v), "{v}");
        }
    }

    #[test]
    fn url_password_finds_the_password_only() {
        let v = "postgres://admin:s3cr3t@db.internal:5432/app";
        assert_eq!(&v[url_password(v).unwrap()], "s3cr3t");
        let v = "https://ghp_token123@github.com/org/repo.git";
        assert_eq!(&v[url_password(v).unwrap()], "ghp_token123");
        // `@` after the path is not userinfo.
        assert!(url_password("https://api.example.com/?to=a@b.com").is_none());
        assert!(url_password("postgres://localhost:5432/app").is_none());
        assert!(url_password("redis://:@host").is_none());
        assert!(url_password("not a url").is_none());
    }

    #[test]
    fn holds_secret_by_name_or_value() {
        assert!(holds_secret("STRIPE_KEY", "anything"));
        assert!(holds_secret("DATABASE_URL", "postgres://u:pw@h/db"));
        assert!(holds_secret("UPSTREAM", "sk_live_51H1234567890abcdefghijk"));
        assert!(!holds_secret("PORT", "8080"));
    }
}
