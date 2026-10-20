//! filtering.rs — include/exclude/prefix transformations for secret keys
//!
//! ⚠️ `migrate` no longer calls this: it runs `core::converter`'s pipeline,
//! which `convert` uses too and which refuses two names that become one
//! (A-12, I-104). This was the third copy of filter → strip → add, and the
//! only one with no collision check. It stays as a library function, now a
//! thin wrapper over that pipeline, for callers that want the old map.

use indexmap::IndexMap;

use crate::core::converter::{ConvertOptions, Naming};

/// Names as they are: filtering and renaming only, before a destination is
/// chosen to fold anything.
pub(crate) struct AsIs;

impl Naming for AsIs {
    fn name(&self) -> &str {
        "migrate"
    }
}

/// Apply glob-based include/exclude filters and optional key prefix
/// transformations to a set of secrets.
///
/// Processing order:
/// 1. `include` — keep only keys matching at least one glob pattern.
/// 2. `exclude` — drop keys matching any glob pattern.
/// 3. `strip_prefix` — remove a leading prefix from surviving keys.
/// 4. `add_prefix` — prepend a prefix to surviving keys.
///
/// Two keys that become one keep the **last** value, in the first one's place
/// — what an `IndexMap` insert does. `evnx migrate` refuses that instead,
/// unless `--allow-collisions`.
///
/// # Arguments
///
/// * `secrets`       — original secrets loaded from the source.
/// * `include`       — optional list of glob patterns (e.g. `["DB_*", "AWS_*"]`).
/// * `exclude`       — optional list of glob patterns (e.g. `["*_LOCAL"]`).
/// * `strip_prefix`  — optional prefix to strip from each key.
/// * `add_prefix`    — optional prefix to prepend to each key.
///
/// # Examples
///
/// ```rust
/// use indexmap::IndexMap;
/// use evnx::commands::migrate::filtering::apply_filters;
///
/// let mut secrets = IndexMap::new();
/// secrets.insert("APP_DB_URL".into(), "postgres://example".into());
/// secrets.insert("APP_DEBUG".into(), "true".into());
/// secrets.insert("HOME".into(), "/root".into());
///
/// let filtered = apply_filters(
///     &secrets,
///     Some(&["APP_*".to_string()]),
///     None,
///     Some("APP_"),
///     None,
/// );
///
/// assert!(filtered.contains_key("DB_URL"));
/// assert!(filtered.contains_key("DEBUG"));
/// assert!(!filtered.contains_key("HOME"));
/// ```
pub fn apply_filters(
    secrets: &IndexMap<String, String>,
    include: Option<&[String]>,
    exclude: Option<&[String]>,
    strip_prefix: Option<&str>,
    add_prefix: Option<&str>,
) -> IndexMap<String, String> {
    let options = ConvertOptions {
        include: include.map(<[String]>::to_vec).unwrap_or_default(),
        exclude: exclude.map(<[String]>::to_vec).unwrap_or_default(),
        strip_prefix: strip_prefix.map(str::to_string),
        prefix: add_prefix.map(str::to_string),
        ..Default::default()
    };
    options
        .prepare(secrets, &AsIs)
        .expect("unchecked collisions never refuse")
        .into_iter()
        .collect()
}

// ⚠️ Moved to `core::glob` so `cloud::run` can use the same matcher. A cloud
// command reaching into a migrate command for a string utility is the wrong
// shape, and a copy would have made three implementations in one crate. Its
// tests live there too; the copies that were here went with A-13.
pub use crate::core::glob::glob_match;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_filters_include() {
        let mut s = IndexMap::new();
        s.insert("DB_HOST".into(), "localhost".into());
        s.insert("AWS_KEY".into(), "key".into());
        s.insert("HOME".into(), "/root".into());

        let result = apply_filters(&s, Some(&["DB_*".into()]), None, None, None);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("DB_HOST"));
    }

    #[test]
    fn test_apply_filters_exclude() {
        let mut s = IndexMap::new();
        s.insert("DB_HOST".into(), "localhost".into());
        s.insert("DB_LOCAL".into(), "test".into());

        let result = apply_filters(&s, None, Some(&["*_LOCAL".into()]), None, None);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("DB_HOST"));
    }

    #[test]
    fn test_apply_filters_strip_prefix() {
        let mut s = IndexMap::new();
        s.insert("APP_DB_URL".into(), "postgres://".into());
        s.insert("APP_SECRET".into(), "abc".into());

        let result = apply_filters(&s, None, None, Some("APP_"), None);
        assert!(result.contains_key("DB_URL"));
        assert!(result.contains_key("SECRET"));
        assert!(!result.contains_key("APP_DB_URL"));
    }

    #[test]
    fn test_apply_filters_add_prefix() {
        let mut s = IndexMap::new();
        s.insert("DB_URL".into(), "postgres://".into());

        let result = apply_filters(&s, None, None, None, Some("PROD_"));
        assert!(result.contains_key("PROD_DB_URL"));
    }

    #[test]
    fn test_apply_filters_combined() {
        let mut s = IndexMap::new();
        s.insert("APP_DB_HOST".into(), "localhost".into());
        s.insert("APP_DB_LOCAL".into(), "test".into());
        s.insert("OTHER_VAR".into(), "val".into());

        let result = apply_filters(
            &s,
            Some(&["APP_*".into()]),   // include only APP_
            Some(&["*_LOCAL".into()]), // exclude *_LOCAL
            Some("APP_"),              // strip APP_ prefix
            None,
        );
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("DB_HOST"));
    }

    /// The old map: two names that become one keep the last value.
    #[test]
    fn t_i104_the_wrapper_keeps_the_last_value() {
        let mut s = IndexMap::new();
        s.insert("API_X".into(), "1".into());
        s.insert("X".into(), "2".into());
        let result = apply_filters(&s, None, None, Some("API_"), None);
        assert_eq!(result.len(), 1);
        assert_eq!(result["X"], "2");
    }
}
