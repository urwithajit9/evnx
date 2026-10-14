//! Who can read a file: one definition of "private" for every command (C-04, A-29).
//!
//! "Private" means no permission bits for group or others (`mode & 0o077 == 0`).
//! Before this module the rule lived in three places (`utils::fs`, `sync`'s
//! permission check and `doctor`), worded three ways.
//!
//! ⚠️ **Windows:** there is no mode to read or set. [`is_private`] answers
//! `true` and [`make_private`] changes nothing and says so through
//! [`MODES_APPLY`], so no caller prints "0600" on a platform where nothing set
//! it.

use std::path::Path;

use anyhow::Result;

/// Whether this platform has Unix modes. Callers that would print a mode, or
/// promise one, check this first.
pub const MODES_APPLY: bool = cfg!(unix);

/// Mode for a new file that holds secrets (`.env`, rendered templates,
/// `convert -o`, backups).
pub const PRIVATE: u32 = 0o600;

/// Mode for a new file meant to be shared and committed (`.env.example`).
pub const SHARED: u32 = 0o644;

/// The permission bits of `path` (follows symlinks), or `None` when the file
/// cannot be read or the platform has no modes.
pub fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// `true` when no one but the owner can read or write `path`. On Windows,
/// always `true` (see the module note). A file that cannot be read is not
/// private.
pub fn is_private(path: &Path) -> bool {
    if !MODES_APPLY {
        return true;
    }
    mode(path).is_some_and(|m| m & 0o077 == 0)
}

/// Set `path` to [`PRIVATE`]. Returns `false` on Windows, where nothing was
/// changed.
pub fn make_private(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE))
            .with_context(|| format!("could not restrict permissions on {}", path.display()))?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

/// The command that makes `path` private, for a message to a person.
pub fn chmod_hint(path: &Path) -> String {
    format!("chmod 600 {}", path.display())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn file_with_mode(m: u32) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(m)).unwrap();
        (d, p)
    }

    #[test]
    fn private_means_no_group_or_other_bits() {
        for (m, private) in [
            (0o600, true),
            (0o400, true),
            (0o640, false),
            (0o644, false),
            (0o604, false),
        ] {
            let (_d, p) = file_with_mode(m);
            assert_eq!(is_private(&p), private, "{m:o}");
        }
    }

    #[test]
    fn a_missing_file_is_not_private() {
        assert!(!is_private(Path::new("/nonexistent/evnx/f")));
    }

    #[test]
    fn make_private_sets_0600() {
        let (_d, p) = file_with_mode(0o644);
        assert!(make_private(&p).unwrap());
        assert_eq!(mode(&p), Some(0o600));
    }
}
