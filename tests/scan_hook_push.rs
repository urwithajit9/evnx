//! `evnx scan hook install --pre-push` (SCAN-PREPUSH, D11): a managed block in
//! the `pre-push` hook, run by real `git push` against a local bare remote.
//!
//! ⚠️ Fixture keys are assembled at run time, so this file holds none whole.
#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn aws() -> String {
    format!("AKIA{}", "4OZRMFJ3VREALKEY")
}

fn with_evnx_on_path() -> String {
    let bin = assert_cmd::cargo::cargo_bin!("evnx");
    format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn git_cmd(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("PATH", with_evnx_on_path())
        .env("NO_COLOR", "1");
    c
}

fn git(dir: &Path, args: &[&str]) -> bool {
    git_cmd(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn evnx(dir: &Path, args: &[&str]) -> Output {
    cargo_bin_cmd!("evnx")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap()
}

/// A working repository with the hook installed and a bare remote called
/// `origin`; `None` without git.
struct Setup {
    work: TempDir,
    _remote: TempDir,
}

fn setup() -> Option<Setup> {
    let remote = TempDir::new().unwrap();
    if !git(remote.path(), &["init", "-q", "--bare", "-b", "main"]) {
        return None;
    }
    let work = TempDir::new().unwrap();
    let w = work.path();
    assert!(git(w, &["init", "-q", "-b", "main"]));
    assert!(git(w, &["config", "user.email", "dev@example.com"]));
    assert!(git(
        w,
        &["remote", "add", "origin", remote.path().to_str().unwrap()]
    ));
    assert!(
        evnx(w, &["scan", "hook", "install", "--pre-push"])
            .status
            .success()
    );
    Some(Setup {
        work,
        _remote: remote,
    })
}

fn commit(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).unwrap();
    assert!(git(dir, &["add", "-A"]));
    assert!(git(dir, &["commit", "-q", "--no-verify", "-m", name]));
}

fn secret_file() -> String {
    format!("AWS_ACCESS_KEY_ID={}\n", aws())
}

fn push(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = git_cmd(dir);
    c.arg("push").args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn all_text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Whether the remote has `branch`.
fn remote_has(dir: &Path, branch: &str) -> bool {
    git(
        dir,
        &["ls-remote", "--exit-code", "--heads", "origin", branch],
    )
}

#[test]
fn the_block_installs_beside_pre_commit_and_each_comes_out_alone() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    let push_hook = w.join(".git/hooks/pre-push");
    let commit_hook = w.join(".git/hooks/pre-commit");
    assert!(push_hook.exists() && !commit_hook.exists());
    // A line the hook already had survives install, reinstall and uninstall.
    fs::write(&push_hook, "#!/bin/sh\necho mine\n").unwrap();
    for _ in 0..2 {
        assert!(
            evnx(w, &["scan", "hook", "install", "--pre-push"])
                .status
                .success()
        );
    }
    let text = fs::read_to_string(&push_hook).unwrap();
    assert_eq!(text.matches(">>> evnx scan (managed) >>>").count(), 1);
    assert!(text.starts_with("#!/bin/sh\necho mine\n"));
    assert!(text.contains("evnx scan --history"));
    // The pre-commit hook is a separate file with its own block.
    assert!(evnx(w, &["scan", "hook", "install"]).status.success());
    let commit_text = fs::read_to_string(&commit_hook).unwrap();
    assert!(commit_text.contains("evnx scan --staged"));
    assert!(!commit_text.contains("--history"));
    let status = evnx(w, &["scan", "hook", "status", "--pre-push"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("installed:"));
    assert!(
        evnx(w, &["scan", "hook", "uninstall", "--pre-push"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(&push_hook).unwrap(),
        "#!/bin/sh\necho mine\n"
    );
    assert!(commit_hook.exists(), "pre-commit is untouched");
    let status = evnx(w, &["scan", "hook", "status", "--pre-push"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("not installed"));
    let status = evnx(w, &["scan", "hook", "status"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("installed:"));
}

#[test]
fn a_manager_generated_pre_push_is_left_alone_and_the_hook_is_named() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    let hook = w.join(".git/hooks/pre-push");
    let body = "#!/usr/bin/env bash\n# File generated by pre-commit: x\nexec pre-commit\n";
    fs::write(&hook, body).unwrap();
    let out = evnx(w, &["scan", "hook", "install", "--pre-push"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("evnx-scan-push") && err.contains("--hook-type pre-push"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&hook).unwrap(), body);
}

#[test]
fn a_push_with_a_secret_in_the_new_commits_is_blocked_without_printing_it() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "a.txt", "hello\n");
    commit(w, "k.env", &secret_file());
    let out = push(w, &["origin", "main"], &[]);
    assert!(!out.status.success(), "{out:?}");
    let text = all_text(&out);
    assert!(
        text.contains("push blocked, secrets are in the commits"),
        "{text}"
    );
    assert!(text.contains("EVNX_SCAN_BYPASS"), "{text}");
    assert!(
        !text.contains(&aws()),
        "the key was printed in full:\n{text}"
    );
    assert!(!remote_has(w, "main"));
}

#[test]
fn a_clean_push_goes_through() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "a.txt", "hello\n");
    let out = push(w, &["origin", "main"], &[]);
    assert!(out.status.success(), "{}", all_text(&out));
    assert!(remote_has(w, "main"));
}

#[test]
fn only_the_commits_the_remote_lacks_are_scanned() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    // The secret reached the remote some other way (no hook).
    commit(w, "k.env", &secret_file());
    assert!(git(w, &["push", "-q", "--no-verify", "origin", "main"]));
    // A clean commit on top: the push is the one new commit.
    commit(w, "b.txt", "two\n");
    let out = push(w, &["origin", "main"], &[]);
    assert!(out.status.success(), "{}", all_text(&out));
    // A new secret on top is found, the old one is not what is reported.
    commit(w, "k2.env", &secret_file().replace("KEY_ID", "KEY_ID2"));
    let out = push(w, &["origin", "main"], &[]);
    assert!(!out.status.success());
}

#[test]
fn a_new_branch_scans_what_no_remote_has_and_a_delete_is_skipped() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "a.txt", "hello\n");
    assert!(push(w, &["origin", "main"], &[]).status.success());
    // A branch off pushed work holds nothing new: it goes through.
    assert!(git(w, &["checkout", "-q", "-b", "same"]));
    assert!(push(w, &["origin", "same"], &[]).status.success());
    // A branch with a secret commit is blocked.
    assert!(git(w, &["checkout", "-q", "-b", "leaky"]));
    commit(w, "k.env", &secret_file());
    let out = push(w, &["origin", "leaky"], &[]);
    assert!(!out.status.success(), "{}", all_text(&out));
    assert!(!remote_has(w, "leaky"));
    // Deleting a remote branch is not a scan.
    let out = push(w, &["origin", "--delete", "same"], &[]);
    assert!(out.status.success(), "{}", all_text(&out));
    assert!(!remote_has(w, "same"));
}

