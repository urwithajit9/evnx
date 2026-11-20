//! `evnx scan --history` remembering the last clean scan, and the rule-set
//! digest (SCAN-RESCAN, D7, D10), against real temporary git repos.
//!
//! ⚠️ Fixture keys are assembled at run time, so this file holds none whole.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn aws() -> String {
    format!("AKIA{}", "4OZRMFJ3VREALKEY")
}

fn git_cmd(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    c
}

fn git(dir: &Path, args: &[&str]) -> bool {
    git_cmd(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo() -> Option<TempDir> {
    let d = TempDir::new().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]).then_some(d)
}

fn commit(dir: &Path, files: &[(&str, &str)], message: &str) {
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
    assert!(git(dir, &["add", "-A"]));
    assert!(git(dir, &["commit", "-q", "-m", message]));
}

fn evnx(dir: &Path, args: &[&str]) -> Output {
    cargo_bin_cmd!("evnx")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap()
}

fn doc(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&out.stdout)))
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn state_file(p: &Path) -> std::path::PathBuf {
    p.join(".git/evnx/history.json")
}

/// A repository with three clean commits.
fn clean_repo() -> Option<TempDir> {
    let d = repo()?;
    commit(d.path(), &[("a.txt", "one\n")], "1");
    commit(d.path(), &[("b.txt", "two\n")], "2");
    commit(d.path(), &[("c.txt", "three\n")], "3");
    Some(d)
}

const JSON: [&str; 3] = ["scan", "--history", "--format"];

fn run_json(p: &Path, extra: &[&str]) -> Output {
    let mut args: Vec<&str> = JSON.to_vec();
    args.push("json");
    args.extend_from_slice(extra);
    evnx(p, &args)
}

#[test]
fn the_digest_is_in_the_report_and_changes_with_the_rules() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    let a = doc(&run_json(p, &[]));
    let digest = a["effective_config"]["rules_digest"].as_str().unwrap();
    assert_eq!(digest.len(), 64, "{digest}");
    let again = doc(&run_json(p, &[]));
    assert_eq!(again["effective_config"]["rules_digest"], digest);
    let more = doc(&run_json(p, &["--pattern", "ZZZ[0-9]{6}"]));
    assert_ne!(more["effective_config"]["rules_digest"], digest);
    // A rule turned off by `[scan] disable` is a different rule set too.
    fs::write(p.join(".evnx.toml"), "[scan]\ndisable = [\"jwt\"]\n").unwrap();
    let off = doc(&run_json(p, &[]));
    assert_ne!(off["effective_config"]["rules_digest"], digest);
}

#[test]
fn the_baseline_records_the_digest_and_stays_schema_1() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "k",
    );
    let out = evnx(p, &["scan", "--history", "--update-baseline"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let base: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(p.join(".evnx/baseline.json")).unwrap()).unwrap();
    assert_eq!(base["schema_version"], 1);
    assert_eq!(base["rules_digest"].as_str().unwrap().len(), 64);
    // The same digest the report names.
    let v = doc(&run_json(p, &[]));
    assert_eq!(v["effective_config"]["rules_digest"], base["rules_digest"]);
    // Nothing of a value or a rule is in it.
    assert!(!base.to_string().contains(&aws()));
}

#[test]
fn a_clean_scan_is_remembered_in_the_git_folder_and_never_committed() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    assert!(!state_file(p).exists());
    let out = run_json(p, &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_file(p)).unwrap()).unwrap();
    let head = String::from_utf8(
        git_cmd(p)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(state["last_commit"], head.trim());
    assert_eq!(doc(&out)["files_scanned"], 3);
    // Inside `.git`: the working tree has nothing new to commit.
    let status = git_cmd(p).args(["status", "--porcelain"]).output().unwrap();
    assert!(status.stdout.is_empty(), "{status:?}");
}

