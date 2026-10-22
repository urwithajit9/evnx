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

use crate::utils::patterns;

/// The built-in provider rules, compiled once by the engine `scan` uses
/// ([`crate::core::secretscan::PatternSet`]), rather than a second time here
/// as plain expressions (R-scan).
fn rules() -> &'static crate::core::secretscan::PatternSet {
    static RULES: OnceLock<crate::core::secretscan::PatternSet> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::core::secretscan::PatternSet::compile(&patterns::builtin_rules())
            .expect("the built-in rules compile; utils::patterns tests them")
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
    !rules().find_all(v).is_empty() || patterns::detect_heuristic(v, "").is_some()
}

/// Whether the **name** says the value is a credential, at the Lenient level
/// (`*_PASSWORD`, `*_SECRET`, `*_TOKEN`, `*_KEY`, …) — see [`is_secret_name`].
pub fn is_sensitive_key(key: &str) -> bool {
    is_secret_name(key, Strictness::Lenient)
}

/// Whether the variable's **name** says it holds a credential (F6, DEC-G14).
///
/// Two levels, for the same reason as [`is_placeholder`]'s — the cost of a
/// wrong answer is opposite in different places:
///
/// * **Strict** — `validate` and `validate --fix`. A hit demands a strong
///   value, and `--fix` may write one. Suffix-anchored, so
///   `SECRET_KEY_ROTATION_DAYS` (a number), `AUTH_ENABLED` (a flag),
///   `QUOTA_FREE_TOKENS` (a count) and `API_KEY_ID` (an id) are not secrets.
/// * **Lenient** — scan, masking (diff, sync preview), sync. A hit hides a
///   value; a miss prints a credential. Substring, so a credential word
///   anywhere counts.
///
/// Before U8c `validate` kept its own suffix rule, which missed `ADMIN_PWD`,
/// `STRIPE_APIKEY`, `CREDENTIALS` and `PASSWD` — names the Lenient rule and
/// the published guide both treat as secrets — so `ADMIN_PWD=123` validated
/// clean.
pub fn is_secret_name(key: &str, strictness: Strictness) -> bool {
    match strictness {
        Strictness::Strict => strict_name(key),
        Strictness::Lenient => lenient_name(key),
    }
}

fn strict_name(key: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "_SECRET",
        "_KEY",
        "_PASSWORD",
        "_TOKEN",
        "_PASSWD",
        "_PWD",
        "_APIKEY",
        "_CREDENTIALS",
    ];
    let upper = key.to_uppercase();
    // ⚠️ Not bare `PWD`: that is the shell's working directory, and a `.env`
    // that sets it holds a path.
    matches!(
        upper.as_str(),
        "SECRET_KEY" | "PASSWORD" | "SECRET" | "PASSWD" | "APIKEY" | "CREDENTIALS"
    ) || SUFFIXES.iter().any(|s| upper.ends_with(s))
}

fn lenient_name(key: &str) -> bool {
    // Substring: catches the word wherever it sits — `DB_PASSWORD`,
    // `MY_AUTH_HEADER`, `legacy_api_key_v2`.
    const CONTAINS: &[&str] = &[
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "TOKEN",
        "API_KEY",
        // ⚠️ `APIKEY` and `PWD` added 2026-10-07 (S2). The published guide has
        // listed `STRIPE_APIKEY` and `ADMIN_PWD` as redacted examples all
        // along, and neither matched: `API_KEY` carries an underscore and
        // `PASSWD` is not `PWD`. The documentation described the intent
        // correctly and the code did not implement it.
        "APIKEY",
        "PWD",
        "PRIVATE_KEY",
        "AUTH",
        "CREDENTIAL",
    ];
    // ⚠️ Suffix, added 2026-10-07 (S2). The substring list above misses every
    // `*_KEY` that is not spelled `API_KEY` or `PRIVATE_KEY`:
    //
    //   STRIPE_KEY       not sensitive   ⛔
    //   ENCRYPTION_KEY   not sensitive   ⛔
    //   SIGNING_KEY      not sensitive   ⛔
    //
    // `evnx validate` has always called those credential-shaped, via its
    // own suffix rule (now the Strict level). The two commands disagreeing meant
    // `diff --show-values` printed values that `validate` would have demanded
    // a strong secret for.
    const SUFFIXES: &[&str] = &["_KEY", "_SECRET", "_PASSWORD", "_TOKEN", "_PASSWD"];

    let upper = key.to_uppercase();
    CONTAINS.iter().any(|p| upper.contains(p))
        || SUFFIXES.iter().any(|sfx| upper.ends_with(sfx))
        || matches!(upper.as_str(), "KEY" | "SECRET" | "PASSWORD")
}