#[test]
fn the_hook_run_by_hand_scans_upstream_to_head() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "a.txt", "hello\n");
    assert!(git(
        w,
        &["push", "-q", "-u", "--no-verify", "origin", "main"]
    ));
    commit(w, "k.env", &secret_file());
    let run = |w: &Path| {
        git_cmd(w)
            .args(["hook", "run", "--ignore-missing", "pre-push"])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    let out = run(w);
    assert_eq!(out.status.code(), Some(1), "{}", all_text(&out));
}

#[test]
fn a_bypass_with_a_reason_pushes_and_is_logged() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "k.env", &secret_file());
    let head = String::from_utf8(
        git_cmd(w)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let out = push(
        w,
        &["origin", "main"],
        &[("EVNX_SCAN_BYPASS", "test key, revoked\nforged\tfield")],
    );
    assert!(out.status.success(), "{}", all_text(&out));
    assert!(remote_has(w, "main"));
    assert!(all_text(&out).contains("bypassed with a reason"));
    let log = fs::read_to_string(w.join(".git/evnx/bypass.log")).unwrap();
    // One line: time, who, which commits, why. The reason cannot add a line
    // or a field.
    assert_eq!(log.lines().count(), 1, "{log:?}");
    let fields: Vec<&str> = log.trim_end().split('\t').collect();
    assert_eq!(fields.len(), 4, "{fields:?}");
    assert!(
        fields[0].ends_with('Z') && fields[0].len() == 20,
        "{fields:?}"
    );
    assert_eq!(fields[1], "dev@example.com");
    assert!(fields[2].contains(head.trim()), "{fields:?}");
    assert_eq!(fields[3], "test key, revoked forged field");
    // The log holds no value, and is inside `.git`: never committed.
    assert!(!log.contains(&aws()));
    let status = git_cmd(w).args(["status", "--porcelain"]).output().unwrap();
    assert!(status.stdout.is_empty());
}

