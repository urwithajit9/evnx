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
    BUILTIN.rules.clone()
}

/// `src/assets/rules.toml` as it is compiled in (C2, RM-50).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    /// The file's format version, for packs and rule files that share it (C6).
    #[allow(dead_code)]
    version: u32,
    rules: Vec<PatternRule>,
}

/// Parsed once per process. ⚠️ `expect`, not an error path: the file is part
/// of the binary, so a bad one is a build defect the loader test catches.
static BUILTIN: std::sync::LazyLock<RuleFile> = std::sync::LazyLock::new(|| {
    toml::from_str(include_str!("../../assets/rules.toml"))
        .expect("src/assets/rules.toml parses; rules::tests checks it")
});

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

    /// C2: `rules.toml` loads, every rule carries an id, a confidence and a
    /// severity that parse, no id repeats, and the whole set compiles.
    #[test]
    fn rules_toml_loads_with_unique_ids_and_compiles() {
        use crate::core::secretscan::{Confidence, PatternSet, Severity};
        let rules = builtin_rules();
        assert!(rules.len() >= 13);
        let mut ids = std::collections::HashSet::new();
        for r in &rules {
            let id =
                r.id.as_deref()
                    .unwrap_or_else(|| panic!("{} has no id", r.name));
            assert!(ids.insert(id), "duplicate id {id}");
            r.confidence
                .as_deref()
                .unwrap()
                .parse::<Confidence>()
                .unwrap();
            r.severity.as_deref().unwrap().parse::<Severity>().unwrap();
        }
        PatternSet::compile(&rules).expect("the built-in rules compile");
    }

    /// C2, S7: the case that split the two fields. A Google API key's shape
    /// is certain, and it is usually a browser key meant to be public.
    #[test]
    fn a_google_api_key_is_high_confidence_and_low_severity() {
        let g = builtin_rules()
            .into_iter()
            .find(|r| r.id.as_deref() == Some("google-api-key"))
            .unwrap();
        assert_eq!(
            (g.confidence.as_deref(), g.severity.as_deref()),
            (Some("high"), Some("low"))
        );
    }

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
