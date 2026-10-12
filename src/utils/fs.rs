//! File system utilities for permission management, file inspection, and search.
//!
//! # Platform notes
//!
//! Permission-related functions (`has_secure_permissions`, `set_secure_permissions`)
//! are only meaningful on Unix. On Windows they compile to no-ops that always
//! return `true` / `Ok(())` so the rest of the codebase does not need `#[cfg]`
//! guards at every call site.
//!
//! # Future work
//!
//! - `atomic_write(path, content)`: write to a temp file then `rename` for
//!   crash-safe updates (avoids partially written `.env` files).
//! - `find_env_files(dir)`: specialised search that respects `.gitignore` and
//!   common exclusions (`target/`, `node_modules/`, `.git/`).
//! - `checksum(path)`: return a SHA-256 hex digest for integrity verification.

use anyhow::Result;
use std::fs;
use std::path::Path;

// ── Permissions ───────────────────────────────────────────────────────────────

/// Return `true` if `path` has owner-only permissions (`600`) on Unix.
///
/// A `.env` file should never be world- or group-readable because it contains
/// credentials. Use [`set_secure_permissions`] to fix an insecure file.
///
/// Always returns `true` on non-Unix platforms where the concept does not apply.
#[cfg(unix)]
pub fn has_secure_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|m| (m.permissions().mode() & 0o077) == 0)
        .unwrap_or(false)
}

/// Non-Unix stub — permissions are not applicable on this platform.
#[cfg(not(unix))]
pub fn has_secure_permissions(_path: &Path) -> bool {
    true
}

/// Set `path` to owner-read/write only (`chmod 600`) on Unix.
///
/// # Errors
///
/// Returns an error if the file metadata cannot be read or the permissions
/// cannot be applied (e.g. the caller does not own the file).
#[cfg(unix)]
pub fn set_secure_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

/// Non-Unix stub — no-op, always succeeds.
#[cfg(not(unix))]
pub fn set_secure_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

// ── File operations ───────────────────────────────────────────────────────────

/// Copy `path` to `<path>.backup` in the same directory.
///
/// Used by commands that modify files in-place (e.g. `validate --fix`) so the
/// user always has a recovery copy before any changes are written.
///
/// # Errors
///
/// Returns an error if the source file cannot be read or the backup cannot be
/// written (e.g. insufficient disk space or permissions).
pub fn backup_file(path: &Path) -> Result<()> {
    let backup_path = path.with_extension("backup");
    fs::copy(path, backup_path)?;
    Ok(())
}

/// Return `true` if `path` appears to be a text file (contains no null bytes).
///
/// Null bytes (`\0`) are the simplest heuristic for detecting binary content.
/// This is not foolproof — some binary formats avoid null bytes — but it is
/// sufficient for detecting accidentally passed files in the CLI context.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn is_text_file(path: &Path) -> Result<bool> {
    let content = fs::read(path)?;
    // Use slice::contains (clippy::manual_contains) instead of iter().any(|&b| b == 0)
    Ok(!content.contains(&0u8))
}

/// Format `bytes` as a human-readable size string with two decimal places.
///
/// Uses binary prefixes (1 KB = 1024 bytes).
///
/// # Examples
///
/// ```
/// use evnx::utils::fs::human_readable_size;
/// assert_eq!(human_readable_size(1024), "1.00 KB");
/// assert_eq!(human_readable_size(1_048_576), "1.00 MB");
/// ```
pub fn human_readable_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_idx])
}

// ── Search ────────────────────────────────────────────────────────────────────