/// Whether `key` has one of `words` as a **whole word**: the name is split at
/// every character that is not a letter or digit, and case is ignored.
///
/// For choosing a placeholder by name (I-120, DEC-S10): `SUPPORT_EMAIL` is not
/// a `PORT`, `MONKEY_COUNT` not a `KEY`, `BYPASS_CACHE` not a `PASS`,
/// `GHOST_MODE` not a `HOST` — all four were, by substring.
///
/// ⚠️ Not a secret test. [`is_secret_name`]'s Lenient level stays a substring
/// match on purpose: there a miss prints a credential, and a false hit only
/// masks a value. A placeholder has neither cost, so it can be exact.
pub fn has_word(key: &str, words: &[&str]) -> bool {
    key.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| words.iter().any(|x| w.eq_ignore_ascii_case(x)))
}

/// Secrets only the application holds: signing keys and session secrets for
/// which any strong random value works, because nothing outside the app has
/// to know it (DEC-VA4). Exact names, upper case.
///
/// ⚠️ A list, not a suffix rule. `DJANGO_SECRET_KEY` is the app's own and
/// `STRIPE_SECRET_KEY` is Stripe's; `JWT_SECRET` is the app's and
/// `SUPABASE_JWT_SECRET` is issued by Supabase. Nothing in the spelling tells
/// them apart, and a wrong "yes" invents a credential (I-260). Left out on
/// purpose: Laravel's `APP_KEY` (`base64:` and a fixed length — use
/// `php artisan key:generate`) and `ENCRYPTION_KEY` (often a fixed length).
pub const APP_SECRETS: &[&str] = &[
    "SECRET_KEY",
    "DJANGO_SECRET_KEY",
    "FLASK_SECRET_KEY",
    "SECRET_KEY_BASE",
    "JWT_SECRET",
    "NEXTAUTH_SECRET",
    "AUTH_SECRET",
    "BETTER_AUTH_SECRET",
    "SESSION_SECRET",
    "COOKIE_SECRET",
    "APP_SECRET",
];

/// May evnx **generate** this variable's value (`validate --fix`)?
///
/// Yes for the app's own secrets ([`APP_SECRETS`]) and for passwords
/// (`*_PASSWORD`, `*_PASSWD`, `*_PWD`), which the project chooses (DEC-VA4).
/// No for everything a provider issues — a Stripe key, an AWS secret, a
/// deploy token: 64 random hex characters there is a fake credential that
/// then passes validation (I-260).
pub fn may_generate(key: &str) -> bool {
    let upper = key.to_uppercase();
    APP_SECRETS.contains(&upper.as_str())
        || matches!(upper.as_str(), "PASSWORD" | "PASSWD")
        || ["_PASSWORD", "_PASSWD", "_PWD"]
            .iter()
            .any(|s| upper.ends_with(s))
}

/// Whether a value a **format rule** matched cannot be a working credential
/// (I-050, N-20): a placeholder (Lenient, no key), a run of one filler
/// character after the prefix (`sk_live_xxxx…`, `ghp_0000…`, `AKIA****…`), or
/// the `user:password@localhost` URL evnx writes into new projects.
///
/// ⚠️ For rule matches, and for a value the contract declares secret (R-init).
/// The name and entropy heuristics already drop placeholders; the format rules
/// did not, so `sk_live_xxxxxxxx…` was reported high with "matches a live key
/// format, not a placeholder".
///
/// ⛔ A run of **eight** identical characters: a real key is random, and eight
/// in a row is not something randomness produces in a key's length.
pub fn is_stand_in(value: &str) -> bool {
    let v = value.trim();
    if is_placeholder("", v, Strictness::Lenient) || crate::core::placeholder::is_url_placeholder(v)
    {
        return true;
    }
    let mut run = 0usize;
    let mut last = None;
    for c in v.chars() {
        if Some(c) == last {
            run += 1;
        } else {
            run = 1;
            last = Some(c);
        }
        if run >= 8 && matches!(c, 'x' | 'X' | '*' | '0' | '#') {
            return true;
        }
    }
    false
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

/// How readily a value is called a placeholder (DEC-G8, DEC-G13).
///
/// The two levels exist because the verdict has opposite costs in different
/// places, and a single list was wrong for one of them:
///
/// * **Strict** — `validate` and `validate --fix`. A placeholder verdict raises
///   an error, and `--fix` **overwrites** the value. Over-matching destroys a
///   real credential (finding A1: `DB_PASSWORD=Tr0ub4dor<3horse` replaced on
///   the strength of a `<`). Anchored and conservative.
/// * **Lenient** — `scan`'s suppression and display (diff, sync preview, spec).
///   A placeholder verdict only hides a finding or labels a value. Includes
///   prefix forms such as `test_key`, `example_value`, `abc123`.
///
/// Measured before unifying (U3): 14 prefix-form values differ between the two
/// (`test_db_pass`, `development_db`, `abc123`, …). Giving `validate` the
/// Lenient list would let `--fix` overwrite them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    Strict,
    Lenient,
}

