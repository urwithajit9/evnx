//! String utility functions for formatting and display.
//!
//! These helpers are used throughout the CLI for consistent presentation
//! of values in terminal output.

/// Truncate a string to `max_len` characters.
///
/// If the string is longer than `max_len`, it is cut and `...` is appended.
/// `max_len` must be at least 4 — otherwise the ellipsis itself would not fit.
///
/// # Examples
///
/// ```
/// use evnx::utils::string::truncate;
/// assert_eq!(truncate("hello world", 8), "hello...");
/// assert_eq!(truncate("hi", 10), "hi");
/// ```
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Redact a sensitive value for safe display in terminal output.
///
/// - Short values (≤ 8 chars): replaced entirely with `*`.
/// - Longer values: first 4 chars shown, then `...****`.
///
/// The original value is never fully exposed in output; this is purely
/// for debugging context (e.g. showing that a key *has* a value).
///
/// # Examples
///
/// ```
/// use evnx::utils::string::redact;
/// assert_eq!(redact("secret"), "******");
/// assert_eq!(redact("secretkey123"), "secr...****");
/// ```
pub fn redact(s: &str) -> String {
    if s.len() <= 8 {
        "*".repeat(s.len())
    } else {
        format!("{}...{}", &s[..4], "*".repeat(4))
    }
}

/// Return a count-aware string using the correct singular or plural form.
///
/// # Examples
///
/// ```
/// use evnx::utils::string::pluralize;
/// assert_eq!(pluralize(1, "file", "files"), "1 file");
/// assert_eq!(pluralize(3, "file", "files"), "3 files");
/// ```
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{} {}", count, singular)
    } else {
        format!("{} {}", count, plural)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        assert_eq!(truncate("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_exact_boundary() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_redact_short() {
        assert_eq!(redact("secret"), "******");
    }

    #[test]
    fn test_redact_long() {
        assert_eq!(redact("secretkey123"), "secr...****");
    }

    #[test]
    fn test_redact_exactly_eight() {
        // 8 chars → all stars
        assert_eq!(redact("12345678"), "********");
    }

    #[test]
    fn test_pluralize_singular() {
        assert_eq!(pluralize(1, "file", "files"), "1 file");
    }

    #[test]
    fn test_pluralize_plural() {
        assert_eq!(pluralize(2, "file", "files"), "2 files");
    }

    #[test]
    fn test_pluralize_zero() {
        assert_eq!(pluralize(0, "file", "files"), "0 files");
    }
}
