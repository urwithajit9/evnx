//! Shared helpers for the integration tests (P0-2; WP-C9 in the review's plan).
//!
//! # Why this exists
//!
//! Every test file used to grow its own copy of the same few things: a temp
//! project with a `.env` in it, a way to run the binary and read its output, a
//! `normalise()` for golden files, and, in three files, a fake `.git` directory
//! standing in for a repository. The copies drift. A fake `.git` is not a
//! repository: `git check-ignore` and everything that asks git the truth fail
//! against it, so the tests that use one test a different program from the one
//! users run (T-SHARE-32).
//!
//! # Using it
//!
//! ```text
//! mod support;
//! use support::{run, Project};
//!
//! let p = Project::new().example("A=1\n");
//! let out = run(p.path(), &["add", "service", "redis", "--yes"]);
//! out.assert_code(0);
//! ```
//!
//! | Module | What | Review id |
//! |---|---|---|
//! | [`project`] | `Project`: a temp folder with files, modes, symlinks and, on request, a real git repository | T-SYNC-00, T-SHARE-32 |
//! | [`run`] | run the built binary with no terminal and no colour; exit code, stdout and stderr without ANSI | T-SYNC-00 |
//! | [`snapshot`] | normalise temp paths, dates, the version and colour; compare with a golden file or rewrite it under `UPDATE_GOLDEN=1` | T-SHARE-37 |
//! | [`renderers`] | run one scenario through two output formats and assert the same exit code and the same files changed | T-SHARE-33 |
//! | [`pty`] | drive interactive prompts through a real pseudo-terminal (Unix only) | T-ADD-00 |
//! | [`demo`] | run a training demo scene from `tests/demos/` and lock it to a golden file (Linux only) | P0-3, U0 |
//! | [`surface`] | `evnx commands --json` parsed from real stdout: every command path and its arguments | T-SHARE-46 |
//! | [`install`] | the binary placed in a fake install location (`.cargo/bin`, a venv), for `evnx update` | T-SHARE-45 |
//!
//! Each test file is its own crate and uses only part of this module, so
//! `dead_code` and `unused_imports` (for the re-exports below) are allowed here
//! rather than in every file that includes it.
#![allow(dead_code, unused_imports)]

#[cfg(feature = "cloud")]
pub mod cloud;
#[cfg(target_os = "linux")]
pub mod demo;
pub mod install;
pub mod project;
#[cfg(unix)]
pub mod pty;
pub mod renderers;
pub mod run;
pub mod snapshot;
#[cfg(unix)]
pub mod stub;
pub mod surface;

pub use project::Project;
pub use run::{run, run_with, Output};

/// The evnx binary under test, as cargo builds it for integration tests.
pub fn evnx_bin() -> &'static str {
    env!("CARGO_BIN_EXE_evnx")
}
