//! `evnx scan --history [RANGE]` (RM-69), against real temporary git repos.
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

fn clerk() -> String {
    ["sk_live_", "6kmbhJgs1nKKT5fAo1N92rDz3dgFDhqy"].concat()
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

fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = git_cmd(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// An empty repository on branch `main`, or `None` without git.
fn repo() -> Option<TempDir> {
    let d = TempDir::new().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]).then_some(d)
}

fn commit(dir: &Path, files: &[(&str, &str)], message: &str) {
    for (name, body) in files {
        let p = dir.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
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

fn findings(out: &Output) -> Vec<serde_json::Value> {
    doc(out)["findings"].as_array().unwrap().clone()
}

#[test]
fn a_secret_added_and_then_removed_is_found_once_at_its_commit() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(p, &[("a.env", "A=1\n")], "first");
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "oops",
    );
    let introduced = git_out(p, &["rev-parse", "HEAD"]);
    // Still there after an unrelated edit, then removed.
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\nOTHER=1\n", aws()))],
        "edit",
    );
    commit(p, &[("a.env", "AWS_ACCESS_KEY_ID=\nOTHER=1\n")], "remove");
    // The working tree is clean, and a plain scan says so.
    assert_eq!(evnx(p, &["scan", "a.env"]).status.code(), Some(0));

    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let f = findings(&out);
    assert_eq!(f.len(), 1, "one finding, not one per version: {f:?}");
    assert_eq!(f[0]["rule_id"], "aws-access-key");
    assert_eq!(f[0]["commit"], introduced.as_str());
    assert_eq!(f[0]["git"], "history");
    assert!(f[0]["commit_time"].as_i64().is_some());
    assert_eq!(f[0]["blob"].as_str().unwrap().len(), 40);
    let short = &introduced[..7];
    assert_eq!(
        f[0]["location"],
        format!("a.env@{short}:1 (AWS_ACCESS_KEY_ID)").as_str()
    );
}

#[test]
fn the_json_carries_the_run_header_and_never_a_value() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "k",
    );
    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    let v = doc(&out);
    assert_eq!(v["run"]["tool"], "evnx");
    assert_eq!(v["run"]["schema_version"], 3);
    assert_eq!(v["schema_version"], 3);
    assert!(!String::from_utf8_lossy(&out.stdout).contains(&aws()));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(&aws()));
}

#[test]
fn a_name_rule_sees_its_key_because_the_blob_is_read_whole() {
    let Some(d) = repo() else { return };
    let p = d.path();
    // `sk_live_` is Stripe's prefix; the name says whose key it is.
    commit(
        p,
        &[(".env", &format!("CLERK_SECRET_KEY={}\n", clerk()))],
        "k",
    );
    commit(p, &[(".env", "CLERK_SECRET_KEY=\n")], "rm");
    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    let f = findings(&out);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0]["rule_id"], "clerk-secret-key");
}

#[test]
fn a_blob_that_exists_only_in_a_merge_is_found() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(p, &[("a.env", "A=1\n")], "base");
    assert!(git(p, &["checkout", "-q", "-b", "side"]));
    commit(p, &[("a.env", "A=2\n")], "side");
    assert!(git(p, &["checkout", "-q", "main"]));
    commit(p, &[("a.env", "A=3\n")], "main");
    // The conflicting merge is resolved by hand, to a file neither side had.
    assert!(!git(p, &["merge", "-q", "side"]));
    fs::write(p.join("a.env"), format!("AWS_ACCESS_KEY_ID={}\n", aws())).unwrap();
    assert!(git(p, &["add", "a.env"]));
    assert!(git(p, &["commit", "-q", "-m", "merge"]));
    let merge = git_out(p, &["rev-parse", "HEAD"]);
    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    let f = findings(&out);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0]["commit"], merge.as_str());
}

