//! One renderer for a command's final result (WP-17, RUST-8).
//!
//! A command that has a report builds it, with no printing, then hands it to
//! [`Out::emit`]:
//!
//! ```text
//! let report = build(...)?;            // names, counts, lengths, masked text
//! Out::new(Mode::of(format)).emit(&report)?;
//! ```
//!
//! [`Out::emit`] chooses by mode: text calls [`Report::text`], JSON goes
//! through [`crate::core::json::emit`] (the `run` header), SARIF through
//! [`crate::core::json::emit_unwrapped`], GitHub annotations through
//! [`Report::github`]. `--exit-only` prints nothing.
//!
//! Scope (WP-17 D5): the **final result** only. Prompts, spinners, progress
//! lines and pre-flight notices stay in `ui::*`, because `init`, `add` and
//! `sync` interleave them with questions.
//!
//! ⛔ A report holds names, counts, lengths, `Disclosure`s and text already
//! masked. Never a [`crate::core::secret::Secret`]:
//! `tests/architecture.rs` checks the report files, and the `Report` trait
//! requires `Debug`, so a report printed with `{:?}` shows nothing a renderer
//! could not.
//! ⛔ This file is not on the `expose()` allow-list, and must not be.

use anyhow::Result;
use serde::Serialize;
use std::fmt;

/// How a result is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Human output (`pretty`, `patch`).
    Text,
    /// `--format json` / `--json`.
    Json,
    /// `--format sarif` (scan).
    Sarif,
    /// `--format github`: workflow annotations.
    Github,
}

impl Mode {
    /// The mode for a `--format` variant's name (`"Json"`, `"Sarif"`, ...);
    /// every other variant is text.
    pub fn of(variant: &str) -> Self {
        match variant {
            "Json" => Mode::Json,
            "Sarif" => Mode::Sarif,
            "Github" => Mode::Github,
            _ => Mode::Text,
        }
    }
}

/// A command's final result.
pub trait Report: Serialize + fmt::Debug {
    /// The documented schema number of the JSON (`run.schema_version`).
    const SCHEMA_VERSION: u32 = 1;

    /// `run.ruleset` (scan only).
    fn ruleset(&self) -> Option<&str> {
        None
    }

    /// Human output. `out.quiet` says whether `-q` was given; what `-q` hides
    /// is the command's own choice (decoration yes, findings no).
    fn text(&self, out: &Out) -> Result<()>;

    /// `--format sarif`; `None` where the command has none.
    fn sarif(&self) -> Option<Result<serde_json::Value>> {
        None
    }

    /// `--format github`; `None` where the command has none (text is used).
    fn github(&self) -> Option<Result<()>> {
        None
    }

    /// With `-q`, print no text at all. Findings keep printing, so the default
    /// is `false`.
    fn silent_when_quiet(&self) -> bool {
        false
    }
}

/// Where and how a report is written.
#[derive(Debug, Clone, Copy)]
pub struct Out {
    pub mode: Mode,
    /// `-q`, read from `ui` at construction.
    pub quiet: bool,
    /// `scan --exit-only`: the exit code is the whole answer.
    pub exit_only: bool,
}

impl Out {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            quiet: crate::utils::ui::is_quiet(),
            exit_only: false,
        }
    }

    /// From `--format` / `--json`.
    pub fn from_args<F: crate::cli::args::FormatSet>(
        args: &crate::cli::args::OutputArgs<F>,
    ) -> Self {
        Self::new(args.resolved().mode())
    }

    pub fn exit_only(mut self, on: bool) -> Self {
        self.exit_only = on;
        self
    }

    /// Write `report` in this mode.
    pub fn emit<R: Report>(&self, report: &R) -> Result<()> {
        if self.exit_only {
            return Ok(());
        }
        match self.mode {
            Mode::Json => crate::core::json::emit(report, R::SCHEMA_VERSION, report.ruleset()),
            Mode::Sarif => match report.sarif() {
                Some(doc) => crate::core::json::emit_unwrapped(&doc?),
                None => report.text(self),
            },
            Mode::Github => match report.github() {
                Some(done) => done,
                None => report.text(self),
            },
            Mode::Text => {
                if self.quiet && report.silent_when_quiet() {
                    return Ok(());
                }
                report.text(self)
            }
        }
    }
}

