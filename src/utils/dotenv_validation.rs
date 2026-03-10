//! Shared utilities for .env file validation and parsing.
//!
//! This module provides heuristic checks and helpers used by both
//! `backup` and `restore` commands to ensure consistent behavior.

/// Heuristically check whether `content` resembles a `.env` file.
///
/// A file passes if at least **80%** of its non-empty lines are one of:
/// - blank lines,
/// - comment lines beginning with `#`, or
/// - `KEY=VALUE` assignments where KEY contains only alphanumerics and `_`.
///
/// The 80% threshold is intentionally lenient — its purpose is to detect
/// obviously wrong content (binary blobs, PDFs, prose) rather than to
/// enforce strict `.env` grammar.
pub fn looks_like_dotenv(content: &str) -> bool {
    if content.trim().is_empty() {
        return true;
    }

    let valid_line = |line: &str| -> bool {
        let line = line.trim();
        line.is_empty()
            || line.starts_with('#')
            || line
                .split_once('=')
                .map(|(key, _)| {
                    !key.trim().is_empty()
                        && key.trim().chars().all(|c| c.is_alphanumeric() || c == '_')
                })
                .unwrap_or(false)
    };

    let non_empty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return true;
    }
    let valid_count = non_empty.iter().filter(|&&l| valid_line(l)).count();
    // Integer equivalent of valid_count / total >= 0.8
    valid_count * 10 >= non_empty.len() * 8
}

/// Count the number of `KEY=VALUE` variable assignments in a `.env` string.
///
/// Comments (`#`) and blank lines are excluded from the count.
pub fn count_dotenv_vars(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let l = line.trim();
            !l.is_empty() && !l.starts_with('#') && l.contains('=')
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_dotenv_valid() {
        assert!(looks_like_dotenv("KEY=value\nOTHER=123\n# comment\n"));
    }

    #[test]
    fn test_looks_like_dotenv_empty() {
        assert!(looks_like_dotenv(""));
        assert!(looks_like_dotenv("   \n  "));
    }

    #[test]
    fn test_looks_like_dotenv_rejects_prose() {
        assert!(!looks_like_dotenv(
            "This is a plain text file.\nIt contains no env vars.\nNone at all."
        ));
    }

    #[test]
    fn test_looks_like_dotenv_tolerates_some_noise() {
        // 4 valid lines, 1 invalid → 80% valid → should pass.
        let content = "KEY=value\nOTHER=123\n# comment\nFOO=bar\nnot-a-valid-line\n";
        assert!(looks_like_dotenv(content));
    }

    #[test]
    fn test_count_vars_normal() {
        let content = "# Comment\nKEY=value\nOTHER=123\n\n# Another\nFOO=bar\n";
        assert_eq!(count_dotenv_vars(content), 3);
    }

    #[test]
    fn test_count_vars_empty() {
        assert_eq!(count_dotenv_vars(""), 0);
        assert_eq!(count_dotenv_vars("# Only comments\n\n"), 0);
    }
}
