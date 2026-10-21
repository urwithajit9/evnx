//! The placeholder value written for a variable whose real value must not be
//! (DEC-S19, U3 A-23's sibling).
//!
//! Moved from `commands::sync::placeholder` in 0.10.12: `add custom` writes the
//! same placeholders into `.env.example` and imported them from the sync
//! command, the one place a command reached into another for a value rule.
//! `sync` and `add` use this module; `init` (`schema::resolver::placeholder_for`,
//! `https://example.com`) and `validate --fix` (`fixer::fill_for_missing`)
//! keep their own shapes on purpose (R-init, R-validate notes).
//!
//! ⚠️ Not a placeholder *judgement*: that is `core::classify::is_placeholder`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Configuration for custom placeholder templates.
///
/// Loaded from the JSON file named by `--template-config`.
///
/// ⚠️ **Unknown keys are refused** (I-121, DEC-S11). serde dropped them
/// without a word, so `{"allow_actaul": [...]}` loaded, did nothing, and the
/// operator believed the setting was in force.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceholderConfig {
    /// Declared for editor tooling; evnx does not read it.
    ///
    /// Present as a field rather than ignored silently because the published
    /// example config carries it, and serde drops unknown keys without a word —
    /// so an operator has no way to tell "accepted and unused" from "misspelled
    /// and dropped". Round-tripping it also means `Serialize` does not delete it
    /// from a file evnx rewrites.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// A human-readable note. Same reasoning as [`schema`](Self::schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Pattern-to-placeholder mappings (regex pattern → placeholder string).
    ///
    /// ⚠️ **An `IndexMap`, not a `HashMap`, and that is load-bearing.** More than
    /// one pattern can match one key, and a `HashMap` iterates in an order that
    /// Rust randomises per process — so the winner changed between runs of the
    /// same command, and the winner is written into `.env.example`, a committed
    /// file. Eight identical runs produced four different answers.
    ///
    /// Declaration order is the last tie-break in
    /// [`generate_placeholder`], which
    /// needs the order the file was written in.
    #[serde(default)]
    pub patterns: IndexMap<String, String>,

    /// Default placeholder for unmatched keys
    #[serde(default = "default_placeholder")]
    pub default: String,

    /// Keys whose **real value** is written to the template instead of a
    /// placeholder.
    ///
    /// For the part of a `.env` that is configuration rather than credentials —
    /// `APP_NAME`, `LOG_LEVEL`, `ENVIRONMENT` — where the actual value is the
    /// useful example and `YOUR_VALUE_HERE` is strictly worse. It also makes
    /// `sync --reverse` deliver working defaults to a teammate's `.env`.
    ///
    /// ⚠️ **Guarded, not trusted.** A listed key whose value looks like a
    /// credential fails the sync by name rather than being copied — see
    /// `sync`'s `check_allow_actual`. Without that
    /// this field is a documented way to commit a live secret.
    #[serde(default)]
    pub allow_actual: Vec<String>,

    /// Documented in `sync-configuration.mdx`, and **not implemented here.**
    ///
    /// Naming policy is real, but it is set by `--naming-policy` or `[sync]
    /// naming_policy` in `.evnx.toml`, not by this file. Kept as a field so
    /// [`PlaceholderConfig::from_path`] can say so, instead of serde discarding
    /// it and the operator believing a policy is in force that is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming_convention: Option<serde_json::Value>,
}

fn default_placeholder() -> String {
    String::from("YOUR_VALUE_HERE")
}