/// Test support for the report canary tests (WP-17): a value planted in a
/// command's input must not appear in what the report says about it.
#[cfg(test)]
pub(crate) mod canary {
    /// 24 random characters, new on every run, never a literal in the repository.
    pub fn value() -> String {
        const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes).expect("OS randomness");
        bytes
            .iter()
            .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
            .collect()
    }

    /// Fail when `text` holds the canary or four characters of either end.
    #[track_caller]
    pub fn assert_absent(canary: &str, what: &str, text: &str) {
        for part in [canary, &canary[..4], &canary[canary.len() - 4..]] {
            assert!(
                !text.contains(part),
                "{what} shows a part of the canary ({} chars)",
                part.len()
            );
        }
    }

    /// `report` as `{:?}`, `{:#?}` and JSON (through the one emitter): none holds the canary.
    #[track_caller]
    pub fn assert_report_clean<R: super::Report>(canary: &str, report: &R) {
        assert_absent(canary, "Debug", &format!("{report:?}"));
        assert_absent(canary, "pretty Debug", &format!("{report:#?}"));
        let (res, docs) = crate::core::json::capture(|| {
            super::Out {
                mode: super::Mode::Json,
                quiet: false,
                exit_only: false,
            }
            .emit(report)
        });
        res.unwrap();
        assert_eq!(docs.len(), 1);
        assert_absent(canary, "JSON", &docs[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Debug, Serialize)]
    struct Probe {
        n: u32,
        #[serde(skip)]
        said: Cell<u32>,
    }

    impl Report for Probe {
        const SCHEMA_VERSION: u32 = 7;
        fn text(&self, _out: &Out) -> Result<()> {
            self.said.set(self.said.get() + 1);
            Ok(())
        }
        fn silent_when_quiet(&self) -> bool {
            true
        }
    }

    fn out(mode: Mode, quiet: bool, exit_only: bool) -> Out {
        Out {
            mode,
            quiet,
            exit_only,
        }
    }

    #[test]
    fn mode_names() {
        assert_eq!(Mode::of("Json"), Mode::Json);
        assert_eq!(Mode::of("Sarif"), Mode::Sarif);
        assert_eq!(Mode::of("Github"), Mode::Github);
        assert_eq!(Mode::of("Pretty"), Mode::Text);
        assert_eq!(Mode::of("Patch"), Mode::Text);
    }

    #[test]
    fn text_quiet_and_exit_only() {
        let p = Probe {
            n: 1,
            said: Cell::new(0),
        };
        out(Mode::Text, false, false).emit(&p).unwrap();
        assert_eq!(p.said.get(), 1);
        out(Mode::Text, true, false).emit(&p).unwrap();
        assert_eq!(p.said.get(), 1, "-q silences a report that asks for it");
        out(Mode::Text, false, true).emit(&p).unwrap();
        assert_eq!(p.said.get(), 1, "--exit-only prints nothing");
        // No SARIF or GitHub form: text stands in.
        out(Mode::Sarif, false, false).emit(&p).unwrap();
        out(Mode::Github, false, false).emit(&p).unwrap();
        assert_eq!(p.said.get(), 3);
    }

    #[test]
    fn json_goes_through_the_one_emitter_with_the_run_header() {
        let p = Probe {
            n: 1,
            said: Cell::new(0),
        };
        let (_, docs) =
            crate::core::json::capture(|| out(Mode::Json, true, false).emit(&p).unwrap());
        assert_eq!(docs.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&docs[0]).unwrap();
        assert_eq!(v["run"]["schema_version"], 7);
        assert_eq!(v["n"], 1);
        assert_eq!(p.said.get(), 0);
    }
}
