//! The exit-code contract: one meaning per code, for every command (C-04, U4).
//!
//! | Code | Constant | Means |
//! |---|---|---|
//! | 0 | [`EXIT_OK`] | ran, nothing to report |
//! | 1 | [`EXIT_FINDINGS`] | ran, and the answer is "no" — secrets found, invalid, different, drifted, uploads failed |
//! | 2 | [`EXIT_ERROR`] | could not run, so **there is no verdict** |
//!
//! ⚠️ 2 must never be reported as 1. A pipeline branches on the code, and a
//! broken setup that exits 1 reads as a real finding — `sync --naming-policy
//! error` did exactly that under `--check` (I-116). Anything that stops a
//! command from reaching its answer is an `Err` that `main` turns into 2,
//! through [`no_verdict`], so the stderr line is the same everywhere.
//!
//! Commands with their own documented codes — `backup`, `restore` (their own
//! error enums), `cloud run` (the child's code) — keep them.

use colored::Colorize;

/// Ran, nothing to report.
pub const EXIT_OK: i32 = 0;
/// Ran, and the answer is "no".
pub const EXIT_FINDINGS: i32 = 1;
/// Could not run: no verdict.
pub const EXIT_ERROR: i32 = 2;

/// The answer of a command that ran to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to report: exit 0.
    Clean,
    /// Something to report: exit 1.
    Findings,
}

impl Verdict {
    /// The exit code for this verdict.
    pub fn code(self) -> i32 {
        match self {
            Verdict::Clean => EXIT_OK,
            Verdict::Findings => EXIT_FINDINGS,
        }
    }
}

/// Exit 1 on [`Verdict::Findings`]; return on [`Verdict::Clean`], so `main`
/// finishes normally. For `outcome.map(verdict::finish)`.
pub fn finish(verdict: Verdict) {
    if verdict == Verdict::Findings {
        std::process::exit(EXIT_FINDINGS);
    }
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

/// Print `err` and the no-verdict line to stderr, then exit 2.
pub fn no_verdict(err: &anyhow::Error, stage: Stage) -> ! {
    eprintln!("{} {:#}", "Error:".on_red().bold(), err);
    eprintln!();
    eprintln!("{}", no_verdict_line(stage));
    std::process::exit(EXIT_ERROR);
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
    }

    #[test]
    fn both_lines_say_no_verdict() {
        for s in [Stage::BeforeRun, Stage::DuringRun] {
            assert!(no_verdict_line(s).starts_with("No verdict:"));
        }
    }
}
