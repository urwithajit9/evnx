// Converter trait and options for format conversion
//
// Provides the core trait and configuration for converting environment variables
// to different output formats.

use anyhow::Result;
use indexmap::IndexMap;

/// Key transformation options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransform {
    /// Transform keys to UPPERCASE
    Uppercase,
    /// Transform keys to lowercase
    Lowercase,
    /// Transform keys to camelCase
    CamelCase,
    /// Transform keys to snake_case
    SnakeCase,
}

impl KeyTransform {
    /// `--transform`'s value, by the one list rule ([`crate::core::args::one_of`]).
    ///
    /// ⚠️ An unknown mode used to be dropped in silence — `--transform Title`
    /// converted with the input's names and exited 0, and a test asserted that
    /// as "succeeds with a warning" when no warning existed (I-090, T-016).
    /// Case does not matter, as for every other list flag.
    pub fn parse(value: &str) -> Result<KeyTransform> {
        crate::core::args::one_of(
            "transform",
            value,
            &[
                ("uppercase", KeyTransform::Uppercase),
                ("lowercase", KeyTransform::Lowercase),
                ("camelcase", KeyTransform::CamelCase),
                ("snake_case", KeyTransform::SnakeCase),
            ],
            &["uppercase", "lowercase", "camelCase", "snake_case"],
        )
    }
}

/// What to do when two names become one name at the target (I-088, I-249).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collisions {
    /// Emit every name, duplicates included — what every format did before
    /// 0.10.9. ⚠️ Only `cloud export` still converts this way; whether it
    /// should refuse too is recorded for R-org-vault-cloud.
    #[default]
    Unchecked,
    /// Refuse, naming the names: what `convert` does unless told otherwise.
    Refuse,
    /// Keep the last value (`--allow-collisions`, DEC-PL2).
    LastWins,
}

/// Options for format conversion
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Keep only names matching one of these globs (`core::glob`); empty keeps all.
    pub include: Vec<String>,

    /// Drop names matching one of these globs; exclude wins over include.
    pub exclude: Vec<String>,

    /// Base64-encode all values
    pub base64: bool,

    /// Prefix to remove from the front of a name before `prefix` is added
    /// (`migrate --strip-prefix`). A name that does not start with it is kept.
    pub strip_prefix: Option<String>,

    /// Prefix to add to all keys
    pub prefix: Option<String>,

    /// Key transformation to apply
    pub transform: Option<KeyTransform>,

    /// Two names that become one: refuse, keep the last, or (export) neither.
    pub collisions: Collisions,

    /// `convert --only` (RM-67): keep only the secrets, or only the rest.
    pub only: Option<Only>,
}

/// Which side of the shared classifier (`core::classify::holds_secret`, by
/// name or by value) `convert --only` keeps (RM-67).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Only {
    /// Variables whose name or value says secret.
    Secrets,
    /// Everything else.
    Public,
}

/// Two or more `.env` names that become one name at the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The names in the `.env`, in file order.
    pub from: Vec<String>,
    /// What each became at the target, in the same order.
    pub to: Vec<String>,
}

