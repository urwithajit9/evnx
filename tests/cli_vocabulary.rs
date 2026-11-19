//! The flag vocabulary (WP-8): every command spells the same idea the same way.
//!
//! Walks `evnx commands --json` and checks the rules below for every command
//! not on [`ALLOW`]. Since WP-8d the list holds permanent exceptions only, each
//! with a `// permanent: <reason>` comment; a row that no longer fails its rule
//! is itself an error, so the list cannot go stale.
//!
//! Rules:
//! * `a`  every `--format` is a value list with `pretty` and `json`, and no
//!   `text` or `github-actions`
//! * `b`  `--format` and a visible `--json` come together
//! * `c`  a visible `--force` overwrites, or goes ahead "even though" a check
//!   failed
//! * `d1` no visible `--file`, `--path` or `--source-file` (`--key-file` is the
//!   shown name for the passphrase file in `backup` and `restore`, WP-15)
//! * `d2` `--dry-run` has `-n`
//! * `d3` `--yes` has `-y`
//! * `e`  every removed spelling exits 2 with its message (see below)
//!
//! No rule says a command must take `--yes`. `align` never asks a question (it
//! keeps a `.bak` of every file it changes, and `evnx undo` puts them back), so
//! it has none, on purpose.

mod support;

use serde_json::Value;
use support::surface::{Entry, entries};

/// (command path, rule) pairs that are allowed to fail. Every row is a permanent
/// exception and says why (`// permanent: <reason>`); a command that merely has
/// not been converted yet does not belong here. Empty since WP-8d.
const ALLOW: &[(&str, &str)] = &[];

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn visible(e: &Entry) -> impl Iterator<Item = &Value> {
    e.args().iter().filter(|a| a["hidden"] == false)
}

/// What a command does wrong under `rule`; empty when it complies.
fn violations(e: &Entry, rule: &str) -> Vec<String> {
    let mut out = Vec::new();
    let has_format = e.args().iter().any(|a| s(&a["long"]) == "format");
    let has_json = visible(e).any(|a| s(&a["long"]) == "json");
    match rule {
        "a" => {
            for a in e.args().iter().filter(|a| s(&a["long"]) == "format") {
                let vals: Vec<&str> = a["possible_values"]
                    .as_array()
                    .map(|v| v.iter().map(s).collect())
                    .unwrap_or_default();
                if !vals.contains(&"pretty") || !vals.contains(&"json") {
                    out.push(format!("--format values {vals:?} lack pretty or json"));
                }
                for bad in ["text", "github-actions"] {
                    if vals.contains(&bad) {
                        out.push(format!("--format accepts {bad}"));
                    }
                }
            }
        }
        "b" => {
            if has_format && !has_json {
                out.push("--format without a visible --json".into());
            }
            if has_json && !has_format {
                out.push("visible --json without --format".into());
            }
        }
        "c" => {
            for a in visible(e).filter(|a| s(&a["long"]) == "force") {
                let h = s(&a["help"]);
                if !(h.starts_with("Replace")
                    || h.starts_with("Overwrite")
                    || h.contains("even though"))
                {
                    out.push(format!("--force help: {h:?}"));
                }
            }
        }
        "d1" => {
            for a in visible(e) {
                if ["file", "path", "source-file"].contains(&s(&a["long"])) {
                    out.push(format!("visible --{}", s(&a["long"])));
                }
            }
        }
        "d2" => {
            for a in visible(e).filter(|a| s(&a["long"]) == "dry-run") {
                if s(&a["short"]) != "n" {
                    out.push("--dry-run without -n".into());
                }
            }
        }
        "d3" => {
            for a in visible(e).filter(|a| s(&a["long"]) == "yes") {
                if s(&a["short"]) != "y" {
                    out.push("--yes without -y".into());
                }
            }
        }
        other => panic!("unknown rule {other}"),
    }
    out
}

const RULES: &[&str] = &["a", "b", "c", "d1", "d2", "d3"];

fn allowed(path: &str, rule: &str) -> bool {
    ALLOW.iter().any(|(p, r)| *p == path && *r == rule)
}

