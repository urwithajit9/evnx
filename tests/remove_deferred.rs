//! The tests plan §13.4 deferred from phase 1 (N14-13): E-7 (concurrent runs),
//! F-3 (remove then add), G-2 (case-colliding names), G-4 (a target that cannot
//! be written). E-6 (interruption) is unit-tested beside the writer in
//! `src/commands/remove/mod.rs`, where the hook that stops a run between staging
//! and renaming lives.
//!
//! Not done, and why: G-1 (Windows) and G-3's platform half need a Windows
//! runner; CI's matrix covers them, a Linux-only suite cannot.

mod support;

use support::{run, Project};

fn evnx(p: &Project, args: &[&str]) -> support::run::Output {
    run(p.path(), args)
}

fn lock_file(p: &Project, name: &str) -> std::path::PathBuf {
    p.join(&format!(".evnx-{name}.lock"))
}

// ── E-7 ─────────────────────────────────────────────────────────────────────

/// A lock held by a live process stops the run, with a message naming it, and
/// changes nothing: no file, no backup.
#[test]
fn e7_a_held_lock_aborts_the_run_and_changes_nothing() {
    let p = Project::new().env("A=1\nB=2\n");
    std::fs::write(lock_file(&p, ".env"), format!("{}\n", std::process::id())).unwrap();
    evnx(&p, &["remove", "A", "--yes"])
        .assert_code(2)
        .assert_contains("another evnx")
        .assert_contains(&std::process::id().to_string())
        .assert_contains("No verdict");
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
    assert!(!p.exists(".env.bak"));
    assert!(
        lock_file(&p, ".env").exists(),
        "someone else's lock was removed"
    );

    // `sync --prune` takes the same lock, on the template.
    let p = Project::new().env("A=1\n").example("A=\nGONE=\n");
    std::fs::write(
        lock_file(&p, ".env.example"),
        format!("{}\n", std::process::id()),
    )
    .unwrap();
    evnx(&p, &["sync", "--prune", "--yes"])
        .assert_code(2)
        .assert_contains("another evnx");
    assert_eq!(p.read(".env.example"), "A=\nGONE=\n");
}

/// A lock whose process is gone is taken over, and removed afterwards.
#[cfg(unix)]
#[test]
fn e7_a_stale_lock_is_taken_over() {
    let p = Project::new().env("A=1\nB=2\n");
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    std::fs::write(lock_file(&p, ".env"), format!("{dead}\n")).unwrap();
    evnx(&p, &["remove", "A", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "B=2\n");
    assert!(!lock_file(&p, ".env").exists());
}

/// Several runs started together on one file: every one either removes its
/// name or stops with a clear message, the file always parses, and exactly the
/// names of the runs that reported success are gone. Never corruption, never a
/// success whose edit was lost.
#[test]
fn e7_concurrent_runs_one_wins_the_rest_abort_cleanly() {
    for round in 0..5 {
        let names: Vec<String> = (0..6).map(|i| format!("K{i}")).collect();
        let body: String = names.iter().map(|n| format!("{n}=v\n")).collect();
        let p = std::sync::Arc::new(Project::new().env(&body));
        let handles: Vec<_> = names
            .iter()
            .cloned()
            .map(|n| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let out = run(p.path(), &["remove", &n, "--yes", "--no-backup"]);
                    (n, out.code, out.all())
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let text = p.read(".env");
        for (n, code, all) in &results {
            let present = text.lines().any(|l| l.starts_with(&format!("{n}=")));
            match code {
                Some(0) => assert!(!present, "round {round}: {n} reported removed but is there"),
                Some(2) => {
                    assert!(
                        all.contains("another evnx") || all.contains("changed while evnx"),
                        "round {round}: unclear abort: {all}"
                    );
                    assert!(present, "round {round}: {n} aborted but is gone");
                }
                other => panic!("round {round}: {n} exited {other:?}: {all}"),
            }
        }
        // Whatever remains is whole lines of the original.
        for l in text.lines() {
            assert!(names.iter().any(|n| l == format!("{n}=v")), "{l:?}");
        }
        assert!(results.iter().any(|(_, c, _)| *c == Some(0)), "nobody won");
        assert!(!lock_file(&p, ".env").exists());
    }
}

// ── F-3 ─────────────────────────────────────────────────────────────────────

fn keys(text: &str) -> std::collections::BTreeSet<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
        .collect()
}

#[test]
fn f3_remove_then_add_custom_restores_the_key_set() {
    let p = Project::new().env("# app\nA=1\nB=2\nC=3\n");
    let original = keys(&p.read(".env"));
    evnx(&p, &["remove", "B", "--yes"]).assert_code(0);
    assert_ne!(keys(&p.read(".env")), original);
    evnx(&p, &["add", "custom", "B=2", "--yes"]).assert_code(0);
    assert_eq!(keys(&p.read(".env")), original);
    // And it parses as evnx reads it.
    let out = evnx(&p, &["parse", "--format", "json"]);
    out.assert_code(0);
    for k in &original {
        out.assert_contains(&format!("\"{k}\""));
    }
}

// ── G-2 ─────────────────────────────────────────────────────────────────────

/// Whether this filesystem keeps `.env.Staging` and `.env.staging` apart.
fn folds_case(p: &Project) -> bool {
    std::fs::write(p.join(".case-probe"), "x").unwrap();
    let folded = p.join(".CASE-PROBE").exists();
    let _ = std::fs::remove_file(p.join(".case-probe"));
    folded
}

#[test]
fn g2_case_colliding_env_files_are_reported_and_never_merged() {
    let p = Project::new().env("A=1\n");
    if folds_case(&p) {
        eprintln!("skipped: this filesystem folds case, so the two names cannot both exist");
        return;
    }
    std::fs::write(p.join(".env.staging"), "A=lower\nB=1\n").unwrap();
    std::fs::write(p.join(".env.Staging"), "A=upper\nC=1\n").unwrap();
    let out = evnx(&p, &["remove", "A", "--all", "--yes"]);
    out.assert_code(0)
        .assert_contains("differ only in case")
        .assert_contains(".env.staging")
        .assert_contains(".env.Staging");
    // Each was edited as itself.
    assert_eq!(p.read(".env.staging"), "B=1\n");
    assert_eq!(p.read(".env.Staging"), "C=1\n");
    evnx(&p, &["remove", "B", "C", "--check"])
        .assert_code(1)
        .assert_contains("differ only in case");
}

// ── G-4 ─────────────────────────────────────────────────────────────────────

/// A folder that cannot be written: a clean exit 2, the file as it was, and
/// nothing left behind. Skipped where the folder is writable anyway (root).
#[cfg(unix)]
#[test]
fn g4_a_read_only_folder_is_a_clean_error_with_no_partial_state() {
    use std::os::unix::fs::PermissionsExt;
    let p = Project::new().env("A=1\nB=2\n");
    let restore =
        |mode| std::fs::set_permissions(p.path(), std::fs::Permissions::from_mode(mode)).unwrap();
    restore(0o555);
    let writable = std::fs::write(p.join(".probe"), "x").is_ok();
    if writable {
        restore(0o755);
        eprintln!("skipped: running as a user that ignores directory permissions");
        return;
    }
    let out = evnx(&p, &["remove", "A", "--yes"]);
    restore(0o755);
    out.assert_code(2).assert_contains("No verdict");
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
    let mut left: Vec<String> = std::fs::read_dir(p.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, [".env"], "partial state: {left:?}");
    assert!(!out.all().contains("A=1"), "no value in the error");
}
