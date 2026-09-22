//! `[vars]` in `.evnx.toml` — the variable contract.
//!
//! # What this is for
//!
//! `.env.example` is doing three jobs badly. It is the documentation, the
//! contract, and the validation source all at once, and it cannot express any of
//! the things a contract needs to say: that a variable is optional, that it must
//! be a URL, that it is a secret, that it only applies in production.
//!
//! The concrete shape of that limit is in `commands/validate`, which computes
//! `required_total` as `example_file.vars.len()` — every line in the template is
//! required, because the template has nowhere to say otherwise.
//!
//! A spec can say it:
//!
//! ```toml
//! [vars.DATABASE_URL]
//! required     = true
//! format       = "url"
//! secret       = true
//! description  = "Primary Postgres connection string"
//! environments = ["development", "staging", "production"]
//!
//! [vars.FEATURE_NEW_CHECKOUT]
//! required = false
//! format   = "bool"
//! ```
//!
//! # ⚠️ It is optional, and stays optional
//!
//! Every project today has no `[vars]` section. A missing spec is not a missing
//! contract — it means the old behaviour, exactly. Nothing in this module may
//! become required for a command to run.
//!
//! # Why it lives in `.evnx.toml` rather than its own file
//!
//! The init redesign deferred this proposal on one condition: *"do not ship a
//! second TOML the user is told to write before deciding the fate of the
//! first."* `.evnx.toml` is now implemented and loading, so the way to honour
//! that is to put the spec **in** it. `Cargo.toml` carries `[dependencies]` the
//! same way.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// What a value has to look like.
///
/// Either a named type or a regular expression. The named types are the ones
/// `validate --validate-formats` already checks, so declaring `format = "url"`
/// exposes existing behaviour declaratively rather than adding a new validator.
#[derive(Debug, Clone)]
pub enum Format {
    Url,
    Int,
    Port,
    Bool,
    Email,
    /// Anything else is a regex, anchored by the author if they want it anchored.
    Pattern(Box<regex::Regex>),
}

