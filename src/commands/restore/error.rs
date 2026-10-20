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
//! | 2    | `WrongPassword`         | Decryption failed — wrong password or tampered |
//! | 2    | `Unreadable`            | Not a backup, truncated, or a newer format (I-149) |
//! | 3    | `FileNotFound`          | Backup file does not exist at the given path   |
//! | 3    | `CloudReference`        | A `cloud://…` argument: not a file (DEC-B4)    |
//! | 4    | `Cancelled`             | User declined the overwrite prompt             |
//! | 5    | `ValidationFallback`    | Restored, but to `.restored` path (bad content)|
//!
//! Every other error — a key file that cannot be read, an output that is the
//! backup itself, no terminal to ask on — is a plain `anyhow` error, which
//! `main` reports as exit **2** with the "No verdict" line, as for every other
//! command (DEC-B9). Exit 1 is never produced.
//!
//! ⚠️ Exit 6 was `CloudNotAvailable`, for a `cloud://` source that was parsed
//! and then refused. It is retired with that source (DEC-B4), and the number
//! stays reserved so a script that branched on it is not handed a new meaning.
//!
//! # Usage in `main.rs`
//!
//! ```rust,ignore
//! use evnx::commands::restore::RestoreError;
//!
//! if let Err(e) = commands::restore::run(backup, options) {
//!     if let Some(re) = e.downcast_ref::<RestoreError>() {
//!         // Cancelled and ValidationFallback already printed their own
//!         // inline messages — do not add a second error line.
//!         if !re.is_silent() {
//!             eprintln!("{} {}", "✗".red(), re);
//!         }
//!         std::process::exit(re.exit_code());
//!     }
//!     // Anything else: exit 2, "No verdict".
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

    /// The backup could not be opened for a reason that is known and is not
    /// the password: not a backup, truncated, or made by a newer evnx (I-149).
    /// The message says which.
    Unreadable(
        /// What is wrong, from [`DecryptError`](crate::commands::backup::DecryptError).
        String,
    ),

    /// The backup path does not exist on the local filesystem.
    FileNotFound(
        /// Human-readable path string, included in the error message.
        String,
    ),

    /// The argument is a `cloud://` reference and no such file exists.
    ///
    /// `restore` reads files made by `evnx backup`. It once parsed
    /// `cloud://project[/id]` and refused it with exit 6, "not yet available";
    /// the vault commands do that job, so the message names them (DEC-B4).
    CloudReference(
        /// The argument as given.
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
}

impl RestoreError {
    /// Process exit code for this error variant.
    ///
    /// See the [module-level exit code table](self) for the full mapping.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::WrongPassword | Self::Unreadable(_) => 2,
            Self::FileNotFound(_) | Self::CloudReference(_) => 3,
            Self::Cancelled => 4,
            Self::ValidationFallback { .. } => 5,
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
            Self::WrongPassword => {
                write!(f, "Wrong password or corrupt backup — decryption failed")
            }
            Self::Unreadable(why) => write!(f, "{why}"),
            Self::FileNotFound(path) => write!(f, "Backup file not found: {path}"),
            Self::CloudReference(arg) => write!(
                f,
                "Backup file not found: {arg}\n\n\
                 \x20 restore reads a file made by `evnx backup`; it does not read from \
                 evnx cloud.\n\
                 \x20 For a vault, use:  evnx cloud pull --vault <name>{}",
                if cfg!(feature = "cloud") {
                    ""
                } else {
                    "\n  (the cloud commands need a build with the `cloud` feature)"
                }
            ),
            Self::Cancelled => write!(f, "Restore cancelled — no files were modified"),
            Self::ValidationFallback { fallback_path } => write!(
                f,
                "Restored to fallback path {fallback_path} (content did not pass .env validation)"
            ),
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
        assert_eq!(RestoreError::FileNotFound("f".into()).exit_code(), 3);
        assert_eq!(RestoreError::Cancelled.exit_code(), 4);
        assert_eq!(
            RestoreError::ValidationFallback {
                fallback_path: "f".into()
            }
            .exit_code(),
            5
        );
        assert_eq!(RestoreError::Unreadable("x".into()).exit_code(), 2);
        assert_eq!(
            RestoreError::CloudReference("cloud://p".into()).exit_code(),
            3
        );
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
        assert!(!RestoreError::Unreadable("x".into()).is_silent());
        assert!(!RestoreError::CloudReference("cloud://p".into()).is_silent());
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
        assert!(downcasted.is_some(), "should downcast back to RestoreError");
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
