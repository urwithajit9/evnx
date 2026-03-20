//! Backup source abstraction — local filesystem or evnx-cloud.
//!
//! [`BackupSource`] is the single seam between "where the encrypted bytes come
//! from" and the rest of the restore pipeline. [`core`](super::core) only ever
//! receives a `&str` of Base64-encoded ciphertext; it never knows whether the
//! bytes came from disk or from the cloud.
//!
//! # Security contract
//!
//! The cloud variant fetches the **encrypted** blob and returns it unchanged.
//! Decryption always happens locally in [`core`](super::core).
//! The evnx-cloud server never receives the password or plaintext content.
//!
//! # Accepted input formats
//!
//! | Input                             | Parsed as                                               |
//! |-----------------------------------|---------------------------------------------------------|
//! | `.env.backup`                     | `Local(".env.backup")`                                  |
//! | `/abs/path/.env.backup`           | `Local("/abs/path/.env.backup")`                        |
//! | `cloud://myproject`               | `Cloud { project: "myproject", backup_id: None }`       |
//! | `cloud://myproject/backup-abc123` | `Cloud { project: "myproject", backup_id: Some("...") }`|

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::error::RestoreError;

// ─── Public type ──────────────────────────────────────────────────────────────

/// Where to read the encrypted backup blob from.
///
/// Constructed by [`BackupSource::parse`] from a CLI argument string and
/// consumed by [`BackupSource::fetch`], which returns the raw
/// Base64-encoded ciphertext for `decrypt_content` in `core.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupSource {
    /// A `.backup` file on the local filesystem.
    Local(PathBuf),

    /// A backup stored in evnx-cloud.
    ///
    /// `backup_id = None` selects the most-recent backup for the project.
    /// A specific ID pins the restore to an exact snapshot.
    ///
    /// **Not yet available** — evnx-cloud integration is planned. Calling
    /// [`BackupSource::fetch`] on this variant returns
    /// [`RestoreError::CloudNotAvailable`] rather than panicking, so existing
    /// code paths are safe to ship.
    Cloud {
        /// evnx-cloud project identifier (slug or UUID).
        project: String,
        /// Specific backup snapshot to restore. `None` = latest.
        backup_id: Option<String>,
    },
}

impl BackupSource {
    /// Parse a user-supplied string into a [`BackupSource`].
    ///
    /// Strings starting with `cloud://` are parsed as cloud references;
    /// everything else is treated as a local filesystem path.
    ///
    /// An empty backup-ID segment (`cloud://proj/`) is normalised to
    /// `None` (latest) rather than `Some("")`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use evnx::commands::restore::source::BackupSource;
    /// # use std::path::PathBuf;
    /// assert_eq!(
    ///     BackupSource::parse(".env.backup"),
    ///     BackupSource::Local(PathBuf::from(".env.backup")),
    /// );
    /// assert_eq!(
    ///     BackupSource::parse("cloud://myproject"),
    ///     BackupSource::Cloud { project: "myproject".into(), backup_id: None },
    /// );
    /// assert_eq!(
    ///     BackupSource::parse("cloud://myproject/backup-abc123"),
    ///     BackupSource::Cloud {
    ///         project: "myproject".into(),
    ///         backup_id: Some("backup-abc123".into()),
    ///     },
    /// );
    /// ```
    pub fn parse(input: &str) -> Self {
        if let Some(rest) = input.strip_prefix("cloud://") {
            let (project, backup_id) = match rest.split_once('/') {
                // "cloud://proj/backup-id" → specific snapshot
                Some((p, id)) if !id.is_empty() => (p.to_owned(), Some(id.to_owned())),
                // "cloud://proj/" → trailing slash, normalise to latest
                Some((p, _)) => (p.to_owned(), None),
                // "cloud://proj" → no slash at all, latest
                None => (rest.to_owned(), None),
            };
            return Self::Cloud { project, backup_id };
        }
        Self::Local(PathBuf::from(input))
    }

    /// Fetch the raw Base64-encoded ciphertext from this source.
    ///
    /// For `Local`, reads the file from disk and validates it is a regular
    /// file (not a directory or symlink to one). For `Cloud`, contacts the
    /// evnx-cloud API (not yet implemented).
    ///
    /// The returned string is the same format produced by `evnx backup` and
    /// is passed directly to `decrypt_content` in `core.rs`.
    ///
    /// # Errors
    ///
    /// - `Local`: [`RestoreError::FileNotFound`] when the path does not exist
    ///   or is not a regular file; IO errors as plain `anyhow` errors.
    /// - `Cloud`: always [`RestoreError::CloudNotAvailable`] until the
    ///   evnx-cloud integration ships.
    pub fn fetch(&self) -> Result<String> {
        match self {
            Self::Local(path) => fetch_local(path),
            Self::Cloud { project, backup_id } => fetch_cloud(project, backup_id.as_deref()),
        }
    }

