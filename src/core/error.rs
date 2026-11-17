//! One error type for the exit contract, and the one place that turns a result
//! into a code (WP-9).
//!
//! Commands keep returning `anyhow::Result`. `main` converts what comes back
//! with [`classify`] (`From<anyhow::Error>`), which walks the chain for the typed
//! errors that carry a meaning of their own — a prompt answered no, a
//! confirmation nobody could give, a cloud failure, a restore written aside —
//! and [`report`] prints it and returns the code. Nothing else in `src/` maps an
//! error to a code, and nothing else calls `process::exit` (but `utils::term`,
//! broken pipe, and `core::inject::exit_as`, the child's code).
//!
//! | Code | Variants |
//! |---|---|
//! | 2 | `Usage`, `Config`, `Io`, `Stopped`, `Said`, `Footed`, `Backup`, `Restore`, `Other` |
//! | 4 | `Declined`, `NeedsConfirmation` |
//! | 5 | `RestoredAside` |
//! | 10-15 | `Cloud`, `Platform`, `SignedOut` (10) |

use std::path::PathBuf;
use std::process::ExitCode;

use colored::Colorize;

use super::backup_error::BackupError;
use super::restore_error::RestoreError;
use super::verdict::{
    self, Footed, NeedsConfirmation, Said, Stage, Stopped, Verdict, EXIT_ASIDE, EXIT_DECLINED,
    EXIT_ERROR,
};

/// Why a command did not reach a verdict, or reached "no" without a finding.
#[derive(Debug, thiserror::Error)]
pub enum EvnxError {
    /// The command line asked for something that cannot be done ([`verdict::usage`]):
    /// a value that is not valid, a file named on it that is not there, flags
    /// that do not go together. The message stands alone, with no footer.
    #[error("{0:#}")]
    Usage(anyhow::Error),

    /// A setting could not be used: `.evnx.toml`, `--env-name`, a named file.
    /// Before the run.
    #[error("{0:#}")]
    Config(anyhow::Error),

    /// The working directory (or a path) could not be read. Before the run.
    #[error("reading {path}: {source}", path = path.display())]
    Io {
        /// What was being read.
        path: PathBuf,
        /// The operating system's answer.
        source: std::io::Error,
    },

    /// A refusal before anything was changed, whose message says what to run
    /// instead ([`verdict::stopped`]). The error is kept whole so the report
    /// prints its full chain.
    #[error("{0:#}")]
    Stopped(anyhow::Error),

    /// A prompt was answered no. Already said: nothing more is printed.
    #[error("cancelled")]
    Declined,

    /// A confirmation was needed, and there was no terminal and no `--yes`
    /// (D3). The message names `--yes`.
    #[error("{0:#}")]
    NeedsConfirmation(anyhow::Error),

    /// `restore` wrote the result next to the output, not in it. Already said.
    #[error("written aside to {}", .0.display())]
    RestoredAside(PathBuf),

    /// The command printed its own explanation: nothing more is printed.
    #[error("already reported")]
    Said,

    /// `status --check` with nobody signed in: exit 10, nothing more printed.
    #[error("not signed in")]
    SignedOut,

    /// An error with its own footer ([`verdict::footed`]).
    #[error("{0:#}")]
    Footed(anyhow::Error),

    /// A cloud request failed (10-15). The error is kept whole: its text is the
    /// chain, and the code comes from `ApiError::exit_code`.
    #[cfg(feature = "cloud")]
    #[error("{error:#}")]
    Cloud {
        /// The failure, with its context.
        error: anyhow::Error,
        /// 10-15.
        code: u8,
    },

    /// A platform request failed (10-15), from `PlatformError::exit_code`.
    #[cfg(feature = "net")]
    #[error("{error:#}")]
    Platform {
        /// The failure, with its context.
        error: anyhow::Error,
        /// 10-15.
        code: u8,
    },

    /// `backup` failed.
    #[error(transparent)]
    Backup(BackupError),

    /// `restore` failed.
    #[error(transparent)]
    Restore(RestoreError),

    /// Anything else: could not run, exit 2.
    #[error("{0:#}")]
    Other(anyhow::Error),
}

impl From<anyhow::Error> for EvnxError {
    fn from(e: anyhow::Error) -> Self {
        classify(e)
    }
}

