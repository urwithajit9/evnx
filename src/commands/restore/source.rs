//! Reading the backup file.
//!
//! [`fetch`] is the seam between "where the encrypted bytes come from" and the
//! rest of the restore pipeline: [`core`](super::core) only ever receives a
//! `&str` of Base64-encoded ciphertext.
//!
//! ⚠️ **There is no cloud source (DEC-B4).** `cloud://project[/id]` used to be
//! parsed into a `BackupSource::Cloud` whose fetch always failed with exit 6,
//! "evnx-cloud restore is not yet available in this build" — a promise, in
//! every shipped build, of a feature the vault commands already are. A vault
//! holds byte-for-byte `.env` files under the account's keys, not files in
//! this format, so `evnx cloud pull` (one version) and `evnx cloud export` (all
//! of them) are the restore for it. A `cloud://` argument is now a backup file
//! that does not exist, with a hint naming them.

use std::path::Path;

use anyhow::{Context, Result};

use super::error::RestoreError;

/// Read the Base64-encoded ciphertext from the backup file `input`.
///
/// The returned string is what `evnx backup` wrote, passed unchanged to
/// `decrypt_content`.
///
/// # Errors
///
/// - [`RestoreError::FileNotFound`] when the path does not exist or is not a
///   regular file.
/// - [`RestoreError::CloudReference`] when it does not exist and reads as a
///   `cloud://` reference.
/// - IO errors as plain `anyhow` errors.
pub fn fetch(input: &str) -> Result<String> {
    let path = Path::new(input);
    if !path.exists() {
        if input.starts_with("cloud://") {
            return Err(RestoreError::CloudReference(input.to_owned()).into());
        }
        return Err(RestoreError::FileNotFound(input.to_owned()).into());
    }
    if !path.is_file() {
        return Err(RestoreError::FileNotFound(format!("{input} (not a regular file)")).into());
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read backup file: {}", path.display()))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(input: &str) -> RestoreError {
        let err = fetch(input).unwrap_err();
        match err.downcast::<RestoreError>() {
            Ok(e) => e,
            Err(other) => panic!("not a RestoreError: {other}"),
        }
    }

    #[test]
    fn a_missing_file_is_file_not_found() {
        // ⚠️ `env::temp_dir()`, not a hardcoded `/tmp`. On Windows `Path::new("/tmp")`
        // means `\tmp` on the current drive, which does not exist — see the
        // sibling test below, where that difference changed which error arm ran.
        let missing = std::env::temp_dir().join("does-not-exist-evnx-test-source-xq39.backup");
        let e = error_of(missing.to_str().unwrap());
        assert!(matches!(e, RestoreError::FileNotFound(_)), "{e}");
        assert_eq!(e.exit_code(), 3);
    }

    /// ⛔ **This asserts the DIRECTORY arm**, which needs a path that exists and
    /// is not a file. `fetch` has two branches and only the second says
    /// "not a regular file".
    ///
    /// ⚠️ It used a hardcoded `/tmp`. On Windows that is `\tmp` on the current
    /// drive, which does not exist — so the **first** arm ran, the message had
    /// no "not a regular file" in it, and the test failed for a reason that
    /// said nothing about the behaviour being pinned. `env::temp_dir()` is a
    /// directory that exists on every platform.
    #[test]
    fn a_directory_is_file_not_found_with_not_regular_file_message() {
        let dir = std::env::temp_dir();
        assert!(
            dir.is_dir(),
            "temp_dir must exist for this test to mean anything"
        );
        let e = error_of(dir.to_str().unwrap());
        assert!(matches!(e, RestoreError::FileNotFound(_)), "{e}");
        assert!(e.to_string().contains("not a regular file"), "{e}");
    }

    /// DEC-B4: a `cloud://` argument is a missing file, exit 3, and the
    /// message names the commands that restore from a vault.
    #[test]
    fn t_dec_b4_a_cloud_reference_is_a_missing_file_with_a_hint() {
        let e = error_of("cloud://myproject/backup-abc123");
        assert!(matches!(e, RestoreError::CloudReference(_)), "{e}");
        assert_eq!(e.exit_code(), 3);
        let msg = e.to_string();
        assert!(msg.contains("cloud://myproject/backup-abc123"), "{msg}");
        assert!(msg.contains("evnx cloud pull"), "{msg}");
    }
}