#[test]
fn every_command_follows_the_vocabulary() {
    let mut failures = Vec::new();
    for e in entries() {
        for rule in RULES {
            if allowed(&e.name(), rule) {
                continue;
            }
            for v in violations(&e, rule) {
                failures.push(format!("evnx {} [{rule}]: {v}", e.name()));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_allow_list_entry_still_fails_its_rule() {
    let all = entries();
    let mut stale = Vec::new();
    for (path, rule) in ALLOW {
        match all.iter().find(|e| e.name() == *path) {
            // A command absent from this build (a feature is off) is not stale.
            None => continue,
            Some(e) if violations(e, rule).is_empty() => {
                stale.push(format!(
                    "evnx {path} [{rule}] now complies: remove it from ALLOW"
                ));
            }
            Some(_) => {}
        }
    }
    assert!(stale.is_empty(), "\n{}", stale.join("\n"));
}

#[test]
fn allow_list_has_no_duplicates() {
    for (i, a) in ALLOW.iter().enumerate() {
        assert!(!ALLOW[i + 1..].contains(a), "{a:?} is listed twice");
    }
}

/// Rule e, at the library: every removed spelling, typed after its command path,
/// is reported with the exact message.
#[test]
fn removed_spellings_are_reported_by_check() {
    use evnx::cli::removed::{Removed, check, message};
    use std::ffi::OsString;
    let all = entries();
    for r in evnx::cli::removed::REMOVED {
        if !all.iter().any(|e| e.path == r.path()) {
            continue; // the command's feature is not built
        }
        let (typed, tail): (String, Vec<&str>) = match r {
            Removed::Flag { old, .. } => (old[old.len() - 1].to_string(), vec![old[old.len() - 1]]),
            Removed::Value { flag, value, .. } => (format!("{flag} {value}"), vec![flag, value]),
        };
        let argv: Vec<OsString> = std::iter::once("evnx")
            .chain(r.path().iter().copied())
            .chain(tail)
            .map(OsString::from)
            .collect();
        assert_eq!(
            check(&argv),
            Some(message(r.path(), &typed, r.new_spelling())),
            "{r:?}"
        );
    }
}

/// Rule e, end to end: the binary exits 2, prints the message alone on stderr,
/// and prints nothing on stdout.
#[test]
fn removed_spellings_exit_2_with_the_message_only() {
    let all = entries();
    let project = support::Project::new();
    for r in evnx::cli::removed::REMOVED {
        if !all.iter().any(|e| e.path == r.path()) {
            continue; // the command's feature is not built
        }
        use evnx::cli::removed::Removed;
        let (typed, tail): (String, Vec<&str>) = match r {
            Removed::Flag { old, .. } => (old[old.len() - 1].to_string(), vec![old[old.len() - 1]]),
            Removed::Value { flag, value, .. } => (format!("{flag} {value}"), vec![flag, value]),
        };
        let args: Vec<&str> = r.path().iter().copied().chain(tail).collect();
        let out = support::run(project.path(), &args);
        out.assert_code(2);
        assert_eq!(
            out.stderr.trim_end(),
            evnx::cli::removed::message(r.path(), &typed, r.new_spelling())
        );
        assert!(out.stdout.is_empty(), "stdout: {}", out.stdout);
    }
}

/// The CHANGELOG lists every removed spelling, so the table and the release
/// notes cannot drift apart.
#[test]
fn changelog_lists_every_removed_spelling() {
    let log = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CHANGELOG.md"))
        .expect("CHANGELOG.md");
    let mut missing = Vec::new();
    for r in evnx::cli::removed::REMOVED {
        let row = format!("`evnx {}`", r.path().join(" "));
        let old = format!("`{}`", r.old_display());
        let found = log
            .lines()
            .any(|l| l.contains(&row) && l.contains(&old) && l.contains(r.new_spelling()));
        if !found {
            missing.push(format!("evnx {}: {}", r.path().join(" "), r.old_display()));
        }
    }
    assert!(
        missing.is_empty(),
        "not in CHANGELOG.md (add a table row):\n{}",
        missing.join("\n")
    );
}
