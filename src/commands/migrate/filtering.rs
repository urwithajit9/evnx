//! filtering.rs — include/exclude/prefix transformations for secret keys
//!
//! Applied in `mod.rs` before handing the `IndexMap` off to any destination,
//! so every destination benefits automatically.

use indexmap::IndexMap;

/// Apply glob-based include/exclude filters and optional key prefix
/// transformations to a set of secrets.
///
/// Processing order:
/// 1. `include` — keep only keys matching at least one glob pattern.
/// 2. `exclude` — drop keys matching any glob pattern.
/// 3. `strip_prefix` — remove a leading prefix from surviving keys.
/// 4. `add_prefix` — prepend a prefix to surviving keys.
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
    let mut result = IndexMap::new();

    for (key, value) in secrets {
        // ── include filter ────────────────────────────────────────────────
        if let Some(patterns) = include {
            if !patterns.iter().any(|p| glob_match(p, key)) {
                continue;
            }
        }

        // ── exclude filter ────────────────────────────────────────────────
        if let Some(patterns) = exclude {
            if patterns.iter().any(|p| glob_match(p, key)) {
                continue;
            }
        }

        // ── key transformation ────────────────────────────────────────────
        let mut new_key = key.clone();

        if let Some(prefix) = strip_prefix {
            if new_key.starts_with(prefix) {
                new_key = new_key[prefix.len()..].to_string();
            }
        }

        if let Some(prefix) = add_prefix {
            new_key = format!("{}{}", prefix, new_key);
        }

        result.insert(new_key, value.clone());
    }

    result
}

/// Minimal glob matching supporting `*` (any sequence of chars) and `?`
/// (any single char). Case-sensitive.
///
/// This avoids a dependency on the `glob` crate for the common `PREFIX_*`
/// pattern used in .env key filtering.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            // Try consuming zero or more characters from text
            if glob_match_inner(&pattern[1..], text) {
                return true;
            }
            if !text.is_empty() {
                return glob_match_inner(pattern, &text[1..]);
            }
            false
        }
        (Some(b'?'), Some(_)) => glob_match_inner(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_match_inner(&pattern[1..], &text[1..]),
        _ => false,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_star() {
        assert!(glob_match("DB_*", "DB_HOST"));
        assert!(glob_match("DB_*", "DB_PASSWORD"));
        assert!(!glob_match("DB_*", "AWS_KEY"));
        assert!(glob_match("*", "ANYTHING"));
    }

    #[test]
    fn test_glob_question_mark() {
        assert!(glob_match("K?Y", "KEY"));
        assert!(glob_match("K?Y", "KAY"));
        assert!(!glob_match("K?Y", "KEEY"));
    }

    #[test]
    fn test_glob_exact() {
        assert!(glob_match("DATABASE_URL", "DATABASE_URL"));
        assert!(!glob_match("DATABASE_URL", "database_url"));
    }

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
}
