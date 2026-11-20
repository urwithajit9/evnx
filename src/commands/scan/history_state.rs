//! What `scan --history` remembers between runs (SCAN-RESCAN, D10).
//!
//! `.git/evnx/history.json` holds the last commit a clean history scan
//! covered, with the rule set and settings it was scanned under. It lives in
//! the clone's git folder, so it is never committed and a fresh clone starts
//! with a full scan.
//!
//! ⚠️ Only a clean scan is remembered. Recording the head after a scan that
//! found a secret would make the next run skip exactly the commits that hold
//! it, and report clean.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub schema_version: u32,
    /// The rule set the scan used (`EffectiveConfig::rules_digest`).
    pub rules_digest: String,
    /// A hash of the other settings that decide what is reported.
    pub settings: String,
    /// The head the clean scan covered, in full.
    pub last_commit: String,
    pub scanned_at: String,
}

/// What the next `--history` scan covers.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// All history; the reason is shown when there was a state to compare.
    Full(Option<String>),
    /// Only the commits after this one.
    Since(String),
    /// Nothing has been committed since this one.
    Nothing(String),
}

/// `.git/evnx/history.json` for the repository `cwd` is in.
pub fn path(cwd: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", "evnx/history.json"])
        .current_dir(cwd)
        .output()
        .context("running git")?;
    anyhow::ensure!(out.status.success(), "git could not name its folder");
    // Relative answers are relative to where git ran.
    Ok(cwd.join(String::from_utf8_lossy(&out.stdout).trim_end()))
}

/// The saved state. A missing or unreadable file is no state: the scan is full.
pub fn load(path: &Path) -> Option<State> {
    let state: State = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (state.schema_version == 1).then_some(state)
}

pub fn save(path: &Path, state: &State) -> Result<()> {
    if let Some(dir) = path.parent() {
        crate::core::safe_write::create_dirs(dir)?;
    }
    let json = serde_json::to_string_pretty(state)? + "\n";
    crate::core::safe_write::write_private(path, json.as_bytes())?;
    Ok(())
}

/// Whether `commit` is `HEAD` or one of its ancestors. A rewritten history, or
/// a commit that was garbage collected, is not.
fn is_ancestor(cwd: &Path, commit: &str) -> bool {
    Command::new("git")
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .current_dir(cwd)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Decide what to scan. `full` is `--full`.
pub fn plan(
    cwd: &Path,
    state: Option<&State>,
    rules_digest: &str,
    settings: &str,
    head: &str,
    full: bool,
) -> Plan {
    if full {
        return Plan::Full(None);
    }
    let Some(state) = state else {
        return Plan::Full(None);
    };
    if state.rules_digest != rules_digest {
        return Plan::Full(Some(
            "rules changed since the last history scan: scanning all history".to_string(),
        ));
    }
    if state.settings != settings {
        return Plan::Full(Some(
            "scan settings or the baseline changed since the last history scan: scanning all history"
                .to_string(),
        ));
    }
    if state.last_commit == head {
        return Plan::Nothing(state.last_commit.clone());
    }
    if !is_ancestor(cwd, &state.last_commit) {
        return Plan::Full(Some(
            "history changed since the last history scan (rewritten or rebased): scanning all history"
                .to_string(),
        ));
    }
    Plan::Since(state.last_commit.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(digest: &str, settings: &str, commit: &str) -> State {
        State {
            schema_version: 1,
            rules_digest: digest.into(),
            settings: settings.into(),
            last_commit: commit.into(),
            scanned_at: "2026-11-20".into(),
        }
    }

    #[test]
    fn no_state_or_a_changed_rule_set_scans_everything() {
        let here = Path::new(".");
        assert_eq!(plan(here, None, "d", "s", "h", false), Plan::Full(None));
        let s = state("old", "s", "c");
        match plan(here, Some(&s), "new", "s", "h", false) {
            Plan::Full(Some(why)) => assert!(why.starts_with("rules changed")),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            plan(here, Some(&s), "old", "other", "h", false),
            Plan::Full(Some(_))
        ));
        assert_eq!(
            plan(here, Some(&state("d", "s", "c")), "d", "s", "h", true),
            Plan::Full(None)
        );
    }

    #[test]
    fn an_unchanged_head_has_nothing_new() {
        assert_eq!(
            plan(
                Path::new("."),
                Some(&state("d", "s", "c")),
                "d",
                "s",
                "c",
                false
            ),
            Plan::Nothing("c".into())
        );
    }
}
