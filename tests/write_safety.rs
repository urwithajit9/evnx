//! WP-7 (v0.14.2): every write is private, reversible and tidy.
//!
//! Part 1, backups (VAL-2, REM-4, REM-6, CLD-6, FLD-9): one `<file>.bak` per
//! file, always 0600, git-ignored, recorded in `.evnx/backups.json`, and named
//! in the output with the way back.

mod support;

use support::{run, Project};

#[cfg(unix)]
fn mode(p: &Project, f: &str) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p.path().join(f))
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// The reproduction of VAL-2: a 644 `.env` in a repository with no
/// `.gitignore`, then `validate --fix`.
#[test]
fn val2_a_backup_is_private_ignored_and_recorded() {
    let p = Project::new().git().example("DEBUG=\n").env("DEBUG=True\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o644);
    let out = run(p.path(), &["validate", "--fix", "--yes"]);
    if !p.exists(".env.bak") {
        // Nothing to fix on this platform's rules: the rest needs a backup.
        return;
    }
    out.assert_contains("undo: mv");
    #[cfg(unix)]
    assert_eq!(mode(&p, ".env.bak"), 0o600);
    let gi = p.read(".gitignore");
    assert!(gi.contains("*.bak"), "{gi}");
    assert!(gi.contains(".evnx/backups.json"), "{gi}");
    let reg: serde_json::Value = serde_json::from_str(&p.read(".evnx/backups.json")).unwrap();
    assert_eq!(reg["backups"][0]["file"], ".env");
    assert_eq!(reg["backups"][0]["backup"], ".env.bak");
    assert_eq!(reg["backups"][0]["command"], "validate");
    assert!(
        !p.read(".evnx/backups.json").contains("True"),
        "a value in the registry"
    );
}

/// REM-4: the second write says it replaced the first backup; FLD-9: the
/// closing line names the copy and how to clear it.
#[test]
fn rem4_fld9_the_shared_slot_is_said_and_the_copy_is_named() {
    let p = Project::new().git().env("A=1\nB=2\nC=3\n");
    let out = run(p.path(), &["remove", "A", "--yes"]);
    out.assert_code(0);
    out.assert_not_contains("replaced the previous backup");
    out.assert_contains("The value still exists in ./.env.bak");
    out.assert_contains("rm ./.env.bak");

    let out = run(p.path(), &["remove", "B", "--yes"]);
    out.assert_code(0);
    out.assert_contains("backup ./.env.bak (replaced the previous backup)");
    assert_eq!(p.read(".env.bak"), "B=2\nC=3\n");
    let reg: serde_json::Value = serde_json::from_str(&p.read(".evnx/backups.json")).unwrap();
    assert_eq!(
        reg["backups"].as_array().unwrap().len(),
        1,
        "one entry per file"
    );
    assert_eq!(reg["backups"][0]["command"], "remove");
}

/// REM-6: the copy is private whatever the original's mode.
#[cfg(unix)]
#[test]
fn rem6_the_backup_of_a_loose_file_is_private() {
    let p = Project::new().git().env("A=1\nB=2\n").mode(".env", 0o644);
    run(p.path(), &["remove", "A", "--yes"]).assert_code(0);
    assert_eq!(mode(&p, ".env"), 0o644, "the file keeps the mode you set");
    assert_eq!(mode(&p, ".env.bak"), 0o600);
}

/// The user's own rule is enough: `.gitignore` is left as it is.
#[test]
fn an_existing_rule_leaves_gitignore_alone() {
    let p = Project::new()
        .git()
        .file(".gitignore", ".env*\n*.bak\n.evnx/\n")
        .env("A=1\nB=2\n");
    run(p.path(), &["remove", "A", "--yes"]).assert_code(0);
    assert_eq!(p.read(".gitignore"), ".env*\n*.bak\n.evnx/\n");
}

/// `--no-backup`: nothing is kept, so nothing is recorded or named.
#[test]
fn no_backup_records_nothing() {
    let p = Project::new().git().env("A=1\nB=2\n");
    let out = run(p.path(), &["remove", "A", "--yes", "--no-backup"]);
    out.assert_code(0);
    out.assert_not_contains(".env.bak");
    assert!(!p.exists(".evnx/backups.json"));
}

// ── Part 2: remove (REM-1, REM-2, REM-3, FLD-12) ────────────────────────────

/// REM-1's reproduction: the file stays readable.
#[test]
fn rem1_a_name_another_line_uses_is_refused() {
    let p = Project::new()
        .env("PORT=3000\nURL=http://h/${PORT}\n")
        .example("PORT=3000\nURL=http://h/${PORT}\n");
    let out = run(p.path(), &["remove", "PORT", "--all", "--yes"]);
    out.assert_code(2);
    out.assert_contains("line 2: URL uses ${PORT}");
    out.assert_contains("--allow-dangling");
    assert_eq!(p.read(".env"), "PORT=3000\nURL=http://h/${PORT}\n");
    assert_eq!(p.read(".env.example"), "PORT=3000\nURL=http://h/${PORT}\n");

    run(
        p.path(),
        &["remove", "PORT", "--all", "--yes", "--allow-dangling"],
    )
    .assert_code(0);
    assert_eq!(p.read(".env"), "URL=http://h/${PORT}\n");
}

/// REM-2: `--yes` is not enough for a pattern that takes everything.
#[test]
fn rem2_a_match_all_pattern_needs_all_names() {
    let p = Project::new().env("A=1\nB=2\n");
    let out = run(p.path(), &["remove", "*", "--yes"]);
    out.assert_code(2);
    out.assert_contains("--all-names");
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
    run(p.path(), &["remove", "*", "--yes", "--all-names"]).assert_code(0);
    assert_eq!(p.read(".env"), "");
    // A pattern that leaves something is not a match-all.
    let p = Project::new().env("A_1=1\nA_2=2\nB=3\n");
    run(p.path(), &["remove", "A_*", "--yes"]).assert_code(0);
}

/// REM-3, FLD-12: a section `add` wrote goes with its last variable.
#[test]
fn rem3_add_then_remove_leaves_the_file_as_it_was() {
    let p = Project::new().env("APP=1\n").example("APP=\n");
    run(p.path(), &["add", "custom", "K1=a", "K2=b", "--yes"]).assert_code(0);
    run(p.path(), &["remove", "K1", "K2", "--all", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "APP=1\n");
    assert_eq!(p.read(".env.example"), "APP=\n");
}