// `regex::Regex` is not `PartialEq`, and `Config` derives it. Two patterns are
// the same when their source text is, which is the comparison anyone testing
// this actually means.
impl PartialEq for Format {
    fn eq(&self, other: &Self) -> bool {
        use Format::*;
        match (self, other) {
            (Url, Url) | (Int, Int) | (Port, Port) | (Bool, Bool) | (Email, Email) => true,
            (Pattern(a), Pattern(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl Format {
    /// Parse the TOML string form.
    ///
    /// ⚠️ A bad regex fails **here**, at load, rather than the first time a value
    /// happens to be checked against it. A contract that cannot be applied is
    /// broken whether or not anyone has tripped over it yet.
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "url" => Format::Url,
            "int" | "integer" => Format::Int,
            "port" => Format::Port,
            "bool" | "boolean" => Format::Bool,
            "email" => Format::Email,
            other => {
                let re = regex::Regex::new(other).map_err(|e| {
                    format!("`{other}` is neither a known format nor a valid regex: {e}")
                })?;
                Format::Pattern(Box::new(re))
            }
        })
    }

    /// The names that are not regexes, for error messages.
    pub const NAMED: [&'static str; 5] = ["url", "int", "port", "bool", "email"];

    /// Does `value` satisfy this format?
    pub fn matches(&self, value: &str) -> bool {
        match self {
            Format::Url => value.contains("://") && !value.contains(' '),
            Format::Int => value.parse::<i64>().is_ok(),
            Format::Port => matches!(value.parse::<u32>(), Ok(p) if (1..=65535).contains(&p)),
            Format::Bool => matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "false" | "1" | "0" | "yes" | "no"
            ),
            Format::Email => {
                let mut parts = value.splitn(2, '@');
                match (parts.next(), parts.next()) {
                    (Some(l), Some(r)) => !l.is_empty() && r.contains('.') && !r.starts_with('.'),
                    _ => false,
                }
            }
            Format::Pattern(re) => re.is_match(value),
        }
    }

    /// How to describe the requirement when it is not met.
    pub fn describe(&self) -> String {
        match self {
            Format::Url => "a URL".into(),
            Format::Int => "an integer".into(),
            Format::Port => "a port between 1 and 65535".into(),
            Format::Bool => "a boolean".into(),
            Format::Email => "an email address".into(),
            Format::Pattern(re) => format!("a match for /{}/", re.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for Format {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Format::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// One variable's declared contract.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct VarSpec {
    /// Unstated means required — see [`VarSpec::is_required`].
    pub required: Option<bool>,
    /// Unstated means "nothing declared", which is **not** the same as
    /// `secret = false`. `scan` falls back to its heuristics for the first and
    /// must not flag the second.
    pub secret: Option<bool>,
    pub format: Option<Format>,
    pub description: Option<String>,
    /// When present, the variable applies only in these environments.
    pub environments: Option<Vec<String>>,
    /// ⚠️ `pub(crate)` rather than private so `commands::spec` can build a
    /// `VarSpec` with `..Default::default()`. Still not part of the public API:
    /// unknown keys are reported through [`VarSpec::unknown_keys`].
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, toml::Value>,
}

impl VarSpec {
    /// ⚠️ Defaults to `true`, which is deliberate: it is what `.env.example`
    /// already means. `validate` treats every line in the template as required
    /// today, so a project writing its first spec keeps the behaviour it had
    /// and opts out per variable with `required = false`.
    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(true)
    }

    /// Does this variable apply when operating on `env`?
    ///
    /// A spec with no `environments` applies everywhere. `env` is `None` for the
    /// plain `.env`, which every declared variable applies to — restricting the
    /// base file to a named environment would be a contradiction.
    pub fn applies_to(&self, env: Option<&str>) -> bool {
        match (&self.environments, env) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(list), Some(name)) => list.iter().any(|e| e == name),
        }
    }

    /// Keys under `[vars.X]` that evnx does not understand.
    pub fn unknown_keys(&self) -> Vec<&str> {
        self.extra.keys().map(String::as_str).collect()
    }
}

/// Every declared variable, keyed by name. `BTreeMap` so output is ordered.
pub type Spec = BTreeMap<String, VarSpec>;

/// Names declared in the spec that are missing from `present`, respecting
/// `required` and `environments`.
///
/// Returned sorted, because `Spec` is a `BTreeMap`.
pub fn missing_required<'a>(
    spec: &'a Spec,
    present: impl Fn(&str) -> bool,
    env: Option<&str>,
) -> Vec<&'a str> {
    spec.iter()
        .filter(|(_, v)| v.is_required() && v.applies_to(env))
        .map(|(k, _)| k.as_str())
        .filter(|k| !present(k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_of(toml_src: &str) -> Spec {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            vars: Spec,
        }
        toml::from_str::<Wrapper>(toml_src)
            .expect("valid toml")
            .vars
    }

    #[test]
    fn a_declared_variable_is_required_unless_it_says_otherwise() {
        let s = spec_of(
            r#"
            [vars.DATABASE_URL]
            format = "url"

            [vars.FEATURE_FLAG]
            required = false
            "#,
        );
        assert!(
            s["DATABASE_URL"].is_required(),
            "unstated must mean required"
        );
        assert!(!s["FEATURE_FLAG"].is_required());
    }

    #[test]
    fn secret_distinguishes_unstated_from_declared_false() {
        let s = spec_of(
            r#"
            [vars.A]
            secret = false
            [vars.B]
            "#,
        );
        assert_eq!(s["A"].secret, Some(false), "explicitly not a secret");
        assert_eq!(s["B"].secret, None, "unstated — scan should fall back");
    }

    #[test]
    fn named_formats_parse_and_check() {
        for (name, good, bad) in [
            ("url", "postgres://h/d", "not a url"),
            ("int", "42", "4.2"),
            ("port", "8080", "70000"),
            ("bool", "TRUE", "maybe"),
            ("email", "a@b.com", "a@b"),
        ] {
            let f = Format::parse(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(f.matches(good), "{name} rejected {good}");
            assert!(!f.matches(bad), "{name} accepted {bad}");
        }
    }

    #[test]
    fn an_unknown_format_is_treated_as_a_regex() {
        let f = Format::parse("^sk_(test|live)_").unwrap();
        assert!(f.matches("sk_live_abc"));
        assert!(!f.matches("pk_live_abc"));
    }

    /// ⚠️ At load, not at first use. A contract that cannot be applied is broken
    /// whether or not a value has been checked against it yet.
    #[test]
    fn a_broken_regex_fails_when_the_spec_is_read() {
        let err = Format::parse("([unclosed").unwrap_err();
        assert!(
            err.contains("neither a known format nor a valid regex"),
            "{err}"
        );

        #[derive(Debug, Deserialize)]
        struct W {
            #[serde(default)]
            #[allow(dead_code)]
            vars: Spec,
        }
        let e = toml::from_str::<W>("[vars.A]\nformat = \"([unclosed\"\n").unwrap_err();
        assert!(e.to_string().contains("valid regex"), "{e}");
    }

    #[test]
    fn environments_restrict_where_a_variable_applies() {
        let s = spec_of(
            r#"
            [vars.PROD_ONLY]
            environments = ["production"]
            [vars.EVERYWHERE]
            "#,
        );
        assert!(s["PROD_ONLY"].applies_to(Some("production")));
        assert!(!s["PROD_ONLY"].applies_to(Some("staging")));
        // The base `.env` is not an environment, so nothing is excluded from it.
        assert!(s["PROD_ONLY"].applies_to(None));
        assert!(s["EVERYWHERE"].applies_to(Some("anything")));
    }

    #[test]
    fn missing_required_respects_optional_and_environment() {
        let s = spec_of(
            r#"
            [vars.NEEDED]
            [vars.OPTIONAL]
            required = false
            [vars.PROD_ONLY]
            environments = ["production"]
            "#,
        );
        let none_present = |_: &str| false;
        assert_eq!(
            missing_required(&s, none_present, Some("staging")),
            ["NEEDED"]
        );
        let mut prod = missing_required(&s, none_present, Some("production"));
        prod.sort();
        assert_eq!(prod, ["NEEDED", "PROD_ONLY"]);
        let all_present = |_: &str| true;
        assert!(missing_required(&s, all_present, None).is_empty());
    }

    #[test]
    fn unknown_keys_are_kept_rather_than_rejected() {
        let s = spec_of("[vars.A]\nrequired = true\nnonsense = 1\n");
        assert_eq!(s["A"].unknown_keys(), ["nonsense"]);
        assert!(s["A"].is_required(), "the known keys still work");
    }
}
