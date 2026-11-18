//! WP-7 (v0.14.2): every write is private, reversible and tidy.
//!
//! Part 1, backups (VAL-2, REM-4, REM-6, CLD-6, FLD-9): one `<file>.bak` per
//! file, always 0600, git-ignored, recorded in `.evnx/backups.json`, and named
//! in the output with the way back.

mod support;

use support::{Project, run};

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
    out.assert_contains("undo: evnx undo .env");
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
    out.assert_contains("evnx undo --clear");

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

// ── Part 3: add (ADD-3, FLD-12) ─────────────────────────────────────────────

/// Two identical runs write the same bytes, and the second `add custom`
/// continues the section the first wrote.
#[test]
fn add3_add_custom_is_reproducible_and_reuses_its_section() {
    let a = Project::new().env("APP=1\n").example("APP=\n");
    let b = Project::new().env("APP=1\n").example("APP=\n");
    for p in [&a, &b] {
        run(p.path(), &["add", "custom", "K1=a", "--yes"]).assert_code(0);
        run(p.path(), &["add", "custom", "K2=b", "--yes"]).assert_code(0);
    }
    assert_eq!(a.read(".env"), b.read(".env"));
    assert_eq!(
        a.read(".env"),
        "APP=1\n\n# ── Custom Variables ──\nK1=a\nK2=b\n"
    );
    assert!(!a.read(".env.example").contains("Added by"));
    assert_eq!(
        a.read(".env.example").matches("Custom Variables").count(),
        1
    );
}

/// ADD-3: on a server, only the env file.
#[test]
fn add3_no_template_writes_only_the_env_file() {
    let p = Project::new().file(".env.prod", "A=1\n");
    let out = run(
        p.path(),
        &[
            "add",
            "custom",
            "K=v",
            "--env-name",
            "prod",
            "--no-template",
            "--yes",
        ],
    );
    out.assert_code(0);
    assert!(p.read(".env.prod").contains("K=v"));
    assert!(!p.exists(".env.example"));
    assert!(!p.exists(".env.prod.example"));
    // Already there: nothing to add.
    run(
        p.path(),
        &[
            "add",
            "custom",
            "K=v",
            "--env-name",
            "prod",
            "--no-template",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read(".env.prod").matches("K=v").count(), 1);
}

// ── Part 4: template (TPL-1, TPL-3) ─────────────────────────────────────────

fn rendered() -> Project {
    Project::new()
        .env("PORT=3000\n")
        .file("t.tpl", "port={{PORT}}\n")
}

/// TPL-1: an existing output is replaced only with `--force`.
#[test]
fn tpl1_an_existing_output_needs_force() {
    let p = rendered().file("out.conf", "precious\n");
    let out = run(
        p.path(),
        &["template", "--input", "t.tpl", "--output", "out.conf"],
    );
    out.assert_code(2);
    out.assert_contains("out.conf already exists, so nothing was written");
    out.assert_contains("--force");
    out.assert_contains("evnx stopped before making changes");
    assert_eq!(p.read("out.conf"), "precious\n");
    // The dry run reports the same refusal.
    run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "out.conf",
            "--dry-run",
        ],
    )
    .assert_code(2);
    run(
        p.path(),
        &[
            "template", "--input", "t.tpl", "--output", "out.conf", "--force",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read("out.conf"), "port=3000\n");
    // A new output needs nothing.
    run(
        p.path(),
        &["template", "--input", "t.tpl", "--output", "new.conf"],
    )
    .assert_code(0);
}

/// TPL-3: a link to outside the folder is refused; inside, it is followed.
#[cfg(unix)]
#[test]
fn tpl3_a_link_to_outside_the_folder_is_refused() {
    let p = rendered();
    let outside = tempfile::TempDir::new().unwrap();
    let target = outside.path().join("elsewhere.conf");
    std::os::unix::fs::symlink(&target, p.path().join("link.conf")).unwrap();
    let out = run(
        p.path(),
        &["template", "--input", "t.tpl", "--output", "link.conf"],
    );
    out.assert_code(2);
    out.assert_contains("outside this folder");
    assert!(!target.exists());
    run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "link.conf",
            "--follow-symlinks",
        ],
    )
    .assert_code(0);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "port=3000\n");

    // Inside the folder: written through, as before.
    std::fs::create_dir(p.path().join("conf")).unwrap();
    std::os::unix::fs::symlink("conf/real.conf", p.path().join("inside.conf")).unwrap();
    run(
        p.path(),
        &["template", "--input", "t.tpl", "--output", "inside.conf"],
    )
    .assert_code(0);
    assert_eq!(p.read("conf/real.conf"), "port=3000\n");
}