/// Return all file paths under the current directory whose path string contains
/// `pattern` as a substring.
///
/// Symbolic links are not followed. Hidden directories and `target/` are not
/// automatically excluded — callers should filter the results if needed.
///
/// # Errors
///
/// Returns an error only if `WalkDir` itself fails to initialise (extremely rare).
/// Individual unreadable entries are silently skipped via `filter_map(|e| e.ok())`.
/// Render a path for **machine-readable output**, with `/` separators on every
/// platform.
///
/// ⛔ **This is a correctness requirement, not cosmetics.** `evnx scan` emits
/// three formats that other programs parse, and a backslash breaks all three:
///
/// * **SARIF** `artifactLocation.uri` is specified as a URI, and a URI uses
///   `/`. GitHub code scanning cannot associate `.\.env` with a file in the
///   repository.
/// * **SARIF** `partialFingerprints` is the *identity* of a finding across
///   runs — it exists so that inserting a line above a secret does not close
///   one alert and open another. With a platform-dependent separator in it,
///   the same secret in the same file has a different identity depending on
///   which runner scanned it, so a matrix build duplicates every alert and a
///   dismissal made on one is lost on the other.
/// * **GitHub Actions annotations** (`::error file=…`) are matched against
///   repository paths, which use `/`. A backslash path annotates nothing.
///
/// ⚠️ **Windows only, and that asymmetry is deliberate.** `\` is a reserved
/// character in a Windows filename, so the replacement is lossless there. On
/// Unix it is a perfectly legal one — a file really can be called `a\b` — and
/// rewriting it would corrupt the path being reported. Unix therefore gets the
/// string unchanged rather than a normalisation that is almost always a no-op
/// and occasionally wrong.
#[cfg(windows)]
pub fn display_portable(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// See the Windows variant for why this is platform-split.
#[cfg(not(windows))]
pub fn display_portable(path: &std::path::Path) -> String {
    path.display().to_string()
}

pub fn find_files(pattern: &str) -> Result<Vec<String>> {
    use walkdir::WalkDir;

    let mut files = Vec::new();

    for entry in WalkDir::new(".")
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            let path_str = path.to_string_lossy();
            if path_str.contains(pattern) {
                files.push(path_str.into_owned());
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {

    /// ⚠️ **On Unix this must be a no-op, including for a file whose name
    /// really contains a backslash.**
    ///
    /// `\` is reserved in a Windows filename, so rewriting it there is
    /// lossless. On Unix it is an ordinary character, and a normalisation that
    /// rewrote it would corrupt the very path being reported — a scanner
    /// naming the wrong file is worse than one naming an awkward file.
    #[test]
    #[cfg(not(windows))]
    fn display_portable_leaves_unix_paths_alone() {
        use std::path::Path;
        assert_eq!(display_portable(Path::new("./.env")), "./.env");
        assert_eq!(display_portable(Path::new("a/b/.env")), "a/b/.env");
        // The case the platform split exists for.
        assert_eq!(
            display_portable(Path::new(r"weird\name.env")),
            r"weird\name.env",
            "a backslash is a legal Unix filename character and must survive"
        );
    }

    /// The Windows half: separators become `/` so SARIF URIs, GitHub
    /// annotations and finding fingerprints are identical across platforms.
    #[test]
    #[cfg(windows)]
    fn display_portable_rewrites_windows_separators() {
        use std::path::Path;
        assert_eq!(display_portable(Path::new(r".\.env")), "./.env");
        assert_eq!(display_portable(Path::new(r"a\b\.env")), "a/b/.env");
        assert_eq!(
            display_portable(Path::new(r"C:\work\app\.env")),
            "C:/work/app/.env"
        );
    }

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_human_readable_size_bytes() {
        assert_eq!(human_readable_size(500), "500.00 B");
    }

    #[test]
    fn test_human_readable_size_kilobytes() {
        assert_eq!(human_readable_size(1024), "1.00 KB");
    }

    #[test]
    fn test_human_readable_size_megabytes() {
        assert_eq!(human_readable_size(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn test_is_text_file_with_text() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"DATABASE_URL=postgresql://localhost\n")
            .unwrap();
        assert!(is_text_file(file.path()).unwrap());
    }

    #[test]
    fn test_is_text_file_with_null_byte() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"hello\x00world").unwrap();
        assert!(!is_text_file(file.path()).unwrap());
    }

    #[test]
    fn test_backup_file_creates_copy() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"SECRET_KEY=abc123").unwrap();

        backup_file(file.path()).unwrap();

        let backup_path = file.path().with_extension("backup");
        assert!(backup_path.exists());

        // Clean up the backup file that persists beyond NamedTempFile scope.
        let _ = fs::remove_file(backup_path);
    }

    #[test]
    #[cfg(unix)]
    fn test_set_and_check_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"KEY=val").unwrap();

        // First make it world-readable so we have something to fix.
        let mut perms = fs::metadata(file.path()).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(file.path(), perms).unwrap();
        assert!(!has_secure_permissions(file.path()));

        set_secure_permissions(file.path()).unwrap();
        assert!(has_secure_permissions(file.path()));
    }
}
