//! Typed error variants for the restore command.
//!
//! [`RestoreError`] gives every distinct failure mode a stable exit code so
//! that shell scripts can reliably distinguish "wrong password" from "file not
//! found" from "user said no."
//!
//! # Exit code map
//!
//! | Code | Variant                 | Meaning                                        |
//! |------|-------------------------|------------------------------------------------|
//! | 2    | `WrongPassword`         | Decryption failed — wrong password or corrupt  |
//! | 3    | `FileNotFound`          | Backup file does not exist at the given path   |
//! | 4    | `Cancelled`             | User declined the overwrite prompt             |
//! | 5    | `ValidationFallback`    | Restored, but to `.restored` path (bad content)|
//! | 6    | `CloudNotAvailable`     | evnx-cloud restore not yet implemented         |
//!
//! Exit code `1` is reserved for all other errors (IO failures, corrupt UTF-8,
//! etc.) propagated as plain `anyhow` errors without a typed variant.
//!
//! # Usage in `main.rs`
//!
//! ```rust,ignore
//! use evnx::commands::restore::RestoreError;
//!
//! if let Err(e) = commands::restore::run(backup, output, verbose, dry_run) {
//!     if let Some(re) = e.downcast_ref::<RestoreError>() {
//!         // Cancelled and ValidationFallback already printed their own
//!         // inline messages — do not add a second error line.
//!         if !re.is_silent() {
//!             eprintln!("{} {}", "✗".red(), re);
//!         }
//!         std::process::exit(re.exit_code());
//!     }
//!     // Generic anyhow error — exit 1.
//!     eprintln!("{} {}", "✗".red(), e);
//!     std::process::exit(1);
//! }
//! ```

use std::fmt;

// ─── Error type ───────────────────────────────────────────────────────────────

/// Typed failure modes for the restore pipeline.
///
/// All variants implement [`std::error::Error`] and can be stored in an
/// `anyhow::Error` via `.into()` / `anyhow::Error::from(...)`.  Call
/// [`exit_code`](RestoreError::exit_code) on the downcasted value in `main.rs`
/// to obtain the correct process exit status.
#[derive(Debug)]
pub enum RestoreError {
    /// The supplied password could not decrypt the backup.
    ///
    /// Either the password is incorrect or the backup file is corrupt /
    /// has been tampered with.  The underlying cryptographic error is
    /// intentionally not exposed to avoid leaking implementation details.
    WrongPassword,

    /// The backup path does not exist on the local filesystem.
    FileNotFound(
        /// Human-readable path string, included in the error message.
        String,
    ),

    /// The user declined the overwrite prompt.
    ///
    /// This is a clean "no-op" outcome — no files were modified.  The
    /// inline message was already printed; `main.rs` should exit silently
    /// with this code.
    Cancelled,

    /// The restore succeeded, but the content failed the `.env` heuristic.
    ///
    /// The file was written to the `.restored` fallback path rather than
    /// the requested output path.  All inline messages (path, next steps)
    /// were already printed; `main.rs` should exit silently with this code.
    ValidationFallback {
        /// The path the file was actually written to (e.g. `.env.restored`).
        fallback_path: String,
    },

    /// evnx-cloud restore is not yet implemented in this build.
    CloudNotAvailable,
}

impl RestoreError {
    /// Process exit code for this error variant.
    ///
    /// See the [module-level exit code table](self) for the full mapping.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::WrongPassword => 2,
            Self::FileNotFound(_) => 3,
            Self::Cancelled => 4,
            Self::ValidationFallback { .. } => 5,
            Self::CloudNotAvailable => 6,
        }
    }

    /// Whether `main.rs` should suppress the error message for this variant.
    ///
    /// `true` for variants that already printed a complete inline explanation
    /// (e.g. `Cancelled`, `ValidationFallback`).  `false` for variants where
    /// `main.rs` should print the error before exiting.
    pub fn is_silent(&self) -> bool {
        matches!(self, Self::Cancelled | Self::ValidationFallback { .. })
    }
}

// ─── Trait impls ─────────────────────────────────────────────────────────────

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPassword => write!(
                f,
                "Wrong password or corrupt backup — decryption failed"
            ),
            Self::FileNotFound(path) => write!(f, "Backup file not found: {path}"),
            Self::Cancelled => write!(f, "Restore cancelled — no files were modified"),
            Self::ValidationFallback { fallback_path } => write!(
                f,
                "Restored to fallback path {fallback_path} (content did not pass .env validation)"
            ),
            Self::CloudNotAvailable => {
                write!(f, "evnx-cloud restore is not yet available in this build")
            }
        }
    }
}

impl std::error::Error for RestoreError {}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── exit_code ─────────────────────────────────────────────────────────────

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(RestoreError::WrongPassword.exit_code(), 2);
        assert_eq!(
            RestoreError::FileNotFound("f".into()).exit_code(),
            3
        );
        assert_eq!(RestoreError::Cancelled.exit_code(), 4);
        assert_eq!(
            RestoreError::ValidationFallback {
                fallback_path: "f".into()
            }
            .exit_code(),
            5
        );
        assert_eq!(RestoreError::CloudNotAvailable.exit_code(), 6);
    }

    // ── is_silent ─────────────────────────────────────────────────────────────

    #[test]
    fn cancelled_and_validation_fallback_are_silent() {
        assert!(RestoreError::Cancelled.is_silent());
        assert!(RestoreError::ValidationFallback {
            fallback_path: "x".into()
        }
        .is_silent());
    }

    #[test]
    fn other_variants_are_not_silent() {
        assert!(!RestoreError::WrongPassword.is_silent());
        assert!(!RestoreError::FileNotFound("f".into()).is_silent());
        assert!(!RestoreError::CloudNotAvailable.is_silent());
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn display_includes_path_for_file_not_found() {
        let msg = RestoreError::FileNotFound("/tmp/my.backup".into()).to_string();
        assert!(msg.contains("/tmp/my.backup"));
    }

    #[test]
    fn display_includes_fallback_path() {
        let msg = RestoreError::ValidationFallback {
            fallback_path: ".env.restored".into(),
        }
        .to_string();
        assert!(msg.contains(".env.restored"));
    }

    // ── anyhow interop ────────────────────────────────────────────────────────

    #[test]
    fn can_be_stored_in_anyhow_and_downcasted() {
        let err: anyhow::Error = RestoreError::WrongPassword.into();
        let downcasted = err.downcast_ref::<RestoreError>();
        assert!(
            downcasted.is_some(),
            "should downcast back to RestoreError"
        );
        assert!(matches!(downcasted.unwrap(), RestoreError::WrongPassword));
    }

    #[test]
    fn exit_code_survives_anyhow_round_trip() {
        let err: anyhow::Error = RestoreError::FileNotFound("x".into()).into();
        let code = err
            .downcast_ref::<RestoreError>()
            .map(|e| e.exit_code())
            .unwrap_or(1);
        assert_eq!(code, 3);
    }
}