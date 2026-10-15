//! Shared utilities for .env file validation and parsing.
//!
//! This module provides heuristic checks and helpers used by both
//! `backup` and `restore` commands to ensure consistent behavior.

/// Whether `content` is a `.env` evnx can read.
///
/// The same parser every command reads `.env` with (`core::envfile::is_dotenv`,
/// U5). ⚠️ It was an 80% line heuristic, which rejected `export A=1` and any
/// file whose PEM block made up more than a fifth of its lines — so `restore`
/// set valid backups aside as `.restored` (I-145) — while accepting files the
/// parser cannot read.
pub fn looks_like_dotenv(content: &str) -> bool {
    crate::core::envfile::is_dotenv(content)
}

/// Count the variables in a `.env` string.
///
/// Parsed, so a multi-line value counts once and a base64 line inside it
/// (which may contain `=`) not at all. Content the parser cannot read falls
/// back to counting `KEY=` lines, since a count is still useful when restore
/// sets such a file aside.
pub fn count_dotenv_vars(content: &str) -> usize {
    crate::core::envfile::count_vars(content).unwrap_or_else(|| {
        content
            .lines()
            .filter(|line| {
                let l = line.trim();
                !l.is_empty() && !l.starts_with('#') && l.contains('=')
            })
            .count()
    })
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

    /// ⚠️ Flipped in U5: a line the parser cannot read makes the file "not a
    /// `.env`" (it was tolerated up to 20% of lines).
    #[test]
    fn test_looks_like_dotenv_refuses_an_unreadable_line() {
        let content = "KEY=value\nOTHER=123\n# comment\nFOO=bar\nnot-a-valid-line\n";
        assert!(!looks_like_dotenv(content));
    }

    /// I-145: what the heuristic rejected.
    #[test]
    fn t_i145_export_and_multiline_are_dotenv() {
        assert!(looks_like_dotenv("export A=1\nexport B=2\n"));
        let pem: String = (0..30).map(|_| "QUJDREVGR0g=\n").collect();
        let content = format!("A=1\nK=\"-----BEGIN-----\n{pem}-----END-----\"\n");
        assert!(looks_like_dotenv(&content));
        assert_eq!(
            count_dotenv_vars(&content),
            2,
            "base64 lines are not variables"
        );
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
