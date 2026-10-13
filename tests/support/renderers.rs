//! One scenario, two output formats (T-SHARE-33).
//!
//! A command with a human renderer and a machine renderer (`doctor --format
//! json`, `add --json`) must agree on the facts: the same exit code and the
//! same changes on disk. A `--fix` that writes a file under one renderer and not
//! the other is a bug that neither renderer's own tests can see.
//!
//! What the two outputs *say* is format-specific; this returns both so the
//! caller can compare the parts that matter for its command.

use super::{run, Output, Project};
use std::collections::BTreeMap;
use std::path::Path;

/// The result of [`both_renderers`].
pub struct Renderings {
    pub pretty: Output,
    pub machine: Output,
    /// Files whose content changed (or appeared, or vanished) in the run, by
    /// relative path, from the pretty run. Equal for both runs, or the call
    /// panics.
    pub changed: Vec<String>,
}

/// Build the scenario twice with `setup`, run `pretty_args` on one copy and
/// `machine_args` on the other, and assert the same exit code and the same
/// files changed with the same content.
///
/// ```text
/// let r = both_renderers(
///     || Project::new().env("A=1\n"),
///     &["doctor"],
///     &["doctor", "--format", "json"],
/// );
/// let _: serde_json::Value = serde_json::from_str(&r.machine.stdout).unwrap();
/// ```
#[track_caller]
pub fn both_renderers(
    setup: impl Fn() -> Project,
    pretty_args: &[&str],
    machine_args: &[&str],
) -> Renderings {
    let (pretty, pretty_changes) = run_once(&setup, pretty_args);
    let (machine, machine_changes) = run_once(&setup, machine_args);

    assert_eq!(
        pretty.code,
        machine.code,
        "exit codes differ between renderers\n--- pretty\n{}\n--- machine\n{}",
        pretty.all(),
        machine.all()
    );
    assert_eq!(
        pretty_changes, machine_changes,
        "the two renderers changed different files (path -> content after the run)"
    );

    Renderings {
        pretty,
        machine,
        changed: pretty_changes.keys().cloned().collect(),
    }
}

/// Run `args` on a fresh copy of the scenario and report what it changed.
fn run_once(
    setup: &impl Fn() -> Project,
    args: &[&str],
) -> (Output, BTreeMap<String, Option<Vec<u8>>>) {
    let p = setup();
    let before = tree(p.path());
    let out = run(p.path(), args);
    let after = tree(p.path());

    let mut changed = BTreeMap::new();
    for (path, content) in &after {
        if before.get(path) != Some(content) {
            changed.insert(path.clone(), Some(content.clone()));
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            changed.insert(path.clone(), None);
        }
    }
    // Content dated by the run itself (an `Added … on <date>` stamp) would
    // differ between the two runs when they straddle a minute; compare it
    // normalised.
    let changed = changed
        .into_iter()
        .map(|(k, v)| {
            let v = v.map(|bytes| {
                super::snapshot::normalise(&String::from_utf8_lossy(&bytes), &[p.path()])
                    .into_bytes()
            });
            (k, v)
        })
        .collect();
    (out, changed)
}

/// Every file under `root` except `.git`, by `/`-separated relative path.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == ".git" {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}
