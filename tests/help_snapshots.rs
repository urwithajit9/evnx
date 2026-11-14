//! `--help` of `evnx` and of every top-level command, as snapshots.
//!
//! A reworded flag, a new option or a changed default is a change users read,
//! so it should be a diff somebody looks at: each case in `tests/cmd/` has a
//! `.stdout` beside it holding the text. Nested subcommands (`vault share`,
//! `scan rules`, …) are not snapshotted; their parent's help lists them.
//!
//! After a deliberate change, rewrite the snapshots and review the diff:
//!
//! ```text
//! TRYCMD=overwrite cargo test --all-features --test help_snapshots
//! ```
//!
//! # Features
//!
//! Which commands exist, and how some of them read, depends on the features
//! the binary was built with, so the cases are in folders by what they need:
//!
//! - `core/`: in every build, and the same in every build (checked against
//!   the default, `cloud`, `migrate`, `backup` and `--all-features` builds).
//! - `cloud/`: `auth`, `org`, `vault`, `cloud`, `platform`: with `cloud`.
//! - `backup/`: `backup`, `restore`: with `backup`.
//! - `full/`: `evnx --help` and the commands whose help changes with the
//!   combination of features (`scan`, `convert`, `migrate`): with all of
//!   `cloud`, `migrate` and `backup`, which is what `--all-features` is.

// Unix only: clap prints the program's own file name in the `Usage:` line, and on
// Windows that is `evnx.exe`, so the same snapshot cannot match both. The help
// text itself does not depend on the platform.
#![cfg(unix)]

#[test]
fn core_commands() {
    trycmd::TestCases::new().case("tests/cmd/core/*.toml");
}

#[cfg(feature = "cloud")]
#[test]
fn cloud_commands() {
    trycmd::TestCases::new().case("tests/cmd/cloud/*.toml");
}

#[cfg(feature = "backup")]
#[test]
fn backup_commands() {
    trycmd::TestCases::new().case("tests/cmd/backup/*.toml");
}

#[cfg(all(feature = "cloud", feature = "migrate", feature = "backup"))]
#[test]
fn full_build() {
    trycmd::TestCases::new().case("tests/cmd/full/*.toml");
}
