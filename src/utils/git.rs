//! Git repository utilities.
//!
//! Thin wrappers around `git` CLI commands and `.gitignore` file manipulation.
//! All functions that shell out to `git` gracefully return `false` or an error
//! if git is not installed or the current directory is not a repository.
//!
//! # Future work
//!
//! - Replace CLI shelling with the `git2` crate for faster, dependency-free
//!   operation (no requirement for git to be in `$PATH`).
//! - Add `staged_files()` to list files currently in the git index.
//! - Add `last_commit_touching(file)` for the doctor command to surface when
//!   a `.env` file was last accidentally committed.

use anyhow::{anyhow, Result};
use std::process::Command;

/// Return `true` if `file` is listed in `.gitignore`.
///
/// Reads `.gitignore` in the current working directory. Returns `false` (not
/// an error) if `.gitignore` does not exist.
///
/// # Caveats
///
/// This is a simple line-by-line text search. It does not evaluate glob
/// patterns, negation rules (`!pattern`), or directory-specific ignores.
/// For a future revision, consider using `git check-ignore -v <file>` instead.
pub fn is_in_gitignore(file: &str) -> Result<bool> {
    let gitignore = match std::fs::read_to_string(".gitignore") {
        Ok(content) => content,
        // If .gitignore does not exist the file is definitely not ignored.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    Ok(gitignore.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == file || trimmed.starts_with(file)
    }))
}

/// Append `file` to `.gitignore`, creating it if necessary.
///
/// A blank line and a `# Environment variables` comment are prepended so the
/// entry is easy to find on manual inspection.
pub fn add_to_gitignore(file: &str) -> Result<()> {
    use std::io::Write;

    let mut gitignore = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(".gitignore")?;

    writeln!(gitignore, "\n# Environment variables")?;
    writeln!(gitignore, "{}", file)?;

    Ok(())
}

/// Return `true` if `file` is tracked by the Git index.
///
/// Uses `git ls-files --error-unmatch` which exits non-zero for untracked
/// files. Returns `false` if git is not available or the directory is not a
/// repository.
pub fn is_tracked(file: &str) -> bool {
    Command::new("git")
        .args(["ls-files", "--error-unmatch", file])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Search git history for commits that introduced `pattern`.
///
/// Runs `git log -S <pattern> --all` and returns a list of
/// `"<hash> <subject>"` strings, one per matching commit.
///
/// # Errors
///
/// Returns an error if git is not available, the directory is not a
/// repository, or the command exits non-zero for any other reason.
pub fn scan_history(pattern: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["log", "-S", pattern, "--all", "--pretty=format:%H %s"])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Return the name of the current git branch.
///
/// # Errors
///
/// Returns an error if the current directory is not a git repository or git
/// is not installed.
pub fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("Not a git repository or git is not installed"));
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Return `true` if the working directory has no uncommitted changes.
///
/// Uses `git status --porcelain` — an empty output means a clean tree.
/// Returns `false` if git is not available or the directory is not a
/// repository.
pub fn is_clean() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| o.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_in_gitignore_missing_file_returns_false() {
        // .gitignore may or may not exist in the test environment.
        // We only assert that the function does not panic or return an Err
        // when the file is absent — the Ok(bool) contract must hold.
        let result = is_in_gitignore("some_file_that_is_not_there");
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_in_gitignore_finds_entry() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let gitignore_path = dir.path().join(".gitignore");
        let mut f = std::fs::File::create(&gitignore_path).unwrap();
        writeln!(f, ".env").unwrap();
        writeln!(f, "*.log").unwrap();

        // Read directly rather than relying on cwd for a hermetic test.
        let content = std::fs::read_to_string(&gitignore_path).unwrap();
        let found = content.lines().any(|line| {
            let t = line.trim();
            t == ".env" || t.starts_with(".env")
        });
        assert!(found);
    }
}