#[test]
fn unchanged_rules_scan_only_the_new_commits() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    assert_eq!(doc(&run_json(p, &[]))["files_scanned"], 3);
    // Nothing new: nothing read, still exit 0.
    let out = run_json(p, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(doc(&out)["files_scanned"], 0);
    assert!(stderr(&out).contains("no commits since the last history scan"));
    // One new commit: one file read, and it is the secret.
    commit(
        p,
        &[("d.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "4",
    );
    let out = run_json(p, &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let v = doc(&out);
    assert_eq!(v["files_scanned"], 1);
    assert_eq!(v["findings"].as_array().unwrap().len(), 1);
    assert!(stderr(&out).contains("scanning the commits since"));
}

#[test]
fn a_scan_that_found_something_is_not_remembered() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "k",
    );
    for _ in 0..2 {
        // Found again, not skipped as already seen.
        let out = run_json(p, &[]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(doc(&out)["findings"].as_array().unwrap().len(), 1);
        assert!(!state_file(p).exists());
    }
    // Accepted in the baseline: clean, and remembered from then on.
    assert_eq!(
        evnx(p, &["scan", "--history", "--update-baseline"])
            .status
            .code(),
        Some(0)
    );
    assert!(state_file(p).exists());
}

#[test]
fn changed_rules_scan_all_history_again() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    assert_eq!(doc(&run_json(p, &[]))["files_scanned"], 3);
    let out = run_json(p, &["--pattern", "ZZZ[0-9]{6}"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(doc(&out)["files_scanned"], 3);
    assert!(
        stderr(&out).contains("rules changed since the last history scan: scanning all history"),
        "{}",
        stderr(&out)
    );
    // That scan is what the next one builds on.
    let next = run_json(p, &["--pattern", "ZZZ[0-9]{6}"]);
    assert_eq!(doc(&next)["files_scanned"], 0);
}

#[test]
fn a_project_rule_added_is_a_changed_rule_set() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    assert_eq!(doc(&run_json(p, &[]))["files_scanned"], 3);
    fs::create_dir_all(p.join(".evnx/rules")).unwrap();
    fs::write(
        p.join(".evnx/rules/acme.toml"),
        "version = 1\n[[rules]]\nid = \"acme-token\"\nname = \"Acme token\"\nregex = 'ACME-[A-Z0-9]{16}'\nconfidence = \"high\"\nseverity = \"high\"\n",
    )
    .unwrap();
    let out = run_json(p, &[]);
    assert_eq!(doc(&out)["files_scanned"], 3, "{}", stderr(&out));
    assert!(stderr(&out).contains("rules changed"), "{}", stderr(&out));
}

#[test]
fn full_rewritten_history_and_other_settings_all_start_over() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    assert_eq!(doc(&run_json(p, &[]))["files_scanned"], 3);
    // --full.
    assert_eq!(doc(&run_json(p, &["--full"]))["files_scanned"], 3);
    // Other settings.
    let out = run_json(p, &["--exclude", "c.txt"]);
    assert_eq!(doc(&out)["files_scanned"], 2);
    assert!(stderr(&out).contains("settings"), "{}", stderr(&out));
    // Rewritten history: the remembered commit is no longer an ancestor.
    let _ = run_json(p, &[]);
    assert!(git(p, &["commit", "-q", "--amend", "-m", "3 again"]));
    let out = run_json(p, &[]);
    assert_eq!(doc(&out)["files_scanned"], 3);
    assert!(stderr(&out).contains("history changed"), "{}", stderr(&out));
}

#[test]
fn a_range_or_paths_scan_what_was_asked_and_remember_nothing() {
    let Some(d) = clean_repo() else { return };
    let p = d.path();
    let out = evnx(p, &["scan", "--history=HEAD~1..HEAD", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!state_file(p).exists());
    let out = evnx(p, &["scan", "--history", "--format", "json", "a.txt"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!state_file(p).exists());
    // `--full` is a flag of a bare `--history` only.
    assert_eq!(evnx(p, &["scan", "--full"]).status.code(), Some(2));
}
