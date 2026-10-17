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
//!
//! One entry point, [`guess`], for every writer of `[vars]`: `spec init` and
//! the writeback that `add` (and later `init`) uses. A variable's entry does not
//! depend on which command introduced it.

use crate::core::classify::{self, Strictness};
use crate::core::spec::Format;
use crate::core::value;

/// What the evidence supports for one variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    /// A format the sample has the shape of **and** passes.
    pub format: Option<Format>,
    /// A format the shape suggested but the sample fails, so it was left out
    /// (I-180, DEC-SP4). The caller says so; the value itself is never printed.
    pub left_out: Option<Format>,
    /// The name says credential (validate's Strict rule, DEC-SP14), or the
    /// sample is a URL carrying a password (DEC-SP10).
    pub secret: bool,
}

/// Guess `name`'s contract from a sample value (`""` when there is none).
///
/// ⚠️ A format is kept only when the sample passes [`Format::matches`] — the
/// same check `validate` applies to the same value. Before 0.10.6 the guess was
/// shape-only, so `PORT=0`, `REDIS_PORT=70000`, `DEBUG=2` and a 20-digit id
/// produced a spec that failed the project it came from (I-180).
pub fn guess(name: &str, sample: &str) -> Guess {
    let v = sample.trim();
    let shape = shape_of(name, v);
    let (format, left_out) = match shape {
        Some(f) if f.matches(v) => (Some(f), None),
        Some(f) => (None, Some(f)),
        None => (None, None),
    };
    Guess {
        format,
        left_out,
        secret: classify::is_secret_name(name, Strictness::Strict)
            || classify::url_password(v).is_some(),
    }
}

/// The format a value has the shape of, or `None` when nothing is obvious.
///
/// `name` is consulted only to break the genuine ambiguity between a port, a
/// boolean and an integer — `8080` and `1` are several things, and only the
/// name says which.
fn shape_of(name: &str, v: &str) -> Option<Format> {
    // An empty value or a placeholder tells us about the template, not about
    // the value. The Lenient level, as for every other display use (DEC-G8).
    if v.is_empty() || classify::is_placeholder(name, v, Strictness::Lenient) {
        return None;
    }

    // The shape only: `guess` then checks it with `core::spec::is_url`, so
    // `sqlite:///db.sqlite3` (no host) is named as left out rather than kept
    // or dropped silently.
    if v.contains("://") && !v.contains(' ') {
        return Some(Format::Url);
    }

    let upper = name.to_ascii_uppercase();
    let digits = v.chars().all(|c| c.is_ascii_digit());

    if digits {
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

    // The one list of boolean words (`core::value`, DEC-T7).
    if value::parse_bool(v).is_some() {
        return Some(Format::Bool);
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

    fn format_of(name: &str, v: &str) -> Option<Format> {
        guess(name, v).format
    }

    /// DEC-SP14 (I-190): `secret` from the name is validate's Strict rule, so
    /// the contract declares secret exactly what `validate` demands a strong
    /// value for.
    #[test]
    fn t_i190_secret_names_are_validates_strict_rule() {
        for yes in [
            "STRIPE_SECRET_KEY",
            "DB_PASSWORD",
            "GITHUB_TOKEN",
            "API_KEY",
            "PRIVATE_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "ADMIN_PWD",
            "STRIPE_APIKEY",
        ] {
            assert!(guess(yes, "").secret, "{yes} should look secret");
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
            // Before DEC-SP14 spec's own list called these secret; validate's
            // rule does not.
            "SENTRY_DSN",
            "TOKEN",
        ] {
            assert!(!guess(no, "").secret, "{no} should not look secret");
        }
    }

    /// DEC-SP10 (I-184): the evidence is in the value.
    #[test]
    fn t_i184_a_url_with_a_password_is_secret() {
        assert!(guess("DATABASE_URL", "postgres://u:p@h/d").secret);
        assert!(guess("REPO_URL", "https://TOKEN@github.com/o/r").secret);
        assert!(!guess("REDIS_URL", "redis://localhost:6379").secret);
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

    /// The shared words (`core::value`, DEC-T7), in any case.
    #[test]
    fn boolean_words_are_bools_whatever_the_name() {
        for v in ["true", "NO", "on", "Off", "y", "N"] {
            assert_eq!(format_of("ANYTHING", v), Some(Format::Bool), "{v}");
        }
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
            let g = guess("API_KEY", v);
            assert_eq!((g.format, g.left_out), (None, None), "inferred from {v:?}");
        }
    }

    #[test]
    fn an_opaque_string_infers_nothing() {
        assert_eq!(format_of("APP_NAME", "my-app"), None);
        assert_eq!(format_of("STRIPE_SECRET_KEY", "sk_live_abc"), None);
    }

    /// I-180, DEC-SP4 (T-SP-60): a shape the sample fails is left out, and
    /// named for the caller. Every row would have failed `validate` before.
    #[test]
    fn t_i180_a_format_the_sample_fails_is_left_out() {
        for (name, v, shape) in [
            ("PORT", "0", Format::Port),
            ("PORT", "65536", Format::Port),
            ("REDIS_PORT", "70000", Format::Port),
            ("DEBUG", "2", Format::Bool),
            ("ENABLE_X", "7", Format::Bool),
            ("USER_ID", "18446744073709551616000", Format::Int),
            ("ADMIN_EMAIL", "a@.com", Format::Email),
            // No host: not a URL by the definition `validate` uses.
            ("DATABASE_URL", "sqlite:///db.sqlite3", Format::Url),
        ] {
            let g = guess(name, v);
            assert_eq!(g.format, None, "{name}={v} kept a format");
            assert_eq!(g.left_out, Some(shape), "{name}={v}");
        }
        // And the ones that fit are kept, with nothing to say.
        for (name, v) in [("PORT", "1"), ("PORT", "65535"), ("DEBUG", "0"), ("N", "-")] {
            assert_eq!(guess(name, v).left_out, None, "{name}={v}");
        }
        assert_eq!(format_of("PORT", " 8080 "), Some(Format::Port));
    }

    /// `validate` trims before it checks; the guess does too, so the two agree.
    #[test]
    fn a_guess_always_passes_the_check_validate_makes() {
        for (name, v) in [
            ("PORT", "8080"),
            ("DEBUG", "true"),
            ("X", "yes"),
            ("COUNT", "42"),
            ("URL", "amqp://h"),
            ("MAIL", "a@b.io"),
        ] {
            let g = guess(name, v);
            let f = g.format.unwrap_or_else(|| panic!("{name}={v}: no format"));
            assert!(f.matches(v.trim()), "{name}={v}");
        }
    }
}
