//! `evnx scan` inside a git repository (I-285, I-286; owner 2026-10-21).
//!
//! A walk counts what git would publish. A file git ignores and does not track
//! is still read, but its findings are summarised in one line and do not make
//! the run exit 1: a filled-in `.env` is where secrets belong. A tracked file
//! and one git would take on the next `git add .` are reported as before. A
//! file named on the command line, `--include-ignored`, and any folder outside
//! a repository report everything, as before.
//!
//! The advice under the findings follows the same facts: history rewriting
//! only for what may have been committed, named by its real path.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const AWS: &str = "AKIA4OZRMFJ3VREALKEY";
const GH: &str = "ghp_\
A1b2C3d4E5f6G7h8I9j0K1l2M3n4Fx3dtsr8";

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A repository holding `files`, with `committed` added (forced, so a file
/// `.gitignore` covers can be committed too) and committed. `None` without git.
fn repo(files: &[(&str, &str)], committed: &[&str]) -> Option<TempDir> {
    let d = TempDir::new().unwrap();
    if !git(d.path(), &["init", "-q"]) {
        return None;
    }
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    if !committed.is_empty() {
        let mut args = vec!["add", "-f", "--"];
        args.extend_from_slice(committed);
        assert!(git(d.path(), &args));
        assert!(git(d.path(), &["commit", "-qm", "x"]));
    }
    Some(d)
}

fn env_with_keys() -> String {
    format!("AWS_ACCESS_KEY_ID={AWS}\nGITHUB_TOKEN={GH}\n")
}

/// (exit code, stdout, stderr)
fn scan(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("NO_COLOR", "1")
        .arg("scan")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn json(dir: &TempDir, args: &[&str]) -> serde_json::Value {
    let mut all = vec!["--format", "json"];
    all.extend_from_slice(args);
    let (_, stdout, _) = scan(dir, &all);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"))
}

// ── I-285: what a walk counts ──────────────────────────────────────────────

/// The report: a gitignored, never-committed `.env` made every healthy project
/// fail with "assume they are compromised".
#[test]
fn t_i285_an_ignored_untracked_env_is_not_counted() {
    let Some(d) = repo(
        &[
            (".gitignore", ".env*\n"),
            (".env", &env_with_keys()),
            ("src/app.py", "x = 1\n"),
        ],
        &[".gitignore", "src/app.py"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("git ignores ./.env"), "{stdout}");
    assert!(stdout.contains("2 findings"), "{stdout}");
    assert!(stdout.contains("--include-ignored"), "{stdout}");
    assert!(!stdout.contains("compromised"), "{stdout}");
    assert!(
        !stdout.contains("AKIA…EY"),
        "held back, not listed: {stdout}"
    );
}

#[test]
fn t_i285_json_lists_ignored_files_and_counts_nothing() {
    let Some(d) = repo(
        &[(".gitignore", ".env*\n"), (".env", &env_with_keys())],
        &[".gitignore"],
    ) else {
        return;
    };
    let v = json(&d, &[]);
    assert_eq!(v["summary"]["total"], 0, "{v:#}");
    assert_eq!(v["summary"]["ignored_by_git"], 2, "{v:#}");
    assert_eq!(v["ignored_by_git"][0]["path"], "./.env", "{v:#}");
    assert_eq!(v["ignored_by_git"][0]["findings"], 2, "{v:#}");
    assert!(v["findings"].as_array().unwrap().is_empty(), "{v:#}");
}

/// Committed: the secret is in the repository, whatever `.gitignore` says now.
#[test]
fn t_i285_a_tracked_env_is_reported_even_if_ignored_later() {
    let Some(d) = repo(
        &[(".gitignore", ".env\n"), (".env", &env_with_keys())],
        &[".env"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    let v = json(&d, &[]);
    assert_eq!(v["summary"]["total"], 2, "{v:#}");
    assert_eq!(v["findings"][0]["git"], "tracked", "{v:#}");
}

/// Not ignored and not tracked: the next `git add .` takes it.
#[test]
fn t_i285_an_unignored_env_is_reported() {
    let Some(d) = repo(&[(".env", &env_with_keys())], &[]) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    let v = json(&d, &[]);
    assert_eq!(v["findings"][0]["git"], "unignored", "{v:#}");
}

/// A file you name is the file you asked about.
#[test]
fn t_i285_a_named_ignored_file_is_reported_in_full() {
    let Some(d) = repo(
        &[(".gitignore", ".env*\n"), (".env", &env_with_keys())],
        &[".gitignore"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[".env"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("AKIA…EY"), "{stdout}");
    let v = json(&d, &[".env"]);
    assert_eq!(v["findings"][0]["git"], "ignored", "{v:#}");
}

#[test]
fn t_i285_include_ignored_reports_them_as_findings() {
    let Some(d) = repo(
        &[(".gitignore", ".env*\n"), (".env", &env_with_keys())],
        &[".gitignore"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &["--include-ignored"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("AKIA…EY"), "{stdout}");
    assert!(!stdout.contains("--include-ignored lists"), "{stdout}");
}

/// A file inside an ignored folder is ignored too.
#[test]
fn t_i285_a_file_in_an_ignored_folder_is_held_back() {
    let Some(d) = repo(
        &[
            (".gitignore", "secrets/\n"),
            ("secrets/prod.env", &env_with_keys()),
        ],
        &[".gitignore"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("git ignores ./secrets/prod.env"),
        "{stdout}"
    );
}

/// `--severity` and `--confidence` decide what the held-back count counts, as
/// they do for findings.
#[test]
fn t_i285_the_held_back_count_follows_the_severity() {
    let Some(d) = repo(
        &[
            (".gitignore", ".env\n"),
            // AWS: critical, high confidence. A short value under a secret
            // name: high severity (C2), medium confidence.
            (
                ".env",
                &format!("AWS_ACCESS_KEY_ID={AWS}\nAPI_TOKEN=abcd1234\n"),
            ),
        ],
        &[".gitignore"],
    ) else {
        return;
    };
    assert_eq!(json(&d, &[])["summary"]["ignored_by_git"], 2);
    assert_eq!(
        json(&d, &["--severity", "high"])["summary"]["ignored_by_git"],
        2
    );
    assert_eq!(
        json(&d, &["--severity", "critical"])["summary"]["ignored_by_git"],
        1
    );
    assert_eq!(
        json(&d, &["--confidence", "high"])["summary"]["ignored_by_git"],
        1
    );
}

/// SARIF and GitHub annotations describe the repository; an ignored file is
/// not in it.
#[test]
fn t_i285_machine_formats_leave_ignored_files_out() {
    let Some(d) = repo(
        &[(".gitignore", ".env\n"), (".env", &env_with_keys())],
        &[".gitignore"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &["--format", "github"]);
    assert_eq!(code, 0);
    assert!(stdout.trim().is_empty(), "{stdout}");
    let (_, sarif, _) = scan(&d, &["--format", "sarif"]);
    let v: serde_json::Value = serde_json::from_str(&sarif).unwrap();
    assert!(
        v["runs"][0]["results"].as_array().unwrap().is_empty(),
        "{v:#}"
    );
}

/// Outside a repository nothing is known, so nothing changes.
#[test]
fn t_i285_outside_a_repository_everything_is_reported() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".gitignore"), ".env\n").unwrap();
    fs::write(d.path().join(".env"), env_with_keys()).unwrap();
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    let v = json(&d, &[]);
    assert!(v["findings"][0].get("git").is_none(), "{v:#}");
    assert!(v.get("ignored_by_git").is_none(), "{v:#}");
    assert!(v["summary"].get("ignored_by_git").is_none(), "{v:#}");
}

// ── I-286: the advice names the real file and fits what git knows ───────────

/// The footer said `git filter-repo --path .env` whatever file held the key.
#[test]
fn t_i286_history_advice_names_the_committed_file() {
    let Some(d) = repo(
        &[("src/main.py", &format!("k = \"{AWS}\"\n"))],
        &["src/main.py"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("git filter-repo --path src/main.py --invert-paths"),
        "{stdout}"
    );
    assert!(!stdout.contains("--path .env"), "{stdout}");
}

/// Never committed: there is no history to rewrite.
#[test]
fn t_i286_an_uncommitted_file_gets_keep_it_out_advice() {
    let Some(d) = repo(&[(".env", &env_with_keys())], &[]) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[]);
    assert!(!stdout.contains("filter-repo"), "{stdout}");
    assert!(stdout.contains("git would commit ./.env"), "{stdout}");
    assert!(stdout.contains(".gitignore"), "{stdout}");
}

/// A source file is not gitignored: its value moves out of it.
#[test]
fn t_i286_an_uncommitted_source_file_is_not_sent_to_gitignore() {
    let Some(d) = repo(&[("src/billing.py", &format!("k = \"{AWS}\"\n"))], &[]) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[]);
    assert!(
        stdout.contains("Move the values out of ./src/billing.py"),
        "{stdout}"
    );
    assert!(!stdout.contains("add it to .gitignore"), "{stdout}");
}

/// Named, ignored and untracked: say nothing was committed.
#[test]
fn t_i286_a_named_ignored_file_says_nothing_was_committed() {
    let Some(d) = repo(
        &[(".gitignore", ".env\n"), (".env", &env_with_keys())],
        &[".gitignore"],
    ) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[".env"]);
    assert!(!stdout.contains("filter-repo"), "{stdout}");
    assert!(!stdout.contains("compromised"), "{stdout}");
    assert!(stdout.contains("git ignores .env"), "{stdout}");
}

/// Outside a repository the advice is today's, with the real path.
#[test]
fn t_i286_outside_a_repository_the_history_advice_names_the_file() {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join("config")).unwrap();
    fs::write(d.path().join("config/app.yml"), format!("key: {AWS}\n")).unwrap();
    let (_, stdout, _) = scan(&d, &[]);
    assert!(
        stdout.contains("git filter-repo --path config/app.yml --invert-paths"),
        "{stdout}"
    );
}

/// I-290: one held-back finding is "it", not "them".
#[test]
fn t_i290_one_held_back_finding_is_it() {
    let Some(d) = repo(
        &[
            (".gitignore", ".env*\n"),
            (".env", &format!("GITHUB_TOKEN={GH}\n")),
        ],
        &[".gitignore"],
    ) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[]);
    assert!(stdout.contains("1 finding there is"), "{stdout}");
    assert!(stdout.contains("(--include-ignored lists it)"), "{stdout}");
}

// ── The walk line: counted files first, git-ignored ones apart ─────────────

/// `scanning ./.env, ./.gitignore, ./leak-test.txt` listed a git-ignored
/// `.env` beside the files that are counted, with nothing to tell them apart.
#[test]
fn the_walk_line_names_git_ignored_files_apart() {
    let Some(d) = repo(
        &[
            (".gitignore", ".env\n"),
            (".env", &env_with_keys()),
            ("leak-test.txt", &format!("key = {AWS}\n")),
        ],
        &[],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains(
            "scanning ./.gitignore, ./leak-test.txt · git-ignored (summarised only): ./.env\n"
        ),
        "{stdout}"
    );
    // `--include-ignored` counts it, so it is listed with the rest.
    let (_, stdout, _) = scan(&d, &["--include-ignored"]);
    assert!(
        stdout.contains("scanning ./.env, ./.gitignore, ./leak-test.txt\n"),
        "{stdout}"
    );
}

/// Long lists keep "and N other files", on each side.
#[test]
fn the_walk_line_shortens_both_lists() {
    let mut files: Vec<(String, String)> = vec![(".gitignore".into(), "*.env\n".into())];
    for i in 0..5 {
        files.push((format!("f{i}.txt"), "x = 1\n".into()));
        files.push((format!("e{i}.env"), "A=1\n".into()));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let Some(d) = repo(&refs, &[]) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[]);
    assert!(
        stdout.contains(
            "scanning ./.gitignore, ./f0.txt, ./f1.txt and 3 other files · git-ignored \
             (summarised only): ./e0.env, ./e1.env, ./e2.env and 2 other files\n"
        ),
        "{stdout}"
    );
}

// ── History rewriting only for a committed, high-confidence format match ───

/// A committed `PASSWORD=` in `tests/fixtures/`, flagged by its name alone,
/// came with revoke, `git filter-repo` and force push. It now gets "check it,
/// and if it is not a secret, allow it or baseline it"; the test folder is
/// named. A committed AWS key beside it keeps the rewrite steps, which name
/// only its file.
#[test]
fn history_rewrite_is_only_for_committed_format_matches() {
    let Some(d) = repo(
        &[
            ("tests/fixtures/app.env", "PASSWORD=hunter2x\n"),
            ("src/main.py", &format!("k = \"{AWS}\"\n")),
        ],
        &["tests/fixtures/app.env", "src/main.py"],
    ) else {
        return;
    };
    let (code, stdout, _) = scan(&d, &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("git filter-repo --path src/main.py --invert-paths"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("--path tests/fixtures/app.env"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Check ./tests/fixtures/app.env"),
        "{stdout}"
    );
    assert!(
        stdout.contains("./tests/fixtures/app.env is in a test folder"),
        "{stdout}"
    );
    assert!(stdout.contains("evnx:allow"), "{stdout}");
    assert!(stdout.contains("--update-baseline"), "{stdout}");
}

/// Only the name-matched file: no revoke, no rewrite, no force push.
#[test]
fn a_committed_name_match_alone_gets_no_rewrite_steps() {
    let Some(d) = repo(
        &[("tests/fixtures/app.env", "PASSWORD=hunter2x\n")],
        &["tests/fixtures/app.env"],
    ) else {
        return;
    };
    let (_, stdout, _) = scan(&d, &[]);
    assert!(!stdout.contains("filter-repo --path"), "{stdout}");
    assert!(!stdout.contains("Force push"), "{stdout}");
    assert!(!stdout.contains("compromised"), "{stdout}");
    assert!(
        stdout.contains("Check ./tests/fixtures/app.env"),
        "{stdout}"
    );
    assert!(
        stdout.contains("If one is a real secret, revoke or rotate it"),
        "{stdout}"
    );
}
