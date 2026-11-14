//! How a value is shown: one rule for every command (C-05, U3).
//!
//! | Situation | Shown as |
//! |---|---|
//! | values not requested | nothing (withheld), whatever the name |
//! | values requested, URL with a password | `postgres://admin:***@db/app` — structure kept (DEC-G9) |
//! | values requested, URL with a secret query value or token-shaped path segment | the URL with those parts as `***` |
//! | values requested, secret by name or value | `***` |
//! | values requested, plain value | the value |
//!
//! ⚠️ The mask never contains a **prefix of the secret**. `diff` once printed
//! the first two characters (`sk***`), which names the provider and narrows
//! the search — and panicked on `DB_PASSWORD=日本語…`, because byte 2 is not a
//! character boundary; `scan` prints `AKIA…LKEY` on purpose, because a finding has to
//! be locatable — that is a finding's preview, not a display mask, and stays in
//! scan.

use super::classify;

/// The mask for a whole value.
pub const MASK: &str = "***";

/// What stands in for a value that was not requested (DEC-G7).
pub const WITHHELD: &str = "<withheld>";

/// Why a value is or is not in the output: what a JSON report says beside it
/// (`value_state`, I-076b), so a `null` cannot be mistaken for a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueState {
    /// Not requested (`--show-values` was not passed).
    Withheld,
    /// Requested, and printed with its secret part replaced.
    Masked,
    /// Requested, and printed as it is.
    Shown,
}

/// How much of a value a command may show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disclosure {
    /// Not shown at all.
    Withheld,
    /// Shown masked; the string is safe to print.
    Masked(String),
    /// Shown as it is.
    Shown(String),
}

impl Disclosure {
    /// The text to print, or `None` when withheld.
    pub fn text(&self) -> Option<&str> {
        match self {
            Disclosure::Withheld => None,
            Disclosure::Masked(s) | Disclosure::Shown(s) => Some(s),
        }
    }

    /// Whether the printed text differs from the value.
    pub fn is_masked(&self) -> bool {
        matches!(self, Disclosure::Masked(_))
    }

    /// The [`ValueState`] a report records for this value.
    pub fn state(&self) -> ValueState {
        match self {
            Disclosure::Withheld => ValueState::Withheld,
            Disclosure::Masked(_) => ValueState::Masked,
            Disclosure::Shown(_) => ValueState::Shown,
        }
    }
}

/// `value` with every secret part replaced: a URL's password by [`MASK`]
/// (structure kept), a secret by name or shape entirely. A plain value comes
/// back unchanged. UTF-8 safe: it only splits at the URL's ASCII delimiters.
pub fn mask(key: &str, value: &str) -> String {
    if let Some(masked) = mask_url(value) {
        return masked;
    }
    if classify::is_sensitive_key(key) || classify::is_secret_shaped(value) {
        return MASK.to_string();
    }
    value.to_string()
}

/// Query parameters whose value is a credential, lower-case.
const SECRET_QUERY_PARAMS: &[&str] = &[
    "token",
    "access_token",
    "api_key",
    "apikey",
    "key",
    "secret",
    "sig",
    "signature",
    "password",
    "auth",
    "code",
];

/// Whether a URL path segment looks like a token: 20 or more characters of
/// letters, digits, `-` and `_`, with at least two of the classes lower case,
/// upper case and digit.
fn token_shaped_segment(seg: &str) -> bool {
    if seg.chars().count() < 20
        || !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return false;
    }
    let classes = [
        seg.chars().any(|c| c.is_ascii_lowercase()),
        seg.chars().any(|c| c.is_ascii_uppercase()),
        seg.chars().any(|c| c.is_ascii_digit()),
    ];
    classes.iter().filter(|c| **c).count() >= 2
}

/// `value` as a URL with its secret parts replaced, or `None` when it is not a
/// URL or holds nothing secret: the password, the value of a secret query
/// parameter, and a token-shaped path segment become [`MASK`]. Everything else
/// is kept, so the host and the shape of the URL stay readable.
fn mask_url(value: &str) -> Option<String> {
    let scheme_end = value.find("://")?;
    let mut changed = false;
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    if let Some(range) = classify::url_password(value) {
        out.push_str(&value[..range.start]);
        out.push_str(MASK);
        rest = &value[range.end..];
        changed = true;
    } else {
        out.push_str(&value[..scheme_end + 3]);
        rest = &rest[scheme_end + 3..];
    }
    // `rest` starts at the authority's end (or after `://`): split off the
    // fragment and query, then mask path segments and query values.
    let (before_frag, frag) = match rest.find('#') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (path_part, query) = match before_frag.find('?') {
        Some(i) => (&before_frag[..i], Some(&before_frag[i + 1..])),
        None => (before_frag, None),
    };
    // The path part still holds the host when no password was masked.
    let (host, path) = match path_part.find('/') {
        Some(i) => (&path_part[..i], &path_part[i..]),
        None => (path_part, ""),
    };
    out.push_str(host);
    for (i, seg) in path.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if token_shaped_segment(seg) {
            out.push_str(MASK);
            changed = true;
        } else {
            out.push_str(seg);
        }
    }
    if let Some(query) = query {
        out.push('?');
        for (i, pair) in query.split('&').enumerate() {
            if i > 0 {
                out.push('&');
            }
            match pair.split_once('=') {
                Some((name, v))
                    if !v.is_empty()
                        && SECRET_QUERY_PARAMS.contains(&name.to_ascii_lowercase().as_str()) =>
                {
                    out.push_str(name);
                    out.push('=');
                    out.push_str(MASK);
                    changed = true;
                }
                _ => out.push_str(pair),
            }
        }
    }
    out.push_str(frag);
    changed.then_some(out)
}

