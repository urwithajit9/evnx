//! The rules evnx ships, as data: the built-in provider patterns, and the
//! build-time prefixes that publish a variable (C1, RM-45). Moved from
//! `utils::patterns`, unchanged.

use crate::core::config::PatternRule;

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
/// Both live in [`detect_heuristic`](super::heuristic::detect_heuristic) and stay token-scoped — running an entropy
/// threshold over whole lines would flag every minified bundle.
pub fn builtin_rules() -> Vec<PatternRule> {
    vec![
        PatternRule::builtin(
            "AWS Access Key",
            r"AKIA[0-9A-Z]{16}",
            "high",
            Some("https://console.aws.amazon.com/iam"),
        ),
        // ⚠️ `ASIA`, not only `AKIA`. An STS temporary key is a live credential
        // until it expires, and it is what every CI role assumption hands out —
        // so it is the form most likely to be pasted into a `.env` by accident.
        // It was not matched at all: `ASIAIOSFODNN7EXAMPLE` scanned clean.
        // Named separately from the long-term key so the output says which kind
        // it found, because what you do about them differs.
        PatternRule::builtin(
            "AWS Temporary Access Key (STS)",
            r"ASIA[0-9A-Z]{16}",
            "high",
            Some("https://console.aws.amazon.com/iam"),
        ),
        // ⚠️ The fine-grained PAT, which the `ghp_|gho_|ghu_|ghs_` alternation
        // above cannot match — different prefix, different length, and it is
        // the form GitHub now steers everyone towards.
        PatternRule::builtin(
            "GitHub Fine-grained PAT",
            r"github_pat_[A-Za-z0-9_]{40,}",
            "high",
            Some("https://github.com/settings/personal-access-tokens"),
        ),
        // Bot, user, app-level and legacy tokens share the `xox<x>-` shape;
        // `xapp-` is the app-level token and has its own prefix.
        PatternRule::builtin(
            "Slack Token",
            r"xox[abprs]-[0-9A-Za-z-]{10,}|xapp-[0-9]-[A-Za-z0-9-]{10,}",
            "high",
            Some("https://api.slack.com/apps"),
        ),
        // ⚠️ `medium`, not `high`. A Google API key is frequently a *browser*
        // key — Maps, Places — that is meant to ship in a page and is restricted
        // by referrer instead of secrecy. Calling every one of them high would
        // train people to ignore the severity column.
        PatternRule::builtin(
            "Google API Key",
            r"AIza[0-9A-Za-z_\-]{35}",
            "medium",
            Some("https://console.cloud.google.com/apis/credentials"),
        ),
        // ⚠️ Also `medium`, and for the same reason: plenty of JWTs are public
        // ID tokens. What makes one sensitive is usually the variable it sits
        // in, which the name heuristic now covers separately.
        PatternRule::builtin(
            "JSON Web Token",
            r"eyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
            "medium",
            None,
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
        // ⚠️ The gap every other rule here left open, and the most common thing
        // in a `.env` after an API key.
        //
        // `DATABASE_URL=postgresql://admin:s3cr3t@db.prod…` scanned **clean**
        // before this, and structurally so: every rule above is a provider prefix
        // and a connection string has none, the sensitive-key heuristic keys off
        // names containing `secret`/`token`/`key` and `DATABASE_URL` has none of
        // them, and the entropy fallback scores a whole URL below threshold.
        // Verified by scanning one. Filed as D15.
        //
        // The classes exclude `/`, `?` and `#`, so the userinfo match cannot run
        // past the authority into a path — `https://host/users/me@example.com` is
        // an ordinary URL, not a credential. Requiring a `:` before the `@` means
        // `postgresql://reader@host/db` is read as a bare username, which is a
        // name and not a secret.
        //
        // No `action_url`: there is no single provider to send anyone to. The
        // place to rotate a database password is the database.
        PatternRule::builtin(
            "Credentials in a connection string",
            r"[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s/?#@]+:[^\s/?#@]+@[^\s/?#]+",
            "high",
            None,
        ),
    ]
}

/// Build-time prefixes that inline a variable into the **client bundle**.
///
/// Each entry is `(prefix, what inlines it)`. The second field is there so a
/// finding can name the thing the reader has to go and change, rather than
/// describing evnx's own rule.
///
/// ⚠️ This is deliberately a short, conservative list of prefixes whose *whole
/// purpose* is public exposure. It is not a framework-detection system — that is
/// a larger piece of work (G1.2) and it is not needed to answer the question
/// this list exists for: "is this name one the bundler publishes?"
///
/// Longest first, so a future entry cannot be shadowed by a shorter one that
/// happens to be its suffix-prefix.
pub const PUBLIC_PREFIXES: &[(&str, &str)] = &[
    ("NEXT_PUBLIC_", "Next.js"),
    ("NUXT_PUBLIC_", "Nuxt"),
    ("EXPO_PUBLIC_", "Expo"),
    ("REACT_APP_", "Create React App"),
    ("STORYBOOK_", "Storybook"),
    ("GATSBY_", "Gatsby"),
    ("VUE_APP_", "Vue CLI"),
    ("VITE_", "Vite"),
    ("PUBLIC_", "SvelteKit and Astro"),
];

/// The build-time prefix on `name`, and what inlines it.
///
/// ⚠️ `PUBLIC_` is in the list even though `PUBLIC_KEY` is a perfectly ordinary
/// name for something meant to be public. That is safe because this only ever
/// *escalates a finding that already exists*: it never creates one. Verified
/// against the real detectors — `PUBLIC_KEY=<base64 public key>` and
/// `NEXT_PUBLIC_API_URL=https://…` both produce no finding at all, so there is
/// nothing for this to escalate.
#[must_use]
pub fn public_prefix(name: &str) -> Option<(&'static str, &'static str)> {
    let upper = name.to_uppercase();
    PUBLIC_PREFIXES
        .iter()
        .find(|(prefix, _)| upper.starts_with(prefix))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ Rewritten when the provider patterns became [`builtin_rules`]. It used
    /// to call `detect_secret` directly, which no longer knows about them — and
    /// asserting against a function production does not use is how a suite keeps
    /// passing while the product breaks. This compiles the built-ins the same way
    /// `evnx scan` does.
    #[test]
    fn the_connection_string_rule_catches_a_password_and_only_a_password() {
        use crate::core::secretscan::PatternSet;
        let set = PatternSet::compile(&builtin_rules()).expect("built-ins must compile");
        let hit = |v: &str| {
            set.strongest(v)
                .map(|m| m.name == "Credentials in a connection string")
                .unwrap_or(false)
        };

        // ⚠️ The case that motivated the rule: a DATABASE_URL nothing else flags.
        assert!(hit("postgresql://admin:s3cr3t@db.example.com:5432/main"));
        assert!(hit("mongodb+srv://u:p@cluster0.mongodb.net/test"));
        assert!(hit("amqp://guest:guest@rabbit:5672/"));
        assert!(hit("mysql://root:toor@127.0.0.1:3306/app"));
        // A password may contain `@`. The rule delimits it differently from a
        // strict URL parser, which does not matter — it still matches.
        assert!(hit("postgres://u:p@ss@host/db"));

        // No credentials at all.
        assert!(!hit("redis://localhost:6379/0"));
        assert!(!hit("postgresql://localhost:5432/db"));
        assert!(!hit("https://api.example.com/v1"));
        assert!(!hit("not-a-url-at-all"));
        assert!(!hit("mailto:someone@example.com"));
        assert!(!hit("postgres://u:p"));

        // A username with no password is a name, not a secret.
        assert!(!hit("postgresql://reader@db.example.com/main"));

        // ⚠️ An `@` past the authority belongs to the path or query. Matching it
        // would report a credential in an ordinary URL — which is why the
        // character classes exclude `/`, `?` and `#`.
        assert!(!hit("https://api.example.com/users/me@example.com"));
        assert!(!hit("https://api.example.com/?to=a@b.com"));
    }

    #[test]
    fn builtin_rules_recognise_the_providers_they_name() {
        use crate::core::secretscan::PatternSet;
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
}
