//! `evnx scan --staged` and `evnx scan hook` (C9; S11, S12), against real
//! `git init` repositories.
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
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// An empty repository, or `None` without git.
fn repo() -> Option<TempDir> {
    let d = TempDir::new().unwrap();
    git(d.path(), &["init", "-q"]).then_some(d)
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

fn rule_ids(out: &Output) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_staged_secret_is_found_with_a_clean_working_tree() {
    let Some(d) = repo() else { return };
    fs::write(
        d.path().join("a.env"),
        format!("AWS_ACCESS_KEY_ID={}\n", aws()),
    )
    .unwrap();
    assert!(git(d.path(), &["add", "a.env"]));
    // Edited out on disk after staging: the commit still carries it.
    fs::write(d.path().join("a.env"), "AWS_ACCESS_KEY_ID=\n").unwrap();
    let out = evnx(d.path(), &["scan", "--staged", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(rule_ids(&out), ["aws-access-key"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["findings"][0]["git"], "staged");
    // And a plain scan of the disk, for contrast, finds nothing.
    assert_eq!(evnx(d.path(), &["scan", "a.env"]).status.code(), Some(0));
}

#[test]
fn a_secret_on_disk_only_is_not_staged_and_not_found() {
    let Some(d) = repo() else { return };
    fs::write(d.path().join("a.env"), "A=1\n").unwrap();
    assert!(git(d.path(), &["add", "a.env"]));
    fs::write(
        d.path().join("a.env"),
        format!("AWS_ACCESS_KEY_ID={}\n", aws()),
    )
    .unwrap();
    fs::write(
        d.path().join("b.env"),
        format!("AWS_ACCESS_KEY_ID={}\n", aws()),
    )
    .unwrap();
    let out = evnx(d.path(), &["scan", "--staged", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(rule_ids(&out).is_empty());
}

#[test]
fn nothing_staged_is_clean_and_says_so() {
    let Some(d) = repo() else { return };
    let out = evnx(d.path(), &["scan", "--staged"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("nothing staged, nothing scanned"));
}

#[test]
fn outside_a_repository_is_no_verdict() {
    let d = TempDir::new().unwrap();
    if git(d.path(), &["rev-parse"]) {
        return; // the temp folder sits inside a repository
    }
    let out = evnx(d.path(), &["scan", "--staged"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--staged needs a git repository"));
}

#[test]
fn paths_limit_the_staged_files_and_stdin_is_refused() {
    let Some(d) = repo() else { return };
    fs::create_dir_all(d.path().join("app")).unwrap();
    fs::write(d.path().join("a.env"), format!("K={}\n", aws())).unwrap();
    fs::write(d.path().join("app/b.env"), "B=1\n").unwrap();
    assert!(git(d.path(), &["add", "."]));
    let out = evnx(d.path(), &["scan", "--staged", "--format", "json", "app"]);
    assert_eq!(out.status.code(), Some(0));
    let out = evnx(d.path(), &["scan", "--staged", "-"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_gitignored_file_that_is_staged_is_reported_and_unstaging_is_the_advice() {
    let Some(d) = repo() else { return };
    fs::write(d.path().join(".gitignore"), ".env\n").unwrap();
    fs::write(
        d.path().join(".env"),
        format!("AWS_ACCESS_KEY_ID={}\n", aws()),
    )
    .unwrap();
    assert!(git(d.path(), &["add", "-f", ".env"]));
    let out = evnx(d.path(), &["scan", "--staged"]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("git restore --staged .env"), "{text}");
    // Nothing is committed yet: no history rewriting.
    assert!(!text.contains("filter-repo"), "{text}");
}

fn hook_file(d: &Path) -> std::path::PathBuf {
    d.join(".git/hooks/pre-commit")
}

#[test]
fn install_is_idempotent_and_keeps_and_restores_an_existing_hook() {
    let Some(d) = repo() else { return };
    let hook = hook_file(d.path());
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\necho mine\n").unwrap();
    for _ in 0..2 {
        assert!(evnx(d.path(), &["scan", "hook", "install"])
            .status
            .success());
    }
    let text = fs::read_to_string(&hook).unwrap();
    assert_eq!(text.matches(">>> evnx scan (managed) >>>").count(), 1);
    assert!(text.starts_with("#!/bin/sh\necho mine\n"));
    let status = evnx(d.path(), &["scan", "hook", "status"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("installed:"));
    assert!(evnx(d.path(), &["scan", "hook", "uninstall"])
        .status
        .success());
    assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\necho mine\n");
}

#[test]
fn a_new_hook_is_removed_whole_by_uninstall() {
    let Some(d) = repo() else { return };
    assert!(evnx(d.path(), &["scan", "hook", "install"])
        .status
        .success());
    let hook = hook_file(d.path());
    assert!(fs::read_to_string(&hook)
        .unwrap()
        .starts_with("#!/bin/sh\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert!(evnx(d.path(), &["scan", "hook", "uninstall"])
        .status
        .success());
    assert!(!hook.exists());
}

#[test]
fn core_hooks_path_is_honoured() {
    let Some(d) = repo() else { return };
    assert!(git(d.path(), &["config", "core.hooksPath", "githooks"]));
    // `git config` above wrote the repository's own config, which the run reads.
    assert!(evnx(d.path(), &["scan", "hook", "install"])
        .status
        .success());
    assert!(d.path().join("githooks/pre-commit").exists());
    assert!(!hook_file(d.path()).exists());
}

#[test]
fn a_hook_a_manager_generated_is_left_alone() {
    let Some(d) = repo() else { return };
    let hook = hook_file(d.path());
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let body = "#!/usr/bin/env bash\n# File generated by prek\nexec prek\n";
    fs::write(&hook, body).unwrap();
    let out = evnx(d.path(), &["scan", "hook", "install"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("prek manages") && err.contains("evnx-scan-installed"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&hook).unwrap(), body);
}

/// The hook, run by a real `git commit`, with `path` as `PATH`.
#[cfg(unix)]
fn commit(d: &Path, path: &str) -> Output {
    git_cmd(d)
        .args(["commit", "-qm", "x"])
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[cfg(unix)]
fn with_evnx_on_path() -> String {
    let bin = assert_cmd::cargo::cargo_bin!("evnx");
    format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(unix)]
#[test]
fn the_hook_blocks_a_staged_secret_without_printing_it() {
    let Some(d) = repo() else { return };
    assert!(evnx(d.path(), &["scan", "hook", "install"])
        .status
        .success());
    fs::write(
        d.path().join("a.env"),
        format!("AWS_ACCESS_KEY_ID={}\n", aws()),
    )
    .unwrap();
    assert!(git(d.path(), &["add", "a.env"]));
    let out = commit(d.path(), &with_evnx_on_path());
    assert!(!out.status.success());
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("commit blocked, secrets are staged"), "{all}");
    assert!(!all.contains(&aws()), "the key was printed in full:\n{all}");

    // Turned off: the commit goes through, and says it skipped.
    assert!(git(d.path(), &["config", "hooks.evnx-scan", "false"]));
    let out = commit(d.path(), &with_evnx_on_path());
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("skipped"));
}

/// S12: no evnx on `PATH` blocks the commit, with a hint.
#[cfg(unix)]
#[test]
fn the_hook_blocks_when_evnx_is_missing() {
    let Some(d) = repo() else { return };
    assert!(evnx(d.path(), &["scan", "hook", "install"])
        .status
        .success());
    fs::write(d.path().join("a.txt"), "hello\n").unwrap();
    assert!(git(d.path(), &["add", "a.txt"]));
    // git and sh, without the folder evnx is built into.
    let path = "/usr/bin:/bin";
    if Path::new("/usr/bin/evnx").exists() || Path::new("/bin/evnx").exists() {
        return;
    }
    let out = commit(d.path(), path);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("evnx is not on PATH") && err.contains("--no-verify"),
        "{err}"
    );
}