/// Values that never hold a real secret: placeholders at both levels
/// (DEC-G10). Lower-case.
const HOLES: &[&str] = &[
    "change-me",
    "none",
    "null",
    "*****",
    // AWS's documentation key pair.
    "akiaiosfodnn7example",
    "wjalrxutnfemi/k7mdeng/bpxrficyexamplekey",
];

/// Words that are a placeholder under a secret-looking name and a real value
/// under any other: `API_KEY=dev` versus `ENVIRONMENT=development` (DEC-G11).
const CONTEXT_WORDS: &[&str] = &[
    "test",
    "testing",
    "dev",
    "development",
    "sample",
    "dummy",
    "fake",
];

/// Whether `key=value`'s value is a placeholder rather than a real setting.
/// `key` may be empty when the caller has none: a context word then keeps the
/// answer each level gave before keys were passed (Lenient yes, Strict no).
pub fn is_placeholder(key: &str, value: &str, strictness: Strictness) -> bool {
    let v = value.trim();
    let lower = v.to_lowercase();

    // DEC-G10: holes at both levels.
    if v.is_empty()
        || (v.len() >= 2 && v.starts_with('<') && v.ends_with('>'))
        || HOLES.contains(&lower.as_str())
    {
        return true;
    }

    // DEC-G11: a context word depends on the name.
    if CONTEXT_WORDS.contains(&lower.as_str()) {
        return if key.is_empty() {
            strictness == Strictness::Lenient
        } else {
            is_sensitive_key(key)
        };
    }

    match strictness {
        Strictness::Strict => strict_rules(value),
        Strictness::Lenient => lenient_rules(value),
    }
}

/// `validate`'s list, moved here unchanged (anchored, never `contains`).
fn strict_rules(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        return true;
    }
    let lower = v.to_lowercase();

    // ⛔ **Anchored, not `contains`.** Every pattern below was a substring
    // test, and `--fix` acts on this verdict by overwriting the value:
    //
    // ```text
    // DB_PASSWORD=Tr0ub4dor<3horse        → 1784af0cb441c772…     matched on `<`
    // DATABASE_URL=postgres://u:pw@db/todos → https://example.com   matched on `todo`
    // ```
    //
    // Both are real values. The first is a passphrase in the style every
    // password guide recommends; the second is a to-do app's database. `<`,
    // `>`, `example`, `todo` and `xxx` all appear inside values people actually
    // hold, and a substring test cannot tell that from a value nobody filled
    // in.
    //
    // ⚠️ This was reported, fixed for the *weak-secret* path, and left here —
    // so the data loss closed by the weak-secret gate stayed reachable through
    // the placeholder path, which is the same `--fix` writing the same random
    // hex over the same credential.

    // `<anything>` — the **whole** value bracketed, which is how a template
    // marks a hole. `a<b` is not one.
    if v.len() >= 2 && v.starts_with('<') && v.ends_with('>') {
        return true;
    }

    // The word a generator writes, at the **start** of the value.
    // `evnx init` emits `your_<name>_value`, so the prefix form is load-bearing.
    const STARTS_WITH: &[&str] = &[
        "your_",
        "your-",
        "your ",
        "yourdomain",
        "youremail",
        "change_me",
        "change-me",
        "changeme",
        "replace_me",
        "replace-me",
        "replaceme",
        "generate-with",
        "generate_with",
        "placeholder",
        "todo",
        "tbd",
        "fixme",
        "xxx",
        "insert_",
        "insert-",
    ];
    if STARTS_WITH.iter().any(|p| lower.starts_with(p)) {
        return true;
    }

    // The **whole** value is the word.
    const EXACT: &[&str] = &[
        "example",
        "placeholder",
        "changeme",
        "change_me",
        "todo",
        "tbd",
        "fixme",
        "none",
        "null",
        "undefined",
        "value",
    ];
    if EXACT.contains(&lower.as_str()) {
        return true;
    }

    // ⓘ A host under one of RFC 2606's reserved documentation domains. These
    // exist precisely so documentation can name a host nobody owns, so a value
    // pointing at one is a hole by definition — `https://api.example.com/v1`
    // is, and `postgres://u:pw@db/example_db` is not.
    const DOC_HOSTS: &[&str] = &["example.com", "example.org", "example.net", "example.edu"];
    if let Some(after_scheme) = lower.split("://").nth(1) {
        let host = after_scheme
            .split('/')
            .next()
            .unwrap_or("")
            .rsplit('@')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
        if DOC_HOSTS
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        {
            return true;
        }
    }

    false
}