// ── Part 5: convert --to yaml (CONV-1) ──────────────────────────────────────

#[test]
fn conv1_yaml_values_are_quoted_strings() {
    let p = Project::new().env("A=yes\nE=on\nI=No\nJ=2026-01-01\nB=true\nK=123\n");
    let out = run(p.path(), &["convert", "--to", "yaml"]);
    out.assert_code(0);
    for line in [
        "A: \"yes\"",
        "E: \"on\"",
        "I: \"No\"",
        "J: \"2026-01-01\"",
        "K: \"123\"",
    ] {
        assert!(
            out.stdout.lines().any(|l| l == line),
            "{line}:\n{}",
            out.stdout
        );
    }
}

// ── Part 6: doctor --fix (WP-7) ─────────────────────────────────────────────

#[test]
fn doctor_fix_clears_empty_sections_older_runs_left() {
    let p = Project::new()
        .git()
        .env("A=1\n\n# ── Stripe ──\n\n# Added by evnx add on 2026-11-01 10:00\n")
        .example("A=\n\n# ── Stripe ──\n");
    let out = run(p.path(), &["doctor"]);
    out.assert_contains("empty section");
    let out = run(p.path(), &["doctor", "--fix"]);
    out.assert_contains("removed 1 empty section");
    assert_eq!(p.read(".env"), "A=1\n");
    assert_eq!(p.read(".env.example"), "A=\n");
    run(p.path(), &["doctor"]).assert_not_contains("empty section");
}

// ── Part 7: evnx history and evnx undo (owner, 2026-11-16) ───────────────────

/// `undo` swaps: once puts the old text back, twice the newer one.
#[test]
fn undo_swaps_the_file_and_its_copy() {
    let p = Project::new().git().env("A=1\nB=2\n");
    run(p.path(), &["remove", "B", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n");

    let out = run(p.path(), &["history"]);
    out.assert_code(0);
    out.assert_contains(".env");
    out.assert_contains("remove");

    let out = run(p.path(), &["undo", ".env", "--yes"]);
    out.assert_code(0);
    out.assert_contains("+ B  back");
    out.assert_not_contains("B=2");
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
    assert_eq!(p.read(".env.bak"), "A=1\n");

    // No FILE: the only one recorded. Again: the newer text.
    run(p.path(), &["undo", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n");
}

#[test]
fn undo_asks_shows_and_dry_runs() {
    let p = Project::new().git().env("A=1\nB=2\n");
    run(p.path(), &["remove", "B", "--yes"]).assert_code(0);
    // No terminal and no --yes: refused (exit 4, D3), nothing changed.
    let out = run(p.path(), &["undo", ".env"]);
    out.assert_code(4);
    out.assert_contains("--yes");
    let out = run(p.path(), &["undo", ".env", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains("+ B  back");
    assert_eq!(p.read(".env"), "A=1\n");
}

#[test]
fn undo_clear_deletes_the_copies_and_history_forgets_them() {
    let p = Project::new().git().env("A=1\nB=2\n");
    run(p.path(), &["remove", "B", "--yes"]).assert_code(0);
    run(p.path(), &["undo", "--clear"]).assert_code(4);
    assert!(p.exists(".env.bak"));
    run(p.path(), &["undo", "--clear", "--yes"]).assert_code(0);
    assert!(!p.exists(".env.bak"));
    let out = run(p.path(), &["history", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["backups"].as_array().unwrap().len(), 0);
    run(p.path(), &["undo", ".env", "--yes"])
        .assert_code(2)
        .assert_contains("no copy of .env");
}

/// `restore` stays for `evnx backup` archives.
#[test]
fn undo_is_not_restore() {
    let p = Project::new().git();
    run(p.path(), &["undo", "--help"])
        .assert_code(0)
        .assert_contains("evnx restore");
}

/// `-q`: the swap happens and nothing is printed.
#[test]
fn undo_is_quiet_under_q() {
    let p = Project::new().git().env("A=1\nB=2\n");
    run(p.path(), &["remove", "B", "--yes"]).assert_code(0);
    let out = run(p.path(), &["-q", "undo", ".env", "--yes"]);
    out.assert_code(0);
    assert_eq!(out.stdout, "", "{}", out.stderr);
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
}