#[test]
fn a_range_limits_the_commits() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("old.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "old",
    );
    assert!(git(p, &["tag", "v1"]));
    commit(p, &[("new.txt", "nothing\n")], "new");
    // The secret is below the range: not looked at.
    let out = evnx(p, &["scan", "--history", "v1..", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(findings(&out).is_empty());
    let out = evnx(p, &["scan", "--history", "v1", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let out = evnx(p, &["scan", "--history=HEAD~1..HEAD", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    // A branch only `all` reaches.
    assert!(git(p, &["checkout", "-q", "-b", "other", "v1"]));
    commit(p, &[("side.env", &format!("K2={}\n", aws()))], "side");
    assert!(git(p, &["checkout", "-q", "main"]));
    let head = evnx(p, &["scan", "--history", "--format", "json"]);
    let all = evnx(p, &["scan", "--history", "all", "--format", "json"]);
    assert_eq!(findings(&head).len(), 1);
    assert_eq!(findings(&all).len(), 2);
    assert!(
        findings(&all)
            .iter()
            .any(|f| f["location"].as_str().unwrap().starts_with("side.env@"))
    );
}

#[test]
fn paths_are_a_pathspec_and_exclude_applies() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[
            ("app/a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws())),
            (
                "lib/b.env",
                &format!("AWS_ACCESS_KEY_ID={}x\n", "4OZRMFJ3VREALKE2"),
            ),
        ],
        "both",
    );
    let out = evnx(p, &["scan", "--history", "--format", "json", "lib"]);
    let f = findings(&out);
    assert!(
        f.iter()
            .all(|f| f["location"].as_str().unwrap().starts_with("lib/")),
        "{f:?}"
    );
    let out = evnx(
        p,
        &["scan", "--history", "--format", "json", "--exclude", "app"],
    );
    assert!(
        findings(&out)
            .iter()
            .all(|f| !f["location"].as_str().unwrap().starts_with("app/"))
    );
}

#[test]
fn the_baseline_applies_to_history_findings() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "k",
    );
    commit(p, &[("a.env", "AWS_ACCESS_KEY_ID=\n")], "rm");
    let out = evnx(p, &["scan", "--history", "--update-baseline"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let base = fs::read_to_string(p.join(".evnx/baseline.json")).unwrap();
    // The baseline names the file, not the commit, and holds no value.
    assert!(base.contains("\"file\": \"a.env\""), "{base}");
    assert!(!base.contains('@'), "{base}");
    assert!(!base.contains(&aws()));
    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v = doc(&out);
    assert_eq!(v["summary"]["known"], 1);
    assert!(findings(&out).is_empty());
}

#[test]
fn a_deleted_file_is_not_a_blob_but_what_it_held_is() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("gone.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "add",
    );
    assert!(git(p, &["rm", "-q", "gone.env"]));
    assert!(git(p, &["commit", "-q", "-m", "delete"]));
    let out = evnx(p, &["scan", "--history", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(findings(&out).len(), 1);
}

#[test]
fn clean_history_exits_0_and_a_clean_range_with_no_files_says_so() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(p, &[("a.txt", "hello\n")], "one");
    assert_eq!(evnx(p, &["scan", "--history"]).status.code(), Some(0));
    let out = evnx(p, &["scan", "--history=HEAD..HEAD"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("nothing scanned"));
}

#[test]
fn trouble_is_exit_2_and_exit_zero_does_not_hide_it() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(p, &[("a.txt", "hello\n")], "one");
    for args in [
        &["scan", "--history=nope..HEAD"][..],
        &["scan", "--history=--output=x"][..],
        &["scan", "--history", "-"][..],
        &["scan", "--history", "--staged"][..],
    ] {
        let mut a = args.to_vec();
        a.push("--exit-zero");
        let out = evnx(p, &a);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }
}

#[test]
fn outside_a_repository_is_no_verdict() {
    let d = TempDir::new().unwrap();
    if git(d.path(), &["rev-parse"]) {
        return; // the temp folder sits inside a repository
    }
    let out = evnx(d.path(), &["scan", "--history"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--history needs a git repository"));
}

#[test]
fn the_advice_is_to_rotate_and_rewrite_history() {
    let Some(d) = repo() else { return };
    let p = d.path();
    commit(
        p,
        &[("a.env", &format!("AWS_ACCESS_KEY_ID={}\n", aws()))],
        "k",
    );
    let out = evnx(p, &["scan", "--history"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Revoke or rotate"), "{text}");
    assert!(text.contains("git filter-repo --path a.env"), "{text}");
    assert!(text.contains("a.env@"), "{text}");
}