#[test]
fn a_bypass_without_a_reason_is_refused() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "k.env", &secret_file());
    for reason in ["", "   ", "\n\t"] {
        let out = push(w, &["origin", "main"], &[("EVNX_SCAN_BYPASS", reason)]);
        assert!(!out.status.success(), "{reason:?}: {}", all_text(&out));
        assert!(
            all_text(&out).contains("EVNX_SCAN_BYPASS needs a reason"),
            "{}",
            all_text(&out)
        );
        assert!(!remote_has(w, "main"));
        assert!(!w.join(".git/evnx/bypass.log").exists());
    }
}

#[test]
fn a_missing_evnx_blocks_the_push_with_a_hint() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    if Path::new("/usr/bin/evnx").exists() || Path::new("/bin/evnx").exists() {
        return;
    }
    commit(w, "a.txt", "hello\n");
    let out = git_cmd(w)
        .args(["push", "origin", "main"])
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = all_text(&out);
    assert!(
        text.contains("evnx is not on PATH") && text.contains("--no-verify"),
        "{text}"
    );
    assert!(!remote_has(w, "main"));
}

#[test]
fn the_config_switch_turns_it_off() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "k.env", &secret_file());
    assert!(git(w, &["config", "hooks.evnx-scan", "false"]));
    let out = push(w, &["origin", "main"], &[]);
    assert!(out.status.success(), "{}", all_text(&out));
    assert!(all_text(&out).contains("skipped"));
}

/// The hook's advice said "force push, after coordinating with everyone who
/// has a clone" and `git filter-repo` for commits no remote had yet. Local
/// commits are fixed locally: amend the last one, or rebase, then push.
#[test]
fn a_blocked_push_of_local_commits_says_amend_not_force_push() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "a.txt", "hello\n");
    assert!(push(w, &["origin", "main"], &[]).status.success());
    // The secret is in the last commit: amend it.
    commit(w, "k.env", &secret_file());
    let out = push(w, &["origin", "main"], &[]);
    assert!(!out.status.success());
    let text = all_text(&out);
    assert!(text.contains("git commit --amend"), "{text}");
    assert!(text.contains("no force push"), "{text}");
    assert!(!text.contains("Force push"), "{text}");
    assert!(!text.contains("filter-repo --path"), "{text}");
    // An older local commit: rebase from the one before it.
    commit(w, "b.txt", "two\n");
    let first = String::from_utf8(
        git_cmd(w)
            .args(["rev-parse", "--short=7", "HEAD~1"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let out = push(w, &["origin", "main"], &[]);
    assert!(!out.status.success());
    let text = all_text(&out);
    assert!(
        text.contains(&format!("git rebase -i {}^", first.trim())),
        "{text}"
    );
    assert!(
        !text.contains("Force push") && !text.contains("filter-repo --path"),
        "{text}"
    );
    // A first push of a branch to an empty remote is local commits too.
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "k.env", &secret_file());
    let text = all_text(&push(w, &["origin", "main"], &[]));
    assert!(text.contains("git commit --amend"), "{text}");
    assert!(!text.contains("Force push"), "{text}");
}

/// History a remote already has keeps the rewrite and force push advice.
#[test]
fn a_secret_a_remote_already_has_keeps_the_force_push_advice() {
    let Some(s) = setup() else { return };
    let w = s.work.path();
    commit(w, "k.env", &secret_file());
    assert!(git(w, &["push", "-q", "--no-verify", "origin", "main"]));
    let out = evnx(w, &["scan", "--history"]);
    let text = all_text(&out);
    assert!(text.contains("git filter-repo --path k.env"), "{text}");
    assert!(text.contains("Force push"), "{text}");
    assert!(!text.contains("--amend"), "{text}");
    // Removed and then added back in a local commit: the remote has the file
    // with that key, so amending the local commit is not enough.
    assert!(git(w, &["rm", "-q", "k.env"]));
    assert!(git(w, &["commit", "-q", "--no-verify", "-m", "rm"]));
    commit(w, "k.env", &secret_file());
    let text = all_text(&push(w, &["origin", "main"], &[]));
    assert!(text.contains("Force push"), "{text}");
    assert!(!text.contains("--amend"), "{text}");
}
