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
//! ⚠️ There is still a second one, in `core/converter.rs`, and it is not
//! equivalent: it handles `*` only at the start, the end, or both, so
//! `DB_*_URL` falls through to an exact comparison and matches nothing. It also
//! takes its arguments the other way round — `glob_match(text, pattern)` rather
//! than `glob_match(pattern, text)` — which is exactly the kind of pair that
//! gets called wrongly one day. Consolidating it changes what `evnx convert
//! --include` matches, so it wants its own change and its own tests.

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

    /// ⚠️ The case `core/converter.rs`'s matcher gets wrong: a `*` in the middle
    /// falls through to an exact comparison there and matches nothing.
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

    #[test]
    fn neither_filter_admits_everything() {
        assert!(admits("ANYTHING", None, None));
    }
}