/// What to show for `key=value`. `show_values` is the user's request
/// (`--show-values`).
///
/// Without it, nothing about the value is shown, whatever the key is called.
/// With it, the value is shown, masked where it holds a secret (I-076).
///
/// ⚠️ Without it, a secret-looking name used to get [`MASK`] "so the reader
/// knows a value is there". In `diff` that printed `SECRET_KEY = ***` beside
/// `ALLOWED_HOSTS` with nothing, and two `***` lines under a changed
/// `JWT_SECRET`: lines that say nothing, in a mode that promises key names
/// only (owner, 2026-10-21). `diff` is the only caller.
pub fn disclose(key: &str, value: &str, show_values: bool) -> Disclosure {
    if !show_values {
        return Disclosure::Withheld;
    }
    let masked = mask(key, value);
    if masked == value {
        Disclosure::Shown(masked)
    } else {
        Disclosure::Masked(masked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SHARE-03, I-076, DEC-G9.
    #[test]
    fn t_i076_a_url_keeps_its_structure_and_loses_its_password() {
        assert_eq!(
            mask(
                "DATABASE_URL",
                "postgres://admin:s3cr3t@db.internal:5432/app"
            ),
            "postgres://admin:***@db.internal:5432/app"
        );
        // A non-secret name does not save a credential in the value.
        assert_eq!(
            mask("UPSTREAM", "https://ghp_token123@github.com/o/r.git"),
            "https://***@github.com/o/r.git"
        );
    }

    #[test]
    fn a_secret_is_masked_whole_with_no_prefix() {
        let m = mask("STRIPE_KEY", "sk_live_51H1234567890abcdefghijk");
        assert_eq!(m, MASK);
        let m = mask("UPSTREAM", "sk_live_51H1234567890abcdefghijk");
        assert_eq!(m, MASK, "secret by value, under a harmless name");
    }

    #[test]
    fn a_url_loses_its_secret_query_values_and_token_shaped_path_segments() {
        assert_eq!(
            mask(
                "WEBHOOK_URL",
                "https://h.example.com/notify?Token=abc123&x=1"
            ),
            "https://h.example.com/notify?Token=***&x=1"
        );
        for name in [
            "access_token",
            "api_key",
            "apikey",
            "key",
            "secret",
            "sig",
            "signature",
            "password",
            "auth",
            "code",
        ] {
            assert_eq!(
                mask("U", &format!("https://h/p?a=1&{name}=v1#f")),
                format!("https://h/p?a=1&{name}=***#f"),
                "{name}"
            );
        }
        assert_eq!(
            mask(
                "U",
                "https://hooks.example.com/services/Zq8Xk2mP9vLw3nB7rT5yD4Yc/x"
            ),
            "https://hooks.example.com/services/***/x"
        );
        // Password and query together.
        assert_eq!(
            mask("U", "https://u:pw@h/p?token=t"),
            "https://u:***@h/p?token=***"
        );
        // Nothing secret: unchanged. A long word of one class is not a token.
        assert_eq!(
            mask(
                "U",
                "https://h/docs/internationalizationlocalization?page=2"
            ),
            "https://h/docs/internationalizationlocalization?page=2"
        );
        assert_eq!(
            mask("U", "https://h/p?empty_token="),
            "https://h/p?empty_token="
        );
    }

    #[test]
    fn a_webhook_name_is_a_secret_name() {
        assert_eq!(mask("SLACK_WEBHOOK", "opaque"), MASK);
    }

    #[test]
    fn plain_and_non_ascii_values_pass_through() {
        assert_eq!(mask("PORT", "8080"), "8080");
        assert_eq!(mask("GREETING", "héllo wörld"), "héllo wörld");
        assert_eq!(mask("API_TOKEN", "日本語の秘密"), MASK);
    }

    #[test]
    fn disclose_follows_the_request() {
        assert_eq!(disclose("PORT", "8080", false), Disclosure::Withheld);
        // Not requested is not requested, whatever the name (owner, 2026-10-21).
        assert_eq!(disclose("DB_PASSWORD", "pw", false), Disclosure::Withheld);
        assert_eq!(
            disclose("DB_PASSWORD", "pw", true),
            Disclosure::Masked(MASK.into())
        );
        assert_eq!(
            disclose("PORT", "8080", true),
            Disclosure::Shown("8080".into())
        );
        assert_eq!(
            disclose("DATABASE_URL", "postgres://u:pw@h/db", true),
            Disclosure::Masked("postgres://u:***@h/db".into())
        );
        // Without --show-values the URL's host is not given away either.
        assert_eq!(
            disclose("DATABASE_URL", "postgres://u:pw@h/db", false),
            Disclosure::Withheld
        );
    }

    /// I-076b: the state a report prints beside the value.
    #[test]
    fn t_i076b_state_follows_the_disclosure() {
        assert_eq!(disclose("PORT", "1", false).state(), ValueState::Withheld);
        assert_eq!(disclose("PORT", "1", true).state(), ValueState::Shown);
        assert_eq!(disclose("API_KEY", "x", true).state(), ValueState::Masked);
        assert_eq!(
            serde_json::to_string(&ValueState::Withheld).unwrap(),
            "\"withheld\""
        );
    }
}
