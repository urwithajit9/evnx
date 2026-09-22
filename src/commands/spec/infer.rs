//! Guessing a variable's contract from what the project already shows us.
//!
//! ⚠️ Everything here is a **starting point for review**, not an answer. The
//! generated spec is a draft a human edits; that is why `run` prints how many
//! entries it wrote and tells you to read them.
//!
//! The inference is deliberately conservative. A wrong `format` turns a working
//! project red on the next `evnx validate`, so a field is emitted only when the
//! evidence is unambiguous — and left out otherwise, which the spec reads as
//! "not stated".

use crate::core::spec::Format;

/// Names that are a secret by their own account, regardless of value.
///
/// Matched as whole `_`-separated words so `KEYCLOAK_URL` is not a "KEY" and
/// `PASSWORD_MIN_LENGTH` is not a password.
const SECRET_WORDS: [&str; 8] = [
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "APIKEY",
    "PRIVATE",
    "CREDENTIALS",
    "DSN",
];

/// Is this name unambiguously a secret?
///
/// ⚠️ The test is on the **last** `_`-separated word, not on the name
/// containing one anywhere. `PASSWORD_MIN_LENGTH` contains "PASSWORD" and is a
/// configuration number; declaring it `secret = true` would have `scan` treat
/// `8` as a leaked credential.
///
/// The rule falls out nicely elsewhere too. `AWS_SECRET_ACCESS_KEY` ends in KEY
/// and is a secret; `AWS_ACCESS_KEY_ID` ends in ID and is not — which is right,
/// the ID half of an AWS pair is public. `TOKEN_EXPIRY_SECONDS` ends in SECONDS
/// and is not a secret either.
pub fn looks_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let words: Vec<&str> = upper.split('_').collect();
    let Some(last) = words.last() else {
        return false;
    };

    if SECRET_WORDS.contains(last) {
        return true;
    }
    // "KEY" is only a secret in a compound: `API_KEY` yes, a bare `KEY` no, and
    // `KEYCLOAK_URL` does not reach here because its last word is URL.
    *last == "KEY" && words.len() > 1
}

/// Guess a format from a sample value, or `None` when nothing is obvious.
///
/// `name` is consulted only to break the genuine ambiguity between a port and
/// an integer — `8080` is both, and only the name says which.
pub fn format_of(name: &str, value: &str) -> Option<Format> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    // A placeholder tells us about the template, not about the value.
    if v.starts_with("YOUR_") || v.starts_with('<') || v.eq_ignore_ascii_case("changeme") {
        return None;
    }

    if v.contains("://") && !v.contains(' ') {
        return Some(Format::Url);
    }

    let upper = name.to_ascii_uppercase();

    if matches!(
        v.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no"
    ) {
        return Some(Format::Bool);
    }

    if v.chars().all(|c| c.is_ascii_digit()) {
        // ⚠️ `1` and `0` are legal booleans and legal integers. The name decides,
        // and when it does not say, `int` is the safer guess: every boolean this
        // project accepts parses as an integer, but not the reverse.
        if upper.ends_with("_PORT") || upper == "PORT" {
            return Some(Format::Port);
        }
        if upper.starts_with("ENABLE_") || upper.ends_with("_ENABLED") || upper.contains("DEBUG") {
            return Some(Format::Bool);
        }
        return Some(Format::Int);
    }

    if let Some((local, domain)) = v.split_once('@') {
        if !local.is_empty() && domain.contains('.') && !domain.contains(' ') {
            return Some(Format::Email);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_words_match_whole_words_only() {
        for yes in [
            "STRIPE_SECRET_KEY",
            "DB_PASSWORD",
            "GITHUB_TOKEN",
            "API_KEY",
            "SENTRY_DSN",
            "PRIVATE_KEY",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(looks_secret(yes), "{yes} should look secret");
        }
        // ⚠️ The near-misses are the point. A substring match would call all of
        // these secrets and the generated spec would be noise.
        for no in [
            "KEYCLOAK_URL",
            "PASSWORD_MIN_LENGTH",
            "APP_NAME",
            "PORT",
            // The public half of an AWS pair, and a duration.
            "AWS_ACCESS_KEY_ID",
            "TOKEN_EXPIRY_SECONDS",
            "KEY",
        ] {
            assert!(!looks_secret(no), "{no} should not look secret");
        }
    }

    #[test]
    fn urls_win_over_everything_else() {
        assert_eq!(
            format_of("DATABASE_URL", "postgres://u:p@h/d"),
            Some(Format::Url)
        );
        assert_eq!(
            format_of("REDIS_URL", "redis://localhost:6379"),
            Some(Format::Url)
        );
    }

    #[test]
    fn a_bare_number_is_an_int_unless_the_name_says_port() {
        assert_eq!(format_of("PORT", "8080"), Some(Format::Port));
        assert_eq!(format_of("APP_PORT", "3000"), Some(Format::Port));
        assert_eq!(format_of("MAX_RETRIES", "5"), Some(Format::Int));
    }

    /// `1` parses as both. Int is the safer guess, because every boolean this
    /// project accepts also parses as an integer.
    #[test]
    fn an_ambiguous_one_prefers_int_unless_the_name_says_bool() {
        assert_eq!(format_of("SOMETHING", "1"), Some(Format::Int));
        assert_eq!(format_of("DEBUG", "1"), Some(Format::Bool));
        assert_eq!(format_of("FEATURE_ENABLED", "0"), Some(Format::Bool));
    }

    #[test]
    fn literal_booleans_are_bools_whatever_the_name() {
        assert_eq!(format_of("ANYTHING", "true"), Some(Format::Bool));
        assert_eq!(format_of("ANYTHING", "NO"), Some(Format::Bool));
    }

    #[test]
    fn emails_need_a_dotted_domain() {
        assert_eq!(format_of("ADMIN_EMAIL", "a@b.com"), Some(Format::Email));
        assert_eq!(format_of("HANDLE", "a@b"), None);
    }

    /// An empty template value and a placeholder both say nothing about the
    /// real value, so they must not produce a format.
    #[test]
    fn placeholders_and_blanks_infer_nothing() {
        for v in ["", "   ", "YOUR_KEY_HERE", "<your-value>", "changeme"] {
            assert_eq!(format_of("API_KEY", v), None, "inferred from {v:?}");
        }
    }

    #[test]
    fn an_opaque_string_infers_nothing() {
        assert_eq!(format_of("APP_NAME", "my-app"), None);
        assert_eq!(format_of("STRIPE_SECRET_KEY", "sk_live_abc"), None);
    }
}
