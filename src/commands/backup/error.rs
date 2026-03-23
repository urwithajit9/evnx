//! Typed errors for the `backup` subcommand.
//!
//! Each variant maps to a specific exit code so that shell scripts and CI
//! pipelines can distinguish failure modes without parsing stderr text.
//!
//! # Exit code table
//!
//! | Code | Variant | Meaning |
//! |------|---------|---------|
//! | 0 | — | Success |
//! | 1 | (generic anyhow) | IO, encoding, unexpected failure |
//! | 2 | [`FileNotFound`] / [`NotAFile`] | Source `.env` not found or not a regular file |
//! | 3 | [`PasswordMismatch`] | Confirmation prompt did not match |
//! | 4 | [`EncryptionFailed`] | Crypto failure |
//! | 5 | [`WriteFailed`] | Could not write backup to disk |
//! | 6 | [`VerifyFailed`] | Post-write integrity check failed |
//!
//! [`FileNotFound`]: BackupError::FileNotFound
//! [`NotAFile`]: BackupError::NotAFile
//! [`PasswordMismatch`]: BackupError::PasswordMismatch
//! [`EncryptionFailed`]: BackupError::EncryptionFailed
//! [`WriteFailed`]: BackupError::WriteFailed
//! [`VerifyFailed`]: BackupError::VerifyFailed

use std::fmt;

// ─── Error type ───────────────────────────────────────────────────────────────

/// Structured errors for the `evnx backup` subcommand.
///
/// Returned from [`run`](super::run) (and ultimately from
/// [`backup_inner`](super::core::backup_inner)) as `anyhow::Error` payloads.
/// `main.rs` downcasts to this type to obtain the correct exit code.
///
/// All variants return `is_silent() = false`: the error message is always
/// printed to stderr before the process exits.
#[derive(Debug)]
pub enum BackupError {
    /// The source `.env` path does not exist on disk. Exit code 2.
    FileNotFound(String),

    /// The source path exists but is not a regular file (e.g. a directory).
    /// Exit code 2.
    NotAFile(String),

    /// The password and its confirmation prompt did not match. Exit code 3.
    PasswordMismatch,

    /// AES-256-GCM encryption or Argon2id key derivation failed. Exit code 4.
    EncryptionFailed(String),

    /// The encrypted backup could not be written to disk. Exit code 5.
    WriteFailed(String),

    /// Post-write integrity check failed: the backup file could not be
    /// re-decrypted, or the recovered content did not match the original.
    ///
    /// The backup file is left on disk so the user can inspect it. Exit code 6.
    VerifyFailed(String),
}

impl BackupError {
    /// The process exit code that `main.rs` should use for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::FileNotFound(_) | Self::NotAFile(_) => 2,
            Self::PasswordMismatch => 3,
            Self::EncryptionFailed(_) => 4,
            Self::WriteFailed(_) => 5,
            Self::VerifyFailed(_) => 6,
        }
    }

    /// Whether `main.rs` should suppress the error message before exiting.
    ///
    /// Always `false` for backup errors — every failure should be surfaced to
    /// the user. Mirrors the [`RestoreError::is_silent`] contract so that
    /// `main.rs` can use the same dispatch pattern for both subcommands.
    ///
    /// [`RestoreError::is_silent`]: crate::commands::restore::error::RestoreError::is_silent
    pub fn is_silent(&self) -> bool {
        false
    }
}

// ─── Trait impls ──────────────────────────────────────────────────────────────

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileNotFound(path) => write!(f, "File not found: {}", path),
            Self::NotAFile(path) => write!(f, "Not a regular file: {}", path),
            Self::PasswordMismatch => write!(f, "Passwords do not match"),
            Self::EncryptionFailed(msg) => write!(f, "Encryption failed: {}", msg),
            Self::WriteFailed(msg) => write!(f, "Failed to write backup file: {}", msg),
            Self::VerifyFailed(msg) => write!(f, "Backup integrity check failed: {}", msg),
        }
    }
}

impl std::error::Error for BackupError {}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Exit codes are stable ─────────────────────────────────────────────────

    #[test]
    fn backup_error_exit_codes_are_stable() {
        assert_eq!(BackupError::FileNotFound("/path".into()).exit_code(), 2);
        assert_eq!(BackupError::NotAFile("/path".into()).exit_code(), 2);
        assert_eq!(BackupError::PasswordMismatch.exit_code(), 3);
        assert_eq!(BackupError::EncryptionFailed("x".into()).exit_code(), 4);
        assert_eq!(BackupError::WriteFailed("x".into()).exit_code(), 5);
        assert_eq!(BackupError::VerifyFailed("x".into()).exit_code(), 6);
    }

    // ── is_silent is always false ─────────────────────────────────────────────

    #[test]
    fn backup_error_is_never_silent() {
        assert!(!BackupError::FileNotFound("/path".into()).is_silent());
        assert!(!BackupError::NotAFile("/path".into()).is_silent());
        assert!(!BackupError::PasswordMismatch.is_silent());
        assert!(!BackupError::EncryptionFailed("x".into()).is_silent());
        assert!(!BackupError::WriteFailed("x".into()).is_silent());
        assert!(!BackupError::VerifyFailed("x".into()).is_silent());
    }

    // ── Display messages include the path / detail ────────────────────────────

    #[test]
    fn backup_error_display_includes_path() {
        let msg = BackupError::FileNotFound("/my/.env".into()).to_string();
        assert!(msg.contains("/my/.env"), "Display must include the path");

        let msg = BackupError::NotAFile("/my/dir".into()).to_string();
        assert!(msg.contains("/my/dir"), "Display must include the path");

        let msg = BackupError::EncryptionFailed("argon2 OOM".into()).to_string();
        assert!(
            msg.contains("argon2 OOM"),
            "Display must include the detail"
        );

        let msg = BackupError::WriteFailed("permission denied".into()).to_string();
        assert!(
            msg.contains("permission denied"),
            "Display must include the detail"
        );

        let msg = BackupError::VerifyFailed("content mismatch".into()).to_string();
        assert!(
            msg.contains("content mismatch"),
            "Display must include the detail"
        );
    }

    #[test]
    fn backup_error_password_mismatch_display() {
        let msg = BackupError::PasswordMismatch.to_string();
        assert!(!msg.is_empty(), "Display must produce a non-empty message");
        // The message is generic user-facing feedback; no credential values
        // (the actual passwords typed) should ever appear in the output.
        assert!(
            !msg.contains("secret"),
            "Display must not expose credential values"
        );
    }

    // ── Implements std::error::Error ─────────────────────────────────────────

    #[test]
    fn backup_error_is_std_error() {
        let e: &dyn std::error::Error = &BackupError::PasswordMismatch;
        assert!(!e.to_string().is_empty());
    }

    // ── VerifyFailed exit code is distinct from all others ────────────────────

    #[test]
    fn verify_failed_exit_code_is_6() {
        // Ensures no accidental collision if new variants are added between
        // WriteFailed (5) and VerifyFailed (6).
        let code = BackupError::VerifyFailed("sha mismatch".into()).exit_code();
        assert_eq!(code, 6);
        assert_ne!(code, BackupError::WriteFailed("x".into()).exit_code());
    }
}