impl PlaceholderConfig {
    /// Load configuration from a JSON file.
    ///
    /// # Errors
    ///
    /// The file cannot be read or is not valid JSON, or a key of `patterns` is
    /// not a valid regular expression.
    ///
    /// ⚠️ That last one used to be silent. `generate_placeholder` compiled each
    /// pattern with `if let Ok(re)`, so an expression with an unbalanced
    /// parenthesis was **skipped**, the key fell through to `default`, and the
    /// operator got a template that looked fine and did not follow their rules.
    /// `evnx scan` already refuses to run an uncompilable `--pattern` for the
    /// same reason; this brings the two into line.
    pub fn from_path<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read placeholder config file: {}", e))?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse placeholder config as JSON: {}", e))?;
        refuse_unknown_keys(&json, path)?;
        let config: Self = serde_json::from_value(json)
            .map_err(|e| anyhow::anyhow!("Failed to parse placeholder config as JSON: {}", e))?;
        config.validate(path)?;
        Ok(config)
    }

    /// Reject what would otherwise fail silently, and say what is ignored.
    fn validate(&self, path: &std::path::Path) -> anyhow::Result<()> {
        for pattern in self.patterns.keys() {
            regex::Regex::new(&format!("(?i){}", pattern)).map_err(|e| {
                anyhow::anyhow!(
                    "{} declares a placeholder pattern that is not a valid regular \
                     expression: {pattern:?}\n  {e}\n  \
                     Nothing was written. Fix the pattern, or remove it.",
                    path.display()
                )
            })?;
        }

        // A warning rather than an error: the field is in the published example,
        // so refusing it would fail every config copied from the docs.
        if self.naming_convention.is_some() {
            crate::utils::ui::warning(format!(
                "{} sets `naming_convention`, which evnx does not read from this \
                 file. Use --naming-policy, or [sync] naming_policy in .evnx.toml.",
                path.display()
            ));
        }

        Ok(())
    }
}

/// Every key a placeholder config may hold.
pub const CONFIG_KEYS: &[&str] = &[
    "$schema",
    "description",
    "patterns",
    "default",
    "allow_actual",
    "naming_convention",
];

