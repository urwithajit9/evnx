//! Glob matching for variable names.
//!
//! `*` matches any run of characters, `?` matches exactly one, and everything
//! else is literal. Case-sensitive, because environment variable names are.
//!
//! # Why this lives in `core`
//!
//! It was in `commands/migrate/filtering.rs`, which made it awkward to reach
//! from `cloud::run` — a cloud command reaching into a migrate command for a
//! string utility is the wrong shape, and copying it would have made **three**
//! implementations in one crate.
//!
//! `convert` had a second one in `core/converter.rs` until 0.10.9 (A-13,
//! I-090): `*` only at the ends and the arguments the other way round, so
//! `DB_*_URL` matched nothing there and `DB_PRIMARY_URL` here. It uses this one
//! now, and `tests/architecture.rs` keeps it the only one.

/// Does `text` match `pattern`?
///
/// Note the argument order: **pattern first**, like `grep`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            // Consume zero characters, or one and try again.
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

/// Keep the names that `include` admits and `exclude` does not.
///
/// `include` unset means everything is admitted; `exclude` unset removes
/// nothing. Exclude wins, which is the ordering every tool with both uses.
pub fn admits(name: &str, include: Option<&[String]>, exclude: Option<&[String]>) -> bool {
    if let Some(patterns) = include {
        if !patterns.iter().any(|p| glob_match(p, name)) {
            return false;
        }
    }
    if let Some(patterns) = exclude {
        if patterns.iter().any(|p| glob_match(p, name)) {
            return false;
        }
    }
    true
}

/// What to say when `--include`/`--exclude` filtered out every one of
/// `before` variables. `purpose` finishes "left nothing to …".
///
/// ⚠️ One wording for `cloud run` and `convert` (I-089): a filter that matches
/// nothing is almost always a typo, and the two commands refuse it the same way.
pub fn nothing_left(before: usize, purpose: &str) -> String {
    format!(
        "--include/--exclude left nothing to {purpose} — all {before} variable(s) were \
         filtered out.\n\
         \x20 Check the patterns: they are case-sensitive globs matched against \
         variable names."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pats(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stars_match_any_run_including_none() {
        assert!(glob_match("DB_*", "DB_URL"));
        assert!(glob_match("DB_*", "DB_"));
        assert!(glob_match("*_URL", "DB_URL"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("DB_*", "REDIS_URL"));
    }

    /// ⚠️ The case convert's own matcher got wrong until 0.10.9: a `*` in the
    /// middle fell through to an exact comparison and matched nothing.
    #[test]
    fn a_star_in_the_middle_works() {
        assert!(glob_match("DB_*_URL", "DB_PRIMARY_URL"));
        assert!(glob_match("DB_*_URL", "DB__URL"));
        assert!(!glob_match("DB_*_URL", "DB_PRIMARY_HOST"));
    }

    #[test]
    fn question_marks_match_exactly_one() {
        assert!(glob_match("DB_?", "DB_1"));
        assert!(!glob_match("DB_?", "DB_"));
        assert!(!glob_match("DB_?", "DB_12"));
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(!glob_match("db_*", "DB_URL"));
    }

    #[test]
    fn include_admits_only_what_matches() {
        let inc = pats(&["DB_*", "AWS_*"]);
        assert!(admits("DB_URL", Some(&inc), None));
        assert!(admits("AWS_KEY", Some(&inc), None));
        assert!(!admits("STRIPE_KEY", Some(&inc), None));
    }

    #[test]
    fn exclude_removes_what_matches() {
        let exc = pats(&["*_LOCAL"]);
        assert!(admits("DB_URL", None, Some(&exc)));
        assert!(!admits("DB_URL_LOCAL", None, Some(&exc)));
    }

    /// Exclude wins, which is what every tool carrying both does.
    #[test]
    fn exclude_beats_include_on_the_same_name() {
        let inc = pats(&["DB_*"]);
        let exc = pats(&["*_LOCAL"]);
        assert!(admits("DB_URL", Some(&inc), Some(&exc)));
        assert!(!admits("DB_URL_LOCAL", Some(&inc), Some(&exc)));
    }

    /// `cloud run`'s wording, byte for byte as 0.10.8 printed it, now shared
    /// with `convert` (I-089).
    #[test]
    fn nothing_left_keeps_cloud_runs_words() {
        assert_eq!(
            nothing_left(3, "inject"),
            "--include/--exclude left nothing to inject — all 3 variable(s) were filtered out.\n  \
             Check the patterns: they are case-sensitive globs matched against variable names."
        );
    }

    #[test]
    fn neither_filter_admits_everything() {
        assert!(admits("ANYTHING", None, None));
    }
}