    /// A human-readable description of this source for log and status messages.
    ///
    /// # Examples
    ///
    /// ```
    /// # use evnx::commands::restore::source::BackupSource;
    /// assert_eq!(
    ///     BackupSource::Local(".env.backup".into()).display_path(),
    ///     ".env.backup",
    /// );
    /// assert_eq!(
    ///     BackupSource::Cloud { project: "prod".into(), backup_id: None }
    ///         .display_path(),
    ///     "cloud://prod (latest)",
    /// );
    /// assert_eq!(
    ///     BackupSource::Cloud {
    ///         project: "prod".into(),
    ///         backup_id: Some("bkp-001".into()),
    ///     }.display_path(),
    ///     "cloud://prod/bkp-001",
    /// );
    /// ```
    pub fn display_path(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Cloud {
                project,
                backup_id: None,
            } => format!("cloud://{project} (latest)"),
            Self::Cloud {
                project,
                backup_id: Some(id),
            } => format!("cloud://{project}/{id}"),
        }
    }
}

// ─── Local ────────────────────────────────────────────────────────────────────

fn fetch_local(path: &Path) -> Result<String> {
    if !path.exists() {
        return Err(RestoreError::FileNotFound(path.display().to_string()).into());
    }
    if !path.is_file() {
        return Err(
            RestoreError::FileNotFound(format!("{} (not a regular file)", path.display())).into(),
        );
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read backup file: {}", path.display()))
}

// ─── Cloud (stub) ─────────────────────────────────────────────────────────────

/// Fetch an encrypted backup blob from evnx-cloud.
///
/// # Planned behaviour (not yet implemented)
///
/// 1. Authenticate using the stored evnx-cloud credential (`evnx auth login`).
/// 2. `GET /v1/projects/{project}/backups/{id}` — or `/latest` when
///    `backup_id` is `None`.
/// 3. Return the raw Base64 ciphertext, **identical in format to a local
///    `.backup` file**, so `decrypt_content` in `core.rs` handles both sources
///    without any branching. The server never receives the password or
///    plaintext content.
///
/// When implemented, this function will be the *only* place that changes to
/// add cloud support — no other restore code needs modification.
fn fetch_cloud(project: &str, backup_id: Option<&str>) -> Result<String> {
    let _ = (project, backup_id);
    Err(RestoreError::CloudNotAvailable.into())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── BackupSource::parse ───────────────────────────────────────────────────

    #[test]
    fn parse_local_relative_path() {
        assert_eq!(
            BackupSource::parse(".env.backup"),
            BackupSource::Local(PathBuf::from(".env.backup")),
        );
    }

    #[test]
    fn parse_local_absolute_path() {
        assert_eq!(
            BackupSource::parse("/home/user/.env.backup"),
            BackupSource::Local(PathBuf::from("/home/user/.env.backup")),
        );
    }

    #[test]
    fn parse_cloud_latest() {
        assert_eq!(
            BackupSource::parse("cloud://myproject"),
            BackupSource::Cloud {
                project: "myproject".into(),
                backup_id: None,
            },
        );
    }

    #[test]
    fn parse_cloud_specific_id() {
        assert_eq!(
            BackupSource::parse("cloud://myproject/backup-abc123"),
            BackupSource::Cloud {
                project: "myproject".into(),
                backup_id: Some("backup-abc123".into()),
            },
        );
    }

    #[test]
    fn parse_cloud_trailing_slash_normalised_to_latest() {
        assert_eq!(
            BackupSource::parse("cloud://proj/"),
            BackupSource::Cloud {
                project: "proj".into(),
                backup_id: None,
            },
        );
    }

    // ── BackupSource::display_path ────────────────────────────────────────────

    #[test]
    fn display_local_path() {
        assert_eq!(
            BackupSource::Local(PathBuf::from(".env.backup")).display_path(),
            ".env.backup",
        );
    }

    #[test]
    fn display_cloud_latest() {
        let src = BackupSource::Cloud {
            project: "prod".into(),
            backup_id: None,
        };
        assert_eq!(src.display_path(), "cloud://prod (latest)");
    }

    #[test]
    fn display_cloud_specific_id() {
        let src = BackupSource::Cloud {
            project: "prod".into(),
            backup_id: Some("bkp-001".into()),
        };
        assert_eq!(src.display_path(), "cloud://prod/bkp-001");
    }

    // ── fetch_local validation ────────────────────────────────────────────────

    #[test]
    fn local_fetch_missing_file_gives_file_not_found_error() {
        let result = fetch_local(Path::new(
            "/tmp/does-not-exist-evnx-test-source-xq39.backup",
        ));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<RestoreError>(),
                Some(RestoreError::FileNotFound(_))
            ),
            "expected RestoreError::FileNotFound"
        );
    }

    #[test]
    fn local_fetch_directory_gives_file_not_found_with_not_regular_file_message() {
        let result = fetch_local(Path::new("/tmp"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        let typed = err.downcast_ref::<RestoreError>();
        assert!(
            matches!(typed, Some(RestoreError::FileNotFound(_))),
            "expected RestoreError::FileNotFound"
        );
        assert!(
            typed.unwrap().to_string().contains("not a regular file"),
            "error message should note it is not a regular file"
        );
    }

    // ── fetch_cloud stub ──────────────────────────────────────────────────────

    #[test]
    fn cloud_fetch_returns_cloud_not_available_error() {
        let result = fetch_cloud("myproject", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<RestoreError>(),
                Some(RestoreError::CloudNotAvailable)
            ),
            "expected RestoreError::CloudNotAvailable"
        );
        assert_eq!(err.downcast_ref::<RestoreError>().unwrap().exit_code(), 6);
    }
}
