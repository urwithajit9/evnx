//! The exit-code contract: one meaning per code, for every command (C-04, U4).
//!
//! | Code | Constant | Means |
//! |---|---|---|
//! | 0 | [`EXIT_OK`] | ran, nothing to report |
//! | 1 | [`EXIT_FINDINGS`] | ran, and the answer is "no" — secrets found, invalid, different, drifted, uploads failed |
//! | 2 | [`EXIT_ERROR`] | could not run, so **there is no verdict** |
//! | 4 | [`EXIT_DECLINED`] | a prompt answered no, or a confirmation needed with no terminal and no `--yes` |
//! | 5 | [`EXIT_ASIDE`] | `restore` wrote the result aside |
//! | 10-15 | [`EXIT_SIGNED_OUT`]… | cloud and platform request failures |
//!
//! ⚠️ 2 must never be reported as 1. A pipeline branches on the code, and a
//! broken setup that exits 1 reads as a real finding — `sync --naming-policy
//! error` did exactly that under `--check` (I-116). Anything that stops a
//! command from reaching its answer is an `Err`, and `main` alone turns it into
//! a code (`core::error::report`), so the stderr line is the same everywhere.
//! No command calls `process::exit`: the only exceptions are `utils::term`
//! (broken pipe, 141) and `core::inject::exit_as` (the child's code).

/// Ran, nothing to report.
pub const EXIT_OK: u8 = 0;
/// Ran, and the answer is "no".
pub const EXIT_FINDINGS: u8 = 1;
/// Could not run: no verdict.
pub const EXIT_ERROR: u8 = 2;
/// Declined: a prompt answered no, or a confirmation was needed and there was
/// no terminal and no `--yes`.
pub const EXIT_DECLINED: u8 = 4;
/// `restore` wrote the result aside (`<output>.restored`).
pub const EXIT_ASIDE: u8 = 5;
/// Not signed in, or the token was rejected: the first of the cloud and
/// platform request failures (10-15, `ApiError::exit_code`).
pub const EXIT_SIGNED_OUT: u8 = 10;

/// The answer of a command that ran to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to report: exit 0.
    Clean,
    /// Something to report: exit 1.
    Findings,
    /// A prompt was answered no: already said, exit 4.
    Declined,
}

impl Verdict {
    /// The exit code for this verdict.
    pub fn code(self) -> u8 {
        match self {
            Verdict::Clean => EXIT_OK,
            Verdict::Findings => EXIT_FINDINGS,
            Verdict::Declined => EXIT_DECLINED,
        }
    }
}

/// Announce a check that had nothing to check and passed for that reason: one
/// line on stderr, the same shape in every command.
///
/// `what` says why, e.g. `.env not found`. For pre-commit hooks on a fresh
/// clone (`sync --skip-without-env`, `diff --skip-without-env`), where exit 2
/// would block every new contributor's first commit. A skip is said, never
/// silent: a clean exit that checked nothing must not read like a pass.
pub fn skipped(what: &str) {
    eprintln!("· {what} — nothing to check, skipped");
}

/// How far a command got before it failed. Only the wording differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Failed before the command started (config, `--env-name`).
    BeforeRun,
    /// Failed while running.
    DuringRun,
}

/// The line that says there is no verdict.
pub fn no_verdict_line(stage: Stage) -> &'static str {
    match stage {
        Stage::BeforeRun => "No verdict: evnx did not run. This is not a clean result.",
        Stage::DuringRun => "No verdict: evnx did not finish. This is not a clean result.",
    }
}

/// A refusal before anything was changed, whose message already says what to
/// run instead (INIT-7). Its footer says so, rather than "No verdict: evnx
/// did not finish", which a beginner read as a crash. Exit code 2, as before.
#[derive(Debug)]
pub struct Stopped(pub String);

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Stopped {}

/// A [`Stopped`] refusal.
pub fn stopped(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Stopped(message.into()))
}

/// The footer of a [`Stopped`] refusal.
pub const STOPPED_LINE: &str =
    "evnx stopped before making changes (exit 2). Run one of the commands above.";

/// The footer of a refusal that needed a confirmation nobody could give.
pub const STOPPED_FOR_YES_LINE: &str =
    "evnx stopped before making changes (exit 4). Run one of the commands above.";

/// A confirmation was needed and could not be asked (no terminal, no `--yes`):
/// exit 4, as a prompt answered no (D3). `message` names `--yes`.
///
/// `stopped` picks the footer: a refusal that was a [`Stopped`] says so, one that
/// was a plain error keeps the "No verdict" line it always had.
#[derive(Debug)]
pub struct NeedsConfirmation {
    /// What to say, and what to run instead.
    pub message: String,
    /// Whether the footer is the [`Stopped`] one.
    pub stopped: bool,
}

impl std::fmt::Display for NeedsConfirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NeedsConfirmation {}

/// A [`Stopped`]-style refusal because a confirmation was needed: exit 4.
pub fn stopped_needs_yes(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(NeedsConfirmation {
        message: message.into(),
        stopped: true,
    })
}

/// A plain error because a confirmation was needed: exit 4.
pub fn needs_yes(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(NeedsConfirmation {
        message: message.into(),
        stopped: false,
    })
}

/// Already said on stderr, in full: exit 2 with nothing more. For a command that
/// prints its own explanation (`convert`'s list of formats) and has no verdict.
#[derive(Debug)]
pub struct Said;

impl std::fmt::Display for Said {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("already reported")
    }
}

impl std::error::Error for Said {}

/// An error that ends with its own footer instead of the stage's: `scan` and
/// `doctor` say what they did not do ("evnx did not scan anything").
///
/// With `message`, the report is `Error: message`, a blank line and the footer;
/// without, a blank line and the footer.
#[derive(Debug)]
pub struct Footed {
    /// The `Error:` line, when there is one.
    pub message: Option<String>,
    /// The last line.
    pub footer: String,
}

impl std::fmt::Display for Footed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_deref().unwrap_or(&self.footer))
    }
}

impl std::error::Error for Footed {}

/// A [`Footed`] error.
pub fn footed(message: Option<String>, footer: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Footed {
        message,
        footer: footer.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_contract() {
        assert_eq!(
            (EXIT_OK, EXIT_FINDINGS, EXIT_ERROR),
            (0, 1, 2),
            "documented in every command's --help; changing one breaks pipelines"
        );
        assert_eq!(Verdict::Clean.code(), 0);
        assert_eq!(Verdict::Findings.code(), 1);
        assert_eq!(Verdict::Declined.code(), 4);
        assert_eq!((EXIT_DECLINED, EXIT_ASIDE, EXIT_SIGNED_OUT), (4, 5, 10));
    }

    #[test]
    fn both_lines_say_no_verdict() {
        for s in [Stage::BeforeRun, Stage::DuringRun] {
            assert!(no_verdict_line(s).starts_with("No verdict:"));
        }
    }
}