/// `scan`'s suppression list, moved here unchanged.
fn lenient_rules(value: &str) -> bool {
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
    if placeholder_pattern(&lower) {
        return true;
    }

    false
}

/// Helper: Check for common placeholder patterns like example123, test456, dev_key
fn placeholder_pattern(value: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::Strictness::{Lenient, Strict};

    /// I-050, N-20: what a format rule may match and still not report.
    #[test]
    fn t_i050_stand_ins_cannot_be_live() {
        for v in [
            "sk_live_xxxxxxxxxxxxxxxxxxxxxxxx",
            "ghp_000000000000000000000000000000000000",
            "AKIA****************",
            "AKIAIOSFODNN7EXAMPLE",
            "postgresql://user:password@localhost:5432",
            "mongodb+srv://user:password@cluster.example.net",
            "amqp://user:password@127.0.0.1:5672",
        ] {
            assert!(super::is_stand_in(v), "{v}");
        }
        for v in [
            "sk_live_51H1234567890abcdefghijk",
            "AKIA4OZRMFJ3VREALKEY",
            "postgres://user:password@db.prod.company.io:5432",
            "postgres://app:password1@localhost:5432",
            "postgres://admin:s3cr3t@localhost:5432",
            // seven in a row is not enough
            "sk_live_51Hxxxxxxx1234567890abcdefghijk",
        ] {
            assert!(!super::is_stand_in(v), "{v}");
        }
    }

    /// I-260, DEC-VA4: the app's own secrets and passwords, never a key a
    /// provider issues — whatever its spelling shares with an app secret.
    #[test]
    fn t_i260_only_app_secrets_and_passwords_may_be_generated() {
        for yes in [
            "SECRET_KEY",
            "secret_key",
            "DJANGO_SECRET_KEY",
            "JWT_SECRET",
            "NEXTAUTH_SECRET",
            "SESSION_SECRET",
            "DB_PASSWORD",
            "REDIS_PASSWD",
            "ADMIN_PWD",
            "PASSWORD",
        ] {
            assert!(super::may_generate(yes), "{yes}");
        }
        for no in [
            "STRIPE_SECRET_KEY",
            "STRIPE_KEY",
            "STRIPE_WEBHOOK_SECRET",
            "SUPABASE_JWT_SECRET",
            "AWS_SECRET_ACCESS_KEY",
            "CLERK_SECRET_KEY",
            "VERCEL_TOKEN",
            "SENDGRID_API_KEY",
            "APP_KEY",
            "ENCRYPTION_KEY",
            "PWD",
        ] {
            assert!(!super::may_generate(no), "{no}");
        }
    }

    /// F6 (DEC-G14): the 8 of U3's 32 names on which the two rules disagreed.
    /// Lenient says yes to all; Strict to the four that name a credential.
    #[test]
    fn t_f6_the_eight_names_at_both_levels() {
        use super::is_secret_name;
        for k in ["PASSWD", "ADMIN_PWD", "STRIPE_APIKEY", "CREDENTIALS"] {
            assert!(is_secret_name(k, Strict), "{k}");
            assert!(is_secret_name(k, Lenient), "{k}");
        }
        for k in ["QUOTA_FREE_TOKENS", "AUTH_ENABLED", "KEY", "API_KEY_ID"] {
            assert!(!is_secret_name(k, Strict), "{k}");
            assert!(is_secret_name(k, Lenient), "{k}");
        }
        // Strict is a subset of Lenient: whatever validate asks a strong value
        // for, masking hides.
        for k in [
            "DB_PASSWORD",
            "jwt_secret",
            "AWS_CREDENTIALS",
            "SECRET_KEY",
            "APIKEY",
            "SIGNING_KEY",
            "GITHUB_TOKEN",
            "ROOT_PASSWD",
            "MY_PWD",
            "X_APIKEY",
        ] {
            assert!(is_secret_name(k, Strict), "{k}");
            assert!(is_secret_name(k, Lenient), "{k} is Strict but not Lenient");
        }
        assert!(
            !is_secret_name("PWD", Strict),
            "the shell's working directory"
        );
        assert!(!is_secret_name("SECRET_KEY_ROTATION_DAYS", Strict));
    }

    /// DEC-G10. Before: scan called `<set-me>`, `none`, `null`, `change-me` and
    /// an empty value real; validate called `*****` and AWS's docs key real.
    #[test]
    fn t_dec_g10_holes_are_placeholders_at_both_levels() {
        for v in [
            "",
            "  ",
            "<set-me>",
            "<your-key>",
            "change-me",
            "none",
            "NULL",
            "*****",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            for s in [Strict, Lenient] {
                assert!(is_placeholder("API_KEY", v, s), "{v:?} at {s:?}");
                assert!(is_placeholder("", v, s), "{v:?} at {s:?}, no key");
            }
        }
        // Only the whole value bracketed; `a<b` is a real value.
        assert!(!is_placeholder("DB_PASSWORD", "Tr0ub4dor<3horse", Strict));
    }

    /// DEC-G11: the name decides for the context words.
    #[test]
    fn t_dec_g11_context_words_depend_on_the_name() {
        for s in [Strict, Lenient] {
            assert!(is_placeholder("API_KEY", "dev", s), "{s:?}");
            assert!(is_placeholder("DB_PASSWORD", "test", s), "{s:?}");
            assert!(!is_placeholder("ENVIRONMENT", "development", s), "{s:?}");
            assert!(!is_placeholder("MODE", "test", s), "{s:?}");
        }
        // No name: each level's old answer.
        assert!(is_placeholder("", "dev", Lenient));
        assert!(!is_placeholder("", "dev", Strict));
    }

    /// DEC-G13: the prefix forms are Lenient only, so `validate --fix` never
    /// overwrites them.
    #[test]
    fn t_dec_g13_prefix_forms_are_lenient_only() {
        for v in [
            "test_db_pass",
            "example_value",
            "dev123",
            "abc123",
            "development_db",
            "dummy_password",
            "####",
        ] {
            assert!(
                !is_placeholder("DB_PASSWORD", v, Strict),
                "{v} must stay real for --fix"
            );
            assert!(
                is_placeholder("DB_PASSWORD", v, Lenient),
                "{v} is scan noise"
            );
        }
    }

    /// Both levels agree on these, before and after.
    #[test]
    fn agreed_values_are_unchanged() {
        for v in [
            "YOUR_KEY_HERE",
            "placeholder",
            "changeme",
            "xxx",
            "todo",
            "your-secret",
        ] {
            for s in [Strict, Lenient] {
                assert!(is_placeholder("API_KEY", v, s), "{v} at {s:?}");
            }
        }
        for v in [
            "sk_live_51HrealkeystuffABC123",
            "postgres://u:pw@db/todos",
            "latest",
            "production",
            "my-app",
            "secret123",
        ] {
            for s in [Strict, Lenient] {
                assert!(!is_placeholder("API_KEY", v, s), "{v} at {s:?}");
            }
        }
    }

    use super::*;

    #[test]
    fn has_word_matches_whole_words_only() {
        assert!(has_word("API_PORT", &["PORT"]));
        assert!(has_word("port", &["PORT"]));
        assert!(has_word("my-db.host", &["HOST"]));
        assert!(!has_word("SUPPORT_EMAIL", &["PORT"]));
        assert!(!has_word("MONKEY_COUNT", &["KEY"]));
        assert!(!has_word("", &["KEY"]));
    }

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