/// Refuse a top-level key evnx does not read, naming it, the key it was
/// probably meant to be, and the ones that exist (I-121, DEC-S11). Before the
/// typed parse, so the message is evnx's rather than serde's.
fn refuse_unknown_keys(json: &serde_json::Value, path: &std::path::Path) -> anyhow::Result<()> {
    let Some(map) = json.as_object() else {
        return Ok(()); // not an object: the typed parse says so
    };
    let unknown: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| !CONFIG_KEYS.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    // Each with its own "did you mean", so two typos get two answers.
    let all = unknown
        .iter()
        .map(|k| {
            match crate::utils::string::closest(k, CONFIG_KEYS.iter().map(|c| (*c, *c))).first() {
                Some(m) => format!("`{k}` (did you mean `{m}`?)"),
                None => format!("`{k}`"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "{} has a key evnx does not read: {all}\n  \
         Accepted keys: {}.\n  \
         Nothing was written: a misspelt key would leave the setting it names off.",
        path.display(),
        CONFIG_KEYS.join(", ")
    )
}

impl Default for PlaceholderConfig {
    fn default() -> Self {
        Self {
            schema: None,
            description: None,
            patterns: IndexMap::new(),
            default: String::from("YOUR_VALUE_HERE"),
            allow_actual: Vec::new(),
            naming_convention: None,
        }
    }
}

/// Generate placeholder using built-in rules + custom config
pub fn generate_placeholder(
    key: &str,
    value: Option<&String>,
    config: &PlaceholderConfig,
) -> String {
    // The project asked for this key's real value to go into the template.
    //
    // ⚠️ Safe only because `executor::check_allow_actual` has already refused the
    // whole sync if any listed value looks like a credential. Reaching here means
    // the value was examined and is configuration, not a secret.
    if config.allow_actual.iter().any(|k| k == key) {
        if let Some(v) = value {
            return v.clone();
        }
    }

    // Then the project's own patterns, most specific first.
    if let Some(placeholder) = best_pattern(key, config) {
        return placeholder;
    }

    // Built-in rules, by whole words of the name (I-120, DEC-S10). They were
    // substring matches: `SUPPORT_EMAIL` got `8000` (PORT), `MONKEY_COUNT`
    // `YOUR_KEY_HERE` (KEY), `BYPASS_CACHE` a password (PASS), `GHOST_MODE`
    // `localhost` (HOST). First match wins, in the order they always had.
    use crate::core::classify::has_word;

    if has_word(key, &["SECRET", "KEY", "TOKEN", "APIKEY"]) {
        return "YOUR_KEY_HERE".to_string();
    }

    if has_word(key, &["PASSWORD", "PASSWD", "PASS", "PWD"]) {
        return "YOUR_PASSWORD_HERE".to_string();
    }

    if has_word(key, &["URL", "URI", "DSN"]) {
        return value
            .and_then(|v| url_placeholder(v))
            .unwrap_or("YOUR_URL_HERE")
            .to_string();
    }

    if has_word(key, &["PORT"]) {
        return "8000".to_string();
    }

    // ⚠️ The value, when it is a switch. `DEBUG=false` became `DEBUG=true`,
    // turning debugging on for whoever copies the template (DEC-S10). A switch
    // is not a secret; anything else under the name falls back as before.
    if has_word(key, &["DEBUG"]) {
        return match value {
            Some(v) if is_switch(v) => v.trim().to_string(),
            _ => "true".to_string(),
        };
    }

    if has_word(key, &["HOST", "SERVER", "HOSTNAME"]) {
        return "localhost".to_string();
    }

    // Use config default, fallback to hardcoded default if empty
    if config.default.is_empty() {
        "YOUR_VALUE_HERE".to_string()
    } else {
        config.default.clone()
    }
}

/// The placeholder of the most specific pattern matching `key`.
///
/// # Why this is not "the first one that matches"
///
/// It was, over a `HashMap`, and more than one pattern routinely matches one key
/// — `STRIPE_SECRET_KEY` matches `SECRET`, `_KEY$` and `^STRIPE_` at once. Rust
/// randomises `HashMap` iteration per process, so the winner changed from run to
/// run, and the winner is written into `.env.example`, **a committed file**.
/// Eight identical runs produced four different answers: every one of the four
/// patterns won at least once. So `evnx sync` produced a spurious `git diff` on
/// a project whose `.env` had not changed at all.
///
/// # The order
///
/// 1. **An exact key match**, which is the project naming the variable outright.
/// 2. **The longest pattern.** A crude specificity measure, but the right way
///    round for the expressions people actually write: `.*_SECRET_KEY$` beats
///    `SECRET`, and anything beats `.*`. It also settles the anchored spelling of
///    an exact match without special-casing it — `^STRIPE_SECRET_KEY$` is longer
///    than every pattern that could match the same key by accident.
/// 3. **Declaration order**, which is why `patterns` is an `IndexMap`. Two
///    equally long patterns matching one key is a genuine ambiguity, and the
///    file is the only place that expresses which the author meant.
///
/// Every rung is total, so the result depends on the config file and the key and
/// nothing else.
fn best_pattern(key: &str, config: &PlaceholderConfig) -> Option<String> {
    config
        .patterns
        .iter()
        .enumerate()
        .filter(|(_, (pattern, _))| {
            // An expression that does not compile cannot match. It is also
            // refused at load time by `PlaceholderConfig::from_path`, so this is
            // only reachable for a config built in memory.
            regex::Regex::new(&format!("(?i){}", pattern))
                .map(|re| re.is_match(key))
                .unwrap_or(false)
        })
        .max_by_key(|(index, (pattern, _))| {
            (
                pattern.eq_ignore_ascii_case(key),
                pattern.len(),
                // `max_by_key` keeps the last maximum, so declaration order has
                // to be reversed to make the *first* declaration win.
                std::cmp::Reverse(*index),
            )
        })
        .map(|(_, (_, placeholder))| placeholder.clone())
}

/// What a URL placeholder looks like for each scheme sync recognises
/// (DEC-S10). Credentials are always the words `user` and `password`.
///
/// ⚠️ Only `postgresql://`, `redis://` and `http(s)://` were recognised, by
/// substring, so `postgres://…` — the spelling Heroku, Render and Supabase
/// hand out — and every `mysql://`, `mongodb://` and `amqp://` URL became
/// `YOUR_URL_HERE`.
pub const URL_PLACEHOLDERS: &[(&str, &str)] = &[
    (
        "postgresql",
        "postgresql://user:password@localhost:5432/dbname",
    ),
    ("postgres", "postgres://user:password@localhost:5432/dbname"),
    ("mysql", "mysql://user:password@localhost:3306/dbname"),
    ("mongodb", "mongodb://user:password@localhost:27017/dbname"),
    (
        "mongodb+srv",
        "mongodb+srv://user:password@cluster.example.net/dbname",
    ),
    ("amqp", "amqp://user:password@localhost:5672/"),
    ("amqps", "amqps://user:password@localhost:5671/"),
    ("redis", "redis://localhost:6379/0"),
    ("rediss", "rediss://localhost:6379/0"),
    ("http", "https://your-api-url-here.com"),
    ("https", "https://your-api-url-here.com"),
];

/// Whether `value` is a URL in the shape [`URL_PLACEHOLDERS`] writes: the
/// words `user` and `password` as credentials, on `localhost` or a
/// documentation domain (N-20).
///
/// `scan` asks, because since I-250 three blueprints write
/// `postgresql://user:password@localhost:5432/dbname` into a new `.env`, and
/// the connection-string rule reported evnx's own placeholder as a live
/// credential. The host matters: `user:password@db.prod.company.io` is not
/// something evnx wrote, so it stays a finding.
pub fn is_url_placeholder(value: &str) -> bool {
    let v = value.trim();
    let Some((_, rest)) = v.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return false;
    };
    let host = host
        .rsplit_once(':')
        .map_or(host, |(h, _)| h)
        .to_ascii_lowercase();
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    let documentation = ["example.com", "example.org", "example.net"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")));
    userinfo == "user:password" && (local || documentation)
}

/// The placeholder for a URL by its scheme, or `None` for a scheme not in
/// [`URL_PLACEHOLDERS`].
fn url_placeholder(value: &str) -> Option<&'static str> {
    let (scheme, _) = value.trim().split_once("://")?;
    URL_PLACEHOLDERS
        .iter()
        .find(|(s, _)| s.eq_ignore_ascii_case(scheme))
        .map(|(_, p)| *p)
}

/// `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`, any case.
fn is_switch(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "false" | "1" | "0" | "yes" | "no" | "on" | "off"
    )
}

/// Whether `value` is one sync itself would write: a configured placeholder,
/// the configured default, or one of the generator's stock values. For the
/// `--dry-run` preview's "placeholder" label.
///
/// ⚠️ Not a placeholder *judgement* (DEC-G12): `localhost`, `8000` and `true`
/// are here because sync generates them, not because they are holes. The
/// shared judgement is `core::classify::is_placeholder`; this was renamed from
/// `is_placeholder_value` so it stops reading like a fourth copy of it.
///
/// ⚠️ This consulted `config.default` and a list of hardcoded strings, and
/// **ignored `config.patterns` entirely** — so a project whose convention is
/// `<set-me>` had every one of its own placeholders reported as a real value.
/// Display only, in the `--dry-run` preview, which is why it went unnoticed; but
/// the preview is exactly where someone decides whether a template is ready.
pub fn is_generated_value(value: &str, config: &PlaceholderConfig) -> bool {
    config.patterns.values().any(|p| p == value)
        || value == config.default
        || value.contains("YOUR_")
        || value.contains("placeholder")
        || value == "localhost"
        || value == "8000"
        || value == "true"
        || value == "false"
        || URL_PLACEHOLDERS.iter().any(|(_, p)| *p == value)
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_config_default() {
        let config = PlaceholderConfig::default();
        assert_eq!(config.default, "YOUR_VALUE_HERE");
        assert!(config.patterns.is_empty());
        assert!(config.allow_actual.is_empty());
    }

    #[test]
    fn test_placeholder_config_serialization() {
        let config = PlaceholderConfig {
            patterns: IndexMap::from([("API_.*".to_string(), "api-key".to_string())]),
            default: "custom".to_string(),
            allow_actual: vec!["PUBLIC_KEY".to_string()],
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: PlaceholderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.default, "custom");
        assert!(parsed.patterns.contains_key("API_.*"));
        assert_eq!(parsed.allow_actual, vec!["PUBLIC_KEY"]);
    }

    #[test]
    fn test_placeholder_config_from_path_success() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"default": "TEST_DEFAULT", "patterns": {{"KEY_.*": "test"}}}}"#
        )
        .unwrap();

        let config = PlaceholderConfig::from_path(file.path()).unwrap();
        assert_eq!(config.default, "TEST_DEFAULT");
        assert!(config.patterns.contains_key("KEY_.*"));
    }

    #[test]
    fn test_placeholder_config_from_path_invalid_json() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        // writeln!(file, r#"{"invalid": json}"#).unwrap();
        writeln!(file, r#"{{"invalid": json}}"#).unwrap();

        let result = PlaceholderConfig::from_path(file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("JSON"));
    }

    #[test]
    fn test_placeholder_config_from_path_missing_file() {
        let result = PlaceholderConfig::from_path("/nonexistent/path.json");
        assert!(result.is_err());
    }

    // SyncPreview and VarChange are simple data structs - minimal tests needed

    /// Four patterns, all matching `STRIPE_SECRET_KEY`. Which one wins used to
    /// depend on `HashMap` iteration order, and the answer is written into a
    /// committed file. These pin the order instead.
    fn overlapping() -> PlaceholderConfig {
        let mut config = PlaceholderConfig::default();
        for (pattern, placeholder) in [
            ("SECRET", "<any-secret>"),
            ("_KEY$", "<any-key>"),
            ("^STRIPE_", "<stripe-thing>"),
            ("STRIPE_SECRET_KEY", "<the-exact-key>"),
        ] {
            config
                .patterns
                .insert(pattern.to_string(), placeholder.to_string());
        }
        config
    }

    #[test]
    fn an_exact_key_match_beats_every_regex() {
        let config = overlapping();
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<the-exact-key>"
        );
        // Declaration order alone would have picked `<any-secret>`, so this is
        // not the first rung passing by accident.
        assert_eq!(
            config.patterns.keys().next().map(String::as_str),
            Some("SECRET")
        );
    }

    #[test]
    fn the_longest_pattern_wins_when_none_is_exact() {
        let mut config = overlapping();
        config.patterns.shift_remove("STRIPE_SECRET_KEY");
        // `^STRIPE_` (8) over `SECRET` (6) and `_KEY$` (5).
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<stripe-thing>"
        );
    }

    #[test]
    fn declaration_order_settles_equally_long_patterns() {
        let mut config = PlaceholderConfig::default();
        // Both six characters, both match, so only the file can say which.
        config
            .patterns
            .insert("SECRET".to_string(), "<first>".to_string());
        config
            .patterns
            .insert("^STRIP".to_string(), "<second>".to_string());
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &config),
            "<first>"
        );

        // Reversed in the file, reversed in the answer — proving it reads the
        // order rather than happening to agree with it.
        let mut reversed = PlaceholderConfig::default();
        reversed
            .patterns
            .insert("^STRIP".to_string(), "<second>".to_string());
        reversed
            .patterns
            .insert("SECRET".to_string(), "<first>".to_string());
        assert_eq!(
            generate_placeholder("STRIPE_SECRET_KEY", None, &reversed),
            "<second>"
        );
    }

    #[test]
    fn a_pattern_matching_nothing_is_ignored_rather_than_winning_on_length() {
        let mut config = PlaceholderConfig::default();
        config.patterns.insert(
            "A_VERY_LONG_PATTERN_THAT_MATCHES_NOTHING".to_string(),
            "<no>".to_string(),
        );
        config
            .patterns
            .insert("KEY".to_string(), "<yes>".to_string());
        assert_eq!(generate_placeholder("API_KEY", None, &config), "<yes>");
    }

    #[test]
    fn allow_actual_writes_the_real_value_and_outranks_a_matching_pattern() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("LOG_LEVEL".to_string(), "<a-level>".to_string());
        config.allow_actual.push("LOG_LEVEL".to_string());

        let value = "debug".to_string();
        assert_eq!(
            generate_placeholder("LOG_LEVEL", Some(&value), &config),
            "debug"
        );
    }

    #[test]
    fn allow_actual_with_no_value_falls_back_to_a_placeholder() {
        // The reverse direction has no real value to copy. It must not produce an
        // empty string, which would read as "this variable is blank".
        let mut config = PlaceholderConfig::default();
        config.allow_actual.push("APP_NAME".to_string());
        assert_eq!(
            generate_placeholder("APP_NAME", None, &config),
            "YOUR_VALUE_HERE"
        );
    }

    #[test]
    fn a_projects_own_placeholder_is_recognised_as_a_placeholder() {
        // ⚠️ `is_placeholder_value` ignored `patterns` entirely, so a project
        // whose convention is `<set-me>` had all of its own placeholders reported
        // as real values in the dry-run preview.
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("API_.*".to_string(), "<set-me>".to_string());

        assert!(is_generated_value("<set-me>", &config));
        assert!(!is_generated_value("sk_live_abc123", &config));
    }

    #[test]
    fn test_generate_placeholder_builtin_rules() {
        let config = PlaceholderConfig::default();

        assert_eq!(
            generate_placeholder("SECRET_KEY", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("API_TOKEN", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("DB_PASSWORD", None, &config),
            "YOUR_PASSWORD_HERE"
        );
        assert_eq!(generate_placeholder("PORT", None, &config), "8000");
        assert_eq!(generate_placeholder("DEBUG_MODE", None, &config), "true");
        assert_eq!(generate_placeholder("DB_HOST", None, &config), "localhost");
        assert_eq!(
            generate_placeholder("RANDOM_VAR", None, &config),
            "YOUR_VALUE_HERE"
        );
    }

    #[test]
    fn test_generate_placeholder_with_url_values() {
        let config = PlaceholderConfig::default();

        let pg_url = "postgresql://user:pass@localhost:5432/db";
        assert!(
            generate_placeholder("DATABASE_URL", Some(&pg_url.to_string()), &config)
                .contains("postgresql")
        );

        let redis_url = "redis://localhost:6379/0";
        assert!(
            generate_placeholder("REDIS_URL", Some(&redis_url.to_string()), &config)
                .contains("redis")
        );

        // ✅ FIXED: Use a key that actually contains "URL"
        let https_url = "https://api.example.com/v1";
        assert!(
            generate_placeholder("API_URL", Some(&https_url.to_string()), &config) // Changed from API_ENDPOINT
                .contains("https://")
        );
    }

    #[test]
    fn test_generate_placeholder_custom_config() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("AWS_.*".to_string(), "aws-placeholder".to_string());
        config.default = "CUSTOM_DEFAULT".to_string();

        assert_eq!(
            generate_placeholder("AWS_SECRET", None, &config),
            "aws-placeholder"
        );
        assert_eq!(
            generate_placeholder("UNKNOWN_VAR", None, &config),
            "CUSTOM_DEFAULT"
        );
    }

    #[test]
    fn test_custom_pattern_matching() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("STRIPE_.*".to_string(), "stripe_test_key".to_string());
        config
            .patterns
            .insert(".*_PORT".to_string(), "3000".to_string());

        assert_eq!(
            generate_placeholder("STRIPE_API_KEY", None, &config),
            "stripe_test_key"
        );
        assert_eq!(
            generate_placeholder("STRIPE_SECRET", None, &config),
            "stripe_test_key"
        );
        assert_eq!(generate_placeholder("SERVER_PORT", None, &config), "3000");
        assert_eq!(generate_placeholder("APP_PORT", None, &config), "3000");
    }

    #[test]
    fn test_custom_pattern_case_insensitive() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("api_.*".to_string(), "lowercase-pattern".to_string());

        // Should match regardless of case due to (?i) flag
        assert_eq!(
            generate_placeholder("API_KEY", None, &config),
            "lowercase-pattern"
        );
        assert_eq!(
            generate_placeholder("api_token", None, &config),
            "lowercase-pattern"
        );
        assert_eq!(
            generate_placeholder("Api_Secret", None, &config),
            "lowercase-pattern"
        );
    }

    #[test]
    fn test_invalid_regex_pattern_fallback() {
        let mut config = PlaceholderConfig::default();
        // Invalid regex pattern - should not panic, should fallback
        config
            .patterns
            .insert("[invalid(regex".to_string(), "should-not-match".to_string());

        // Should fallback to built-in rules or default
        let result = generate_placeholder("TEST_KEY", None, &config);
        assert_eq!(result, "YOUR_KEY_HERE"); // Built-in rule for KEY
    }

    #[test]
    fn test_is_generated_value() {
        let config = PlaceholderConfig::default();

        assert!(is_generated_value("YOUR_VALUE_HERE", &config));
        assert!(is_generated_value("YOUR_KEY_HERE", &config));
        assert!(is_generated_value("localhost", &config));
        assert!(is_generated_value("placeholder", &config));
        assert!(!is_generated_value("sk_live_abc123", &config));
        assert!(!is_generated_value("production-db.example.com", &config));
        assert!(!is_generated_value("my-secret-value", &config));
    }

    #[test]
    fn test_is_placeholder_value_with_custom_config() {
        let config = PlaceholderConfig {
            default: "CUSTOM_PLACEHOLDER".to_string(),
            ..Default::default()
        };

        assert!(is_generated_value("CUSTOM_PLACEHOLDER", &config));
        // ✅ FIXED: "YOUR_VALUE_HERE" still matches because of .contains("YOUR_") check
        // This is intentional behavior - we detect common placeholder patterns
        assert!(is_generated_value("YOUR_VALUE_HERE", &config)); // Still true due to "YOUR_" check

        // Test a value that should NOT be detected as placeholder
        assert!(!is_generated_value("my-actual-secret-123", &config));
    }

    // NEW: Edge cases
    #[test]
    fn test_generate_placeholder_empty_key() {
        let config = PlaceholderConfig::default();
        let result = generate_placeholder("", None, &config);
        // Should not panic, should return default
        assert!(!result.is_empty());
    }

    #[test]
    fn test_generate_placeholder_special_chars_in_key() {
        let config = PlaceholderConfig::default();
        // Use a key that won't match any built-in rules
        // Avoid: KEY, SECRET, TOKEN, PASSWORD, URL, PORT, DEBUG, HOST, SERVER
        let result = generate_placeholder("my-custom-var", None, &config);
        assert_eq!(result, "YOUR_VALUE_HERE");
    }

    #[test]
    fn test_generate_placeholder_priority_order() {
        let mut config = PlaceholderConfig::default();
        // Add a pattern that could conflict with built-in rules
        config
            .patterns
            .insert(".*PASSWORD".to_string(), "custom-pass".to_string());

        // Custom pattern should take priority over built-in "PASSWORD" rule
        assert_eq!(
            generate_placeholder("MY_PASSWORD", None, &config),
            "custom-pass"
        );
    }

    /// T-SYNC-26 (I-120, DEC-S10): a rule word inside another word is not
    /// the word.
    #[test]
    fn t_i120_look_alike_names_get_the_default() {
        let config = PlaceholderConfig::default();
        for key in [
            "SUPPORT_EMAIL",
            "MONKEY_COUNT",
            "BYPASS_CACHE",
            "GHOST_MODE",
            "IMPORT_PATH",
            "PASSENGER_COUNT",
            "TOKENIZER_MODEL",
        ] {
            assert_eq!(
                generate_placeholder(key, None, &config),
                "YOUR_VALUE_HERE",
                "{key}"
            );
        }
        // Whole words still match, in any case and any position.
        assert_eq!(
            generate_placeholder("stripe_key", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("SENDGRID_APIKEY", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("ADMIN_PWD", None, &config),
            "YOUR_PASSWORD_HERE"
        );
        assert_eq!(
            generate_placeholder("SMTP_SERVER", None, &config),
            "localhost"
        );
    }

    /// T-SYNC-27: a switch keeps its value; anything else under DEBUG falls
    /// back as before.
    #[test]
    fn t_i120_debug_keeps_a_switch_value() {
        let config = PlaceholderConfig::default();
        let v = |s: &str| s.to_string();
        assert_eq!(
            generate_placeholder("DEBUG", Some(&v("false")), &config),
            "false"
        );
        assert_eq!(
            generate_placeholder("APP_DEBUG", Some(&v(" Off ")), &config),
            "Off"
        );
        assert_eq!(
            generate_placeholder("DEBUG", Some(&v("verbose")), &config),
            "true"
        );
        assert_eq!(generate_placeholder("DEBUG", None, &config), "true");
    }

    /// T-SYNC-28: the placeholder follows the scheme, never the credential.
    #[test]
    fn t_i120_url_placeholder_by_scheme() {
        let config = PlaceholderConfig::default();
        for (value, scheme) in [
            ("postgres://admin:s3cr3t@db/app", "postgres://"),
            ("postgresql://admin:s3cr3t@db/app", "postgresql://"),
            ("mysql://root:s3cr3t@db/app", "mysql://"),
            ("mongodb://u:s3cr3t@h/db", "mongodb://"),
            (
                "mongodb+srv://u:s3cr3t@cluster0.mongodb.net/db",
                "mongodb+srv://",
            ),
            ("amqp://guest:s3cr3t@rabbit:5672/", "amqp://"),
            ("rediss://:s3cr3t@cache:6380/0", "rediss://"),
            ("REDIS://cache:6379", "redis://"),
            ("https://api.example.com/v1", "https://"),
        ] {
            let got = generate_placeholder("DB_URL", Some(&value.to_string()), &config);
            assert!(got.starts_with(scheme), "{value} -> {got}");
            assert!(!got.contains("s3cr3t"), "{value} -> {got}");
            assert!(is_generated_value(&got, &config), "{got}");
        }
        let got = generate_placeholder("DB_URL", Some(&"ftp://x".to_string()), &config);
        assert_eq!(got, "YOUR_URL_HERE");
        // URI and DSN are URL words too.
        let got = generate_placeholder("MONGO_URI", Some(&"mongodb://h/db".to_string()), &config);
        assert!(got.starts_with("mongodb://"), "{got}");
    }

    /// I-121, DEC-S11: every key the struct reads is listed, so the refusal
    /// cannot reject a real setting.
    #[test]
    fn t_i121_config_keys_match_the_fields() {
        let json = serde_json::to_value(PlaceholderConfig {
            schema: Some("s".into()),
            description: Some("d".into()),
            naming_convention: Some(serde_json::json!("x")),
            ..Default::default()
        })
        .unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut listed = CONFIG_KEYS.to_vec();
        listed.sort_unstable();
        assert_eq!(keys, listed);
    }
}