impl Collision {
    /// `A_B, a_b → a-b`; `A_B → A-B, a_b → a-b` when the spellings differ and
    /// only the target's case rule makes them one; `A_B, a_b` when nothing
    /// renamed them and the target alone ignores case.
    pub fn describe(&self) -> String {
        if self.to.iter().all(|t| *t == self.to[0]) {
            format!("{} → {}", self.from.join(", "), self.to[0])
        } else if self.from == self.to {
            self.from.join(", ")
        } else {
            self.from
                .iter()
                .zip(&self.to)
                .map(|(f, t)| format!("{f} → {t}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// The name whose value is kept under [`Collisions::LastWins`].
    pub fn kept(&self) -> &str {
        self.from.last().map(String::as_str).unwrap_or_default()
    }
}

impl ConvertOptions {
    /// Create new default options
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter variables based on include/exclude patterns
    pub fn filter_vars(&self, vars: &IndexMap<String, String>) -> IndexMap<String, String> {
        vars.iter()
            .filter(|(key, value)| self.should_include(key, value))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Check if a variable should be included: `core::glob`, the rule
    /// `migrate` and `cloud run` use (A-13).
    fn should_include(&self, key: &str, value: &str) -> bool {
        crate::core::glob::admits(key, nonempty(&self.include), nonempty(&self.exclude))
            && self.only.is_none_or(|only| {
                (only == Only::Secrets) == crate::core::classify::holds_secret(key, value)
            })
    }

    /// Transform a key according to the specified transformation
    pub fn transform_key(&self, key: &str) -> String {
        let key = match self.strip_prefix.as_deref() {
            Some(strip) => key.strip_prefix(strip).unwrap_or(key),
            None => key,
        };
        let key = if let Some(ref prefix) = self.prefix {
            format!("{}{}", prefix, key)
        } else {
            key.to_string()
        };

        match self.transform {
            Some(KeyTransform::Uppercase) => key.to_uppercase(),
            Some(KeyTransform::Lowercase) => key.to_lowercase(),
            Some(KeyTransform::CamelCase) => to_camel_case(&key),
            Some(KeyTransform::SnakeCase) => to_snake_case(&key),
            None => key,
        }
    }

    /// Transform a value (base64 encode if enabled)
    pub fn transform_value(&self, value: &str) -> String {
        if self.base64 {
            use base64::{engine::general_purpose, Engine as _};
            general_purpose::STANDARD.encode(value.as_bytes())
        } else {
            value.to_string()
        }
    }

    /// `(name in .env, name at the target, value)` for every variable the
    /// filters admit, in file order.
    fn named<'a, N: Naming + ?Sized>(
        &self,
        vars: &'a IndexMap<String, String>,
        converter: &N,
    ) -> Vec<(&'a str, String, &'a str)> {
        vars.iter()
            .filter(|(k, v)| self.should_include(k, v))
            .map(|(k, v)| {
                let target = converter.target_name(&self.transform_key(k));
                (k.as_str(), target, v.as_str())
            })
            .collect()
    }

    /// Groups of indices into `named` that are one name at the target.
    fn groups<N: Naming + ?Sized>(
        named: &[(&str, String, &str)],
        converter: &N,
    ) -> Vec<Vec<usize>> {
        let mut by_name: IndexMap<String, Vec<usize>> = IndexMap::new();
        for (i, (_, target, _)) in named.iter().enumerate() {
            let key = if converter.names_ignore_case() {
                target.to_lowercase()
            } else {
                target.clone()
            };
            by_name.entry(key).or_default().push(i);
        }
        by_name.into_values().collect()
    }

    /// Every set of names that become one name in `converter`'s output.
    pub fn collisions_in<N: Naming + ?Sized>(
        &self,
        vars: &IndexMap<String, String>,
        converter: &N,
    ) -> Vec<Collision> {
        let named = self.named(vars, converter);
        Self::groups(&named, converter)
            .into_iter()
            .filter(|g| g.len() > 1)
            .map(|g| Collision {
                from: g.iter().map(|&i| named[i].0.to_string()).collect(),
                to: g.iter().map(|&i| named[i].1.clone()).collect(),
            })
            .collect()
    }

    /// The one pipeline every format renders from (A-12): filter, prefix and
    /// transform, the format's own name rule, the collision rule, base64.
    ///
    /// ⚠️ Each format used to repeat filter → rename → encode inline, and some
    /// renamed again on their own (gcp lowercased and hyphenated, terraform
    /// lowercased), after the only collision check had run. So `A_B` and `a_b`
    /// became two `gcloud secrets create a-b` and exit 0 (I-249). The names a
    /// format emits now come from here, and so does the check.
    pub fn prepare<N: Naming + ?Sized>(
        &self,
        vars: &IndexMap<String, String>,
        converter: &N,
    ) -> Result<Vec<(String, String)>> {
        self.prepare_with(vars, converter, "Nothing was written.")
    }

    /// [`prepare`](Self::prepare), ending a refusal with `consequence` — what
    /// the command did not do. `convert` writes; `migrate` prints or sends
    /// (I-104), and "Nothing was written" would not say that.
    pub fn prepare_with<N: Naming + ?Sized>(
        &self,
        vars: &IndexMap<String, String>,
        converter: &N,
        consequence: &str,
    ) -> Result<Vec<(String, String)>> {
        let named = self.named(vars, converter);
        let pick = |i: usize| (named[i].1.clone(), self.transform_value(named[i].2));
        match self.collisions {
            Collisions::Unchecked => Ok((0..named.len()).map(pick).collect()),
            Collisions::Refuse => {
                let clashes = self.collisions_in(vars, converter);
                if !clashes.is_empty() {
                    anyhow::bail!(self.collision_message(&clashes, converter, consequence));
                }
                Ok((0..named.len()).map(pick).collect())
            }
            // Where the first of them stood, with the last one's name and value.
            Collisions::LastWins => Ok(Self::groups(&named, converter)
                .into_iter()
                .map(|g| pick(*g.last().expect("a group is never empty")))
                .collect()),
        }
    }

    /// Warn once per collision that `--allow-collisions` resolved, naming the
    /// value kept. Nothing to say under the other two rules.
    pub fn announce_kept<N: Naming + ?Sized>(&self, vars: &IndexMap<String, String>, naming: &N) {
        if self.collisions != Collisions::LastWins {
            return;
        }
        for c in self.collisions_in(vars, naming) {
            crate::utils::ui::warning(format!(
                "{}: kept the value of {} (--allow-collisions)",
                c.describe(),
                c.kept()
            ));
        }
    }

    fn collision_message<N: Naming + ?Sized>(
        &self,
        clashes: &[Collision],
        converter: &N,
        consequence: &str,
    ) -> String {
        // "after renaming" when `--transform` alone merged them, the format's
        // name otherwise: the reader needs to know which rule to work around.
        let by_format = converter.names_ignore_case()
            || clashes.iter().any(|c| {
                c.from
                    .iter()
                    .zip(&c.to)
                    .any(|(f, t)| self.transform_key(f) != *t)
            });
        let place = if by_format {
            format!("in {}", converter.name())
        } else {
            "after renaming".to_string()
        };
        let mut msg = format!(
            "these names become the same name {place}, and one value would be lost:\n  {}",
            clashes
                .iter()
                .map(Collision::describe)
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        if converter.names_ignore_case() {
            msg.push_str(&format!(
                "\n  ({} treats names that differ only in case as one name.)",
                converter.name()
            ));
        }
        let without = if self.transform.is_some() {
            ", convert without the transform"
        } else if self.strip_prefix.is_some() {
            ", migrate without --strip-prefix"
        } else {
            ""
        };
        msg.push_str(&format!(
            "\nRename one in the .env{without}, or pass --allow-collisions to keep the last \
             value. {consequence}"
        ));
        msg
    }
}

/// End converted output with exactly one newline.
///
/// # Why this exists, and why it is shared
///
/// A [`Converter`] returns a `String` and makes its own choice about a trailing
/// newline: the JSON-shaped ones end without, the line-oriented ones end with.
/// That left `convert` with two bugs at once, because its two write paths each
/// applied a different rule on top:
///
/// ```text
///                   converter ends \n?   stdout (println!)   --output (fs::write)
///   json, vercel …         no            one \n   ✅          none     ❌  D22
///   yaml, shell …         yes            two \n   ❌  D23      one \n   ✅
/// ```
///
/// So `convert --to json > a` and `convert --to json -o b` produced different
/// bytes — **all 29 format spellings differed** — and 22 of them printed a
/// trailing blank line. Neither is a property anyone chose; both fall out of
/// which function happened to do the writing.
///
/// Normalising here fixes both at once and puts the decision in one place
/// rather than in fourteen converters. ⚠️ Add a format and it inherits this —
/// which is the point.
pub fn newline_terminated(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Converter trait for format conversion
pub trait Converter {
    /// Convert environment variables to the target format
    ///
    /// # Arguments
    ///
    /// * `vars` - Environment variables to convert
    /// * `options` - Conversion options (filtering, transformations, etc.)
    ///
    /// # Returns
    ///
    /// Formatted output as a string
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String>;

    /// Get the name of this format
    ///
    /// Used for format selection and error messages.
    fn name(&self) -> &str;

    /// Get a description of this format
    ///
    /// Used in help text and interactive selection.
    fn description(&self) -> &str;

    /// The name `key` — already prefixed and transformed — has at the target.
    ///
    /// The name as it is, unless the format renames. ⚠️ A format that renames
    /// says so here rather than in its own loop, so [`ConvertOptions::prepare`]
    /// can see two names becoming one (I-249).
    fn target_name(&self, key: &str) -> String {
        key.to_string()
    }

    /// Whether the target treats names that differ only in case as one
    /// (Azure Key Vault, GitHub secrets).
    fn names_ignore_case(&self) -> bool {
        false
    }
}

/// How a destination names what it is given: the part of a [`Converter`]
/// that [`ConvertOptions::prepare`] needs to see two names becoming one.
///
/// ⚠️ Its own trait so `migrate`, whose destinations are not converters, runs
/// the same pipeline (A-12, I-104). `migrate` had a third copy of
/// filter → strip → add with no collision check, so `--strip-prefix API_`
/// on `API_X` and `X` printed one `X` and lost a value. Every converter is a
/// `Naming` through the impl below; nothing in `formats/` repeats it.
pub trait Naming {
    /// Shown in a refusal: "these names become the same name in <name>".
    fn name(&self) -> &str;

    /// See [`Converter::target_name`].
    fn target_name(&self, key: &str) -> String {
        key.to_string()
    }

    /// See [`Converter::names_ignore_case`].
    fn names_ignore_case(&self) -> bool {
        false
    }
}

impl<C: Converter + ?Sized> Naming for C {
    fn name(&self) -> &str {
        Converter::name(self)
    }

    fn target_name(&self, key: &str) -> String {
        Converter::target_name(self, key)
    }

    fn names_ignore_case(&self) -> bool {
        Converter::names_ignore_case(self)
    }
}

// Helper functions

/// A pattern list for `core::glob::admits`: none when empty.
fn nonempty(v: &[String]) -> Option<&[String]> {
    (!v.is_empty()).then_some(v)
}

/// Convert string to camelCase
fn to_camel_case(s: &str) -> String {
    let parts: Vec<&str> = s.split('_').collect();
    if parts.is_empty() {
        return String::new();
    }

    let mut result = parts[0].to_lowercase();
    for part in &parts[1..] {
        if !part.is_empty() {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                result.push_str(&first.to_uppercase().to_string());
                result.push_str(&chars.as_str().to_lowercase());
            }
        }
    }
    result
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();

    for i in 0..chars.len() {
        let ch = chars[i];

        if ch.is_uppercase() {
            if i > 0 {
                let prev = chars[i - 1];
                let next = chars.get(i + 1);

                if prev.is_lowercase()
                    || (prev.is_uppercase() && next.map(|c| c.is_lowercase()).unwrap_or(false))
                {
                    result.push('_');
                }
            }
            result.extend(ch.to_lowercase());
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A format that lowercases and hyphenates, like gcp.
    struct Hyphens;
    impl Converter for Hyphens {
        fn convert(&self, _: &IndexMap<String, String>, _: &ConvertOptions) -> Result<String> {
            Ok(String::new())
        }
        fn name(&self) -> &str {
            "hyphens"
        }
        fn description(&self) -> &str {
            ""
        }
        fn target_name(&self, key: &str) -> String {
            key.to_lowercase().replace('_', "-")
        }
    }

    /// A format whose target ignores case, like Key Vault.
    struct Folds;
    impl Converter for Folds {
        fn convert(&self, _: &IndexMap<String, String>, _: &ConvertOptions) -> Result<String> {
            Ok(String::new())
        }
        fn name(&self) -> &str {
            "folds"
        }
        fn description(&self) -> &str {
            ""
        }
        fn names_ignore_case(&self) -> bool {
            true
        }
    }

    struct Plain;
    impl Converter for Plain {
        fn convert(&self, _: &IndexMap<String, String>, _: &ConvertOptions) -> Result<String> {
            Ok(String::new())
        }
        fn name(&self) -> &str {
            "plain"
        }
        fn description(&self) -> &str {
            ""
        }
    }

    #[test]
    fn test_default_options() {
        let opts = ConvertOptions::default();
        assert!(opts.include.is_empty());
        assert!(opts.exclude.is_empty());
        assert!(!opts.base64);
        assert!(opts.prefix.is_none());
        assert!(opts.transform.is_none());
        assert_eq!(opts.collisions, Collisions::Unchecked);
    }

    #[test]
    fn test_new_options() {
        let opts = ConvertOptions::new();
        assert!(opts.include.is_empty());
    }

    #[test]
    fn test_filter_vars_no_pattern() {
        let opts = ConvertOptions::default();
        let filtered = opts.filter_vars(&vars(&[("KEY1", "value1"), ("KEY2", "value2")]));
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_vars_include() {
        let opts = ConvertOptions {
            include: vec!["AWS_*".to_string()],
            ..Default::default()
        };
        let filtered = opts.filter_vars(&vars(&[("AWS_KEY", "1"), ("DB_KEY", "2")]));
        assert_eq!(filtered.keys().collect::<Vec<_>>(), ["AWS_KEY"]);
    }

    #[test]
    fn test_filter_vars_exclude() {
        let opts = ConvertOptions {
            exclude: vec!["*_LOCAL".to_string()],
            ..Default::default()
        };
        let filtered = opts.filter_vars(&vars(&[("KEY1", "1"), ("KEY2_LOCAL", "2")]));
        assert_eq!(filtered.keys().collect::<Vec<_>>(), ["KEY1"]);
    }

    /// A-13: the patterns `core::glob` takes — `?`, a `*` in the middle, a
    /// list — and exclude wins.
    #[test]
    fn t_i090_filters_are_core_glob() {
        let v = vars(&[
            ("PORT", "1"),
            ("DB_PRIMARY_URL", "2"),
            ("DB_PRIMARY_HOST", "3"),
        ]);
        let keys = |include: &[&str], exclude: &[&str]| {
            let opts = ConvertOptions {
                include: include.iter().map(|s| s.to_string()).collect(),
                exclude: exclude.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            opts.filter_vars(&v).keys().cloned().collect::<Vec<_>>()
        };
        assert_eq!(keys(&["P?RT"], &[]), ["PORT"]);
        assert_eq!(keys(&["DB_*_URL"], &[]), ["DB_PRIMARY_URL"]);
        assert_eq!(
            keys(&["PORT", "DB_*_HOST"], &[]),
            ["PORT", "DB_PRIMARY_HOST"]
        );
        assert_eq!(keys(&["DB_*"], &["*_HOST"]), ["DB_PRIMARY_URL"]);
    }

    #[test]
    fn test_transform_key_prefix() {
        let opts = ConvertOptions {
            prefix: Some("APP_".to_string()),
            ..Default::default()
        };
        assert_eq!(opts.transform_key("DATABASE_URL"), "APP_DATABASE_URL");
    }

    #[test]
    fn test_transform_key_uppercase() {
        let opts = ConvertOptions {
            transform: Some(KeyTransform::Uppercase),
            ..Default::default()
        };
        assert_eq!(opts.transform_key("database_url"), "DATABASE_URL");
    }

    #[test]
    fn test_transform_key_lowercase() {
        let opts = ConvertOptions {
            transform: Some(KeyTransform::Lowercase),
            ..Default::default()
        };
        assert_eq!(opts.transform_key("DATABASE_URL"), "database_url");
    }

    #[test]
    fn test_transform_value_base64() {
        let opts = ConvertOptions {
            base64: true,
            ..Default::default()
        };
        assert_eq!(opts.transform_value("hello"), "aGVsbG8=");
    }

    #[test]
    fn t_i090_transform_parse_is_the_one_list_rule() {
        assert_eq!(
            KeyTransform::parse("CamelCase").unwrap(),
            KeyTransform::CamelCase
        );
        assert_eq!(
            KeyTransform::parse("snake_case").unwrap(),
            KeyTransform::SnakeCase
        );
        let err = KeyTransform::parse("Title").unwrap_err().to_string();
        assert!(err.contains("unknown transform 'title'"), "{err}");
        assert!(
            err.contains("uppercase, lowercase, camelCase, snake_case"),
            "{err}"
        );
    }

    /// I-249: the format's own rename is part of the check.
    #[test]
    fn t_i249_a_format_rename_is_a_collision() {
        let v = vars(&[("A_B", "one"), ("a_b", "two"), ("PORT", "1")]);
        let refuse = ConvertOptions {
            collisions: Collisions::Refuse,
            ..Default::default()
        };
        let err = refuse.prepare(&v, &Hyphens).unwrap_err().to_string();
        assert!(err.contains("in hyphens"), "{err}");
        assert!(err.contains("A_B, a_b → a-b"), "{err}");
        assert!(err.contains("--allow-collisions"), "{err}");

        // The target folds case: two spellings, one name.
        let err = refuse.prepare(&v, &Folds).unwrap_err().to_string();
        assert!(err.contains("  A_B, a_b\n"), "{err}");
        assert!(err.contains("differ only in case"), "{err}");

        // Names kept as they are: no collision.
        assert_eq!(refuse.prepare(&v, &Plain).unwrap().len(), 3);
    }

    /// DEC-PL2: the last value, where the first name stood.
    #[test]
    fn t_i249_last_wins_keeps_the_last_value_in_the_first_place() {
        let v = vars(&[("A_B", "one"), ("PORT", "1"), ("a_b", "two")]);
        let opts = ConvertOptions {
            collisions: Collisions::LastWins,
            ..Default::default()
        };
        assert_eq!(
            opts.prepare(&v, &Hyphens).unwrap(),
            [
                ("a-b".to_string(), "two".to_string()),
                ("port".to_string(), "1".to_string())
            ]
        );
        let found = opts.collisions_in(&v, &Hyphens);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kept(), "a_b");
    }

    /// I-088: the transform alone says "after renaming" and names the flag.
    #[test]
    fn t_i088_a_transform_collision_says_after_renaming() {
        let v = vars(&[("Api", "1"), ("API", "2")]);
        let opts = ConvertOptions {
            transform: Some(KeyTransform::Uppercase),
            collisions: Collisions::Refuse,
            ..Default::default()
        };
        let err = opts.prepare(&v, &Plain).unwrap_err().to_string();
        assert!(err.contains("after renaming"), "{err}");
        assert!(err.contains("Api, API → API"), "{err}");
        assert!(err.contains("convert without the transform"), "{err}");
    }

    /// `cloud export` converts unchecked: what it printed before 0.10.9.
    #[test]
    fn unchecked_keeps_every_name() {
        let v = vars(&[("A_B", "one"), ("a_b", "two")]);
        let opts = ConvertOptions::default();
        assert_eq!(opts.prepare(&v, &Hyphens).unwrap().len(), 2);
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("database_url"), "databaseUrl");
        assert_eq!(to_camel_case("secret_key"), "secretKey");
        assert_eq!(to_camel_case("aws_access_key_id"), "awsAccessKeyId");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("DatabaseURL"), "database_url");
        assert_eq!(to_snake_case("SecretKey"), "secret_key");
        assert_eq!(to_snake_case("HTTPServer"), "http_server");
        assert_eq!(to_snake_case("UserID"), "user_id");
        assert_eq!(to_snake_case("MySQLDatabase"), "my_sql_database");
        assert_eq!(to_snake_case("JSONParser"), "json_parser");
        assert_eq!(to_snake_case("Already_snake"), "already_snake");
    }
}