impl EvnxError {
    /// The process exit code.
    pub fn exit_code(&self) -> u8 {
        match self {
            EvnxError::Declined | EvnxError::NeedsConfirmation(_) => EXIT_DECLINED,
            EvnxError::RestoredAside(_) => EXIT_ASIDE,
            EvnxError::SignedOut => 10,
            #[cfg(feature = "cloud")]
            EvnxError::Cloud { code, .. } => *code,
            #[cfg(feature = "net")]
            EvnxError::Platform { code, .. } => *code,
            EvnxError::Backup(be) => be.exit_code() as u8,
            EvnxError::Restore(re) => re.exit_code() as u8,
            _ => EXIT_ERROR,
        }
    }

    /// A stable machine id for this kind of failure (snake_case).
    pub fn code(&self) -> &'static str {
        match self {
            EvnxError::Usage(_) => "usage",
            EvnxError::Config(_) => "config",
            EvnxError::Io { .. } => "io",
            EvnxError::Stopped(_) => "stopped",
            EvnxError::Declined => "declined",
            EvnxError::NeedsConfirmation(_) => "needs_confirmation",
            EvnxError::RestoredAside(_) => "restored_aside",
            EvnxError::Said => "reported",
            EvnxError::SignedOut => "not_signed_in",
            EvnxError::Footed(_) => "no_verdict",
            #[cfg(feature = "cloud")]
            EvnxError::Cloud { code, .. } => request_code(*code),
            #[cfg(feature = "net")]
            EvnxError::Platform { code, .. } => request_code(*code),
            EvnxError::Backup(_) => "backup_failed",
            EvnxError::Restore(_) => "restore_failed",
            EvnxError::Other(_) => "error",
        }
    }

    /// Whether the command already said everything: nothing is printed.
    pub fn is_silent(&self) -> bool {
        match self {
            EvnxError::Declined
            | EvnxError::RestoredAside(_)
            | EvnxError::Said
            | EvnxError::SignedOut => true,
            EvnxError::Restore(re) => re.is_silent(),
            _ => false,
        }
    }

    /// Whether it failed before the command started: the footer's wording.
    fn stage(&self) -> Stage {
        match self {
            EvnxError::Usage(_) | EvnxError::Config(_) | EvnxError::Io { .. } => Stage::BeforeRun,
            _ => Stage::DuringRun,
        }
    }

    /// The error whose chain the report prints, for the variants that keep one.
    fn chain(&self) -> Option<&anyhow::Error> {
        match self {
            EvnxError::Config(e)
            | EvnxError::Usage(e)
            | EvnxError::Stopped(e)
            | EvnxError::NeedsConfirmation(e)
            | EvnxError::Footed(e)
            | EvnxError::Other(e) => Some(e),
            #[cfg(feature = "cloud")]
            EvnxError::Cloud { error, .. } => Some(error),
            #[cfg(feature = "net")]
            EvnxError::Platform { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// The stable id for a cloud or platform request failure's code.
#[cfg(feature = "net")]
fn request_code(code: u8) -> &'static str {
    match code {
        10 => "not_signed_in",
        11 => "forbidden",
        12 => "plan_limit",
        13 => "unreachable",
        14 => "conflict",
        15 => "server_error",
        _ => "request_failed",
    }
}

/// Find the typed meaning of an error anywhere in its chain.
pub fn classify(e: anyhow::Error) -> EvnxError {
    if e.chain().any(|c| c.is::<crate::utils::prompt::Declined>()) {
        return EvnxError::Declined;
    }
    {
        if e.downcast_ref::<RestoreError>().is_some() {
            if let Ok(re) = e.downcast::<RestoreError>() {
                return match re {
                    RestoreError::Cancelled => EvnxError::Declined,
                    RestoreError::ValidationFallback { fallback_path } => {
                        EvnxError::RestoredAside(PathBuf::from(fallback_path))
                    }
                    re => EvnxError::Restore(re),
                };
            }
            unreachable!("downcast_ref said it was a RestoreError");
        }
        if e.downcast_ref::<BackupError>().is_some() {
            if let Ok(be) = e.downcast::<BackupError>() {
                return EvnxError::Backup(be);
            }
            unreachable!("downcast_ref said it was a BackupError");
        }
    }
    if e.chain().any(|c| c.is::<verdict::SignedOut>()) {
        return EvnxError::SignedOut;
    }
    if e.chain().any(|c| c.is::<Said>()) {
        return EvnxError::Said;
    }
    if e.chain().any(|c| c.is::<Footed>()) {
        return EvnxError::Footed(e);
    }
    if e.chain()
        .any(|c| c.is::<NeedsConfirmation>() || c.is::<crate::utils::prompt::NoTerminal>())
    {
        return EvnxError::NeedsConfirmation(e);
    }
    // A cloud failure that has its own exit code (X3: 10-15) keeps the same
    // text and "no verdict" line as any other error; only the code differs.
    #[cfg(feature = "cloud")]
    if let Some(code) = crate::cloud::client::exit_code_of(&e) {
        return EvnxError::Cloud {
            error: e,
            code: code as u8,
        };
    }
    #[cfg(feature = "net")]
    if let Some(code) = crate::platform::exit_code_of(&e) {
        return EvnxError::Platform {
            error: e,
            code: code as u8,
        };
    }
    if e.chain().any(|c| c.is::<verdict::UsageError>()) {
        return EvnxError::Usage(e);
    }
    if e.chain().any(|c| c.is::<Stopped>()) {
        return EvnxError::Stopped(e);
    }
    EvnxError::Other(e)
}

/// The line under an error: what it means for the result.
fn footer(error: &EvnxError) -> String {
    let chain = error.chain();
    let has = |f: fn(&anyhow::Error) -> bool| chain.is_some_and(f);
    if let Some(Footed { footer, .. }) =
        chain.and_then(|e| e.chain().find_map(|c| c.downcast_ref()))
    {
        return footer.clone();
    }
    if has(|e| {
        e.chain().any(|c| {
            c.downcast_ref::<NeedsConfirmation>()
                .is_some_and(|n| n.stopped)
        })
    }) {
        return verdict::STOPPED_FOR_YES_LINE.to_string();
    }
    if has(|e| e.chain().any(|c| c.is::<Stopped>())) {
        return verdict::STOPPED_LINE.to_string();
    }
    if let Some(reason) = reason(error) {
        return format!("No verdict: {reason}. This is not a clean result.");
    }
    verdict::no_verdict_line(error.stage()).to_string()
}

/// Why there is no verdict, when the error says: a request that failed, or a
/// file that could not be read. Anything else gets the general line.
fn reason(error: &EvnxError) -> Option<&'static str> {
    if matches!(error, EvnxError::NeedsConfirmation(_)) {
        return Some("a question needed an answer and there is no terminal to ask on");
    }
    match error.exit_code() {
        10 => return Some("nobody is signed in"),
        11 => return Some("the server refused access"),
        12 => return Some("the plan's limit was reached"),
        13 => return Some("the server could not be reached"),
        14 => return Some("the server holds a newer version"),
        15 => return Some("the server reported an error"),
        _ => {}
    }
    let chain = error.chain()?;
    if chain.chain().any(|c| c.is::<verdict::MissingFile>()) {
        return Some("a file evnx needed was not found");
    }
    let io = chain
        .chain()
        .find_map(|c| c.downcast_ref::<std::io::Error>())?;
    Some(match io.kind() {
        std::io::ErrorKind::NotFound => "a file evnx needed was not found",
        std::io::ErrorKind::PermissionDenied => {
            "a file could not be read or written (permission denied)"
        }
        _ => "a file could not be read or written",
    })
}

/// Print an error as `main` always has: `Error: …`, a blank line, then what it
/// means. Silent errors print nothing.
fn print(error: &EvnxError) {
    if error.is_silent() {
        return;
    }
    let label = "Error:".on_red().bold();
    match error {
        // These two never had a footer: their message stands alone.
        EvnxError::Backup(_) | EvnxError::Restore(_) => {
            eprintln!("{label} {error}");
        }
        // A usage error is not a run that failed: the message is all there is.
        // Whichever way it got here (`Config` carries the main-side checks).
        _ if error
            .chain()
            .is_some_and(|e| e.chain().any(|c| c.is::<verdict::UsageError>())) =>
        {
            eprintln!("{label} {error}");
        }
        EvnxError::Footed(e) => {
            let message = e
                .chain()
                .find_map(|c| c.downcast_ref::<Footed>())
                .and_then(|f| f.message.as_deref());
            if let Some(message) = message {
                eprintln!("{label} {message}");
            }
            eprintln!();
            eprintln!("{}", footer(error));
        }
        _ => {
            eprintln!("{label} {error}");
            eprintln!();
            eprintln!("{}", footer(error));
        }
    }
}

/// The one place a result becomes an exit code: print what is to be said, and
/// return the code. `main` returns it, so every destructor runs.
pub fn report(result: Result<Verdict, EvnxError>) -> ExitCode {
    match result {
        Ok(verdict) => ExitCode::from(verdict.code()),
        Err(error) => {
            print(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(e: anyhow::Error) -> u8 {
        classify(e).exit_code()
    }

    #[test]
    fn plain_errors_could_not_run() {
        let e = classify(anyhow::anyhow!("boom"));
        assert_eq!(e.exit_code(), 2);
        assert_eq!(e.code(), "error");
        assert!(!e.is_silent());
    }

    #[test]
    fn a_no_is_declined_and_silent() {
        let e = classify(crate::utils::prompt::Declined.into());
        assert_eq!(e.exit_code(), 4);
        assert_eq!(e.code(), "declined");
        assert!(e.is_silent());
        let wrapped = anyhow::Error::from(crate::utils::prompt::Declined).context("outer");
        assert_eq!(code_of(wrapped), 4);
    }

    #[test]
    fn a_confirmation_nobody_can_give_is_declined_and_said() {
        for e in [
            verdict::needs_yes("add --yes"),
            verdict::stopped_needs_yes("add --yes"),
            crate::utils::prompt::no_terminal("evnx x --yes"),
        ] {
            let e = classify(e);
            assert_eq!(e.exit_code(), 4);
            assert!(!e.is_silent());
            assert_eq!(e.code(), "needs_confirmation");
        }
    }

    #[test]
    fn stopped_stays_two() {
        let e = classify(verdict::stopped("run this instead"));
        assert_eq!(e.exit_code(), 2);
        assert_eq!(e.code(), "stopped");
        assert_eq!(footer(&e), verdict::STOPPED_LINE);
        let e = classify(verdict::stopped_needs_yes("run this instead"));
        assert_eq!(footer(&e), verdict::STOPPED_FOR_YES_LINE);
    }

    #[test]
    fn footers_follow_the_stage() {
        let before = EvnxError::Usage(anyhow::anyhow!("x"));
        assert_eq!(before.exit_code(), 2);
        assert_eq!(footer(&before), verdict::no_verdict_line(Stage::BeforeRun));
        assert!(classify(verdict::usage("bad value")).code() == "usage");
        let during = classify(anyhow::anyhow!("x"));
        assert_eq!(footer(&during), verdict::no_verdict_line(Stage::DuringRun));
    }

    #[test]
    fn io_names_what_was_read() {
        let e = EvnxError::Io {
            path: PathBuf::from("the current directory"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().starts_with("reading the current directory: "));
    }

    #[test]
    fn said_is_silent_and_two() {
        let e = classify(anyhow::Error::new(Said));
        assert!(e.is_silent());
        assert_eq!(e.exit_code(), 2);
    }

    #[test]
    fn footed_keeps_its_own_footer() {
        let e = classify(verdict::footed(Some("m".into()), "its footer"));
        assert_eq!(e.exit_code(), 2);
        assert_eq!(footer(&e), "its footer");
    }

    #[test]
    fn backup_and_restore_map_onto_the_table() {
        assert_eq!(code_of(BackupError::PasswordMismatch.into()), 2);
        assert_eq!(code_of(BackupError::VerifyFailed("x".into()).into()), 2);
        assert_eq!(code_of(RestoreError::FileNotFound("x".into()).into()), 2);
        assert_eq!(code_of(RestoreError::WrongPassword.into()), 2);
        assert_eq!(code_of(RestoreError::Cancelled.into()), 4);
        let aside = classify(
            RestoreError::ValidationFallback {
                fallback_path: ".env.restored".into(),
            }
            .into(),
        );
        assert_eq!(aside.exit_code(), 5);
        assert!(aside.is_silent());
    }
}
