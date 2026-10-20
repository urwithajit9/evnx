//! `evnx backup` through the binary (T-BR, package R-backup-restore).
//!
//! The review (`21-backup-restore.md` §5) found two process-level tests for
//! both commands, so the exit-code table, the password sources and every
//! rotation edge were pinned as enum values or not at all (T-030). These run
//! the built binary the way a script does: no terminal, no colour.
//!
//! Interactive prompts (the 8-character floor, the confirmation) are in
//! `backup_pty.rs`; restore is in `restore_integration.rs`.
#![cfg(feature = "backup")]

mod support;

use support::{run, run_with, Project};

const ENV: &str = "DB_URL=postgres://user:pw@db.internal/app\nAPI_SECRET=sk_demo_000\n";
const PW: &str = "demo-passphrase-123\n";

/// A project with a `.env` and a key file `pw`.
fn project() -> Project {
    Project::new().env(ENV).file("pw", PW)
}

#[cfg(unix)]
fn mode(p: &Project, name: &str) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p.join(name))
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

// ── I-146 / DEC-B3: the output must not be the source or a directory ─────────

/// I-146: `--output` equal to the source turned the `.env` into ciphertext;
/// the plaintext survived only as `src.env.1` (exit 0).
#[test]
fn t_i146_output_equal_to_the_source_is_refused() {
    let p = project().file("src.env", ENV);
    let out = run(
        p.path(),
        &[
            "backup",
            "src.env",
            "--key-file",
            "pw",
            "--output",
            "src.env",
        ],
    );
    out.assert_code(2).assert_contains("same file");
    assert_eq!(p.read("src.env"), ENV, "the source was changed");
    assert!(!p.exists("src.env.1"), "the source was rotated away");
}

/// The same file under another name is the same file: a symlink and a hard
/// link to the source are refused too.
#[cfg(unix)]
#[test]
fn t_i146_a_link_to_the_source_is_the_source() {
    let p = project()
        .file("src.env", ENV)
        .symlink("src.env", "link.env");
    std::fs::hard_link(p.join("src.env"), p.join("hard.env")).unwrap();
    for output in ["link.env", "hard.env", "./src.env"] {
        run(
            p.path(),
            &["backup", "src.env", "--key-file", "pw", "--output", output],
        )
        .assert_code(2);
        assert_eq!(p.read("src.env"), ENV, "{output}: the source was changed");
    }
}

/// Rotation renames `out.1` to `out.2`: when the source *is* `out.1`, the
/// backup would move the plaintext it is backing up into the rotation chain.
#[test]
fn t_i146_rotation_that_would_move_the_source_is_refused() {
    let p = project().file("out.1", ENV);
    run(
        p.path(),
        &["backup", "out.1", "--key-file", "pw", "--output", "out"],
    )
    .assert_code(2)
    .assert_contains("out.1");
    assert_eq!(p.read("out.1"), ENV);
    assert!(!p.exists("out.2"), "the source was rotated");
}

/// The write goes through a symlinked output (DEC-B7), so rotation has to
/// rotate the file the link names. It renamed the link itself to `.1`, so the
/// output's name stopped existing and the previous backup was overwritten.
#[cfg(unix)]
#[test]
fn t_i146_rotation_through_a_symlink_rotates_the_target() {
    let p = project()
        .dir("vault")
        .symlink("vault/app.backup", ".env.backup");
    run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    std::fs::write(p.join(".env"), "SECOND=1\n").unwrap();
    run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);

    let link = std::fs::symlink_metadata(p.join(".env.backup")).unwrap();
    assert!(link.file_type().is_symlink(), "the link was replaced");
    assert!(!p.exists(".env.backup.1"), "the link was rotated");
    assert_eq!(restored(&p, ".env.backup"), "SECOND=1\n");
    assert_eq!(restored(&p, "vault/app.backup.1"), ENV);
}

/// DEC-B3: a directory at the output is refused with exit 2 — "could not
/// run" — and nothing is renamed. 0.10.3 refused it at the write, exit 5.
#[test]
fn t_dec_b3_a_directory_at_the_output_is_exit_2() {
    let p = project().dir("odir");
    run(
        p.path(),
        &["backup", "--key-file", "pw", "--output", "odir"],
    )
    .assert_code(2)
    .assert_contains("odir");
    assert!(p.join("odir").is_dir());
    assert!(!p.exists("odir.1"), "the directory was rotated");
}

/// A directory in a rotation slot would be renamed in the shift: refused
/// before anything moves.
#[test]
fn t_dec_b3_a_directory_in_a_rotation_slot_is_refused() {
    let p = project()
        .file(".env.backup", "previous backup\n")
        .dir(".env.backup.1");
    run(p.path(), &["backup", "--key-file", "pw"])
        .assert_code(2)
        .assert_contains(".env.backup.1");
    assert_eq!(p.read(".env.backup"), "previous backup\n");
    assert!(p.join(".env.backup.1").is_dir());
    assert!(!p.exists(".env.backup.2"));
}

// ── I-147 / DEC-B2: one floor for every source of a new backup ──────────────

#[test]
fn t_i147_a_short_key_file_is_refused() {
    let p = project().file("short", "abc\n");
    run(
        p.path(),
        &["backup", "--key-file", "short", "--output", "s.bak"],
    )
    .assert_code(2)
    .assert_contains("at least 8 characters (got 3)");
    assert!(!p.exists("s.bak"));
}

#[test]
fn t_i147_a_short_evnx_password_is_refused() {
    let p = project();
    run_with(
        p.path(),
        &["backup", "--output", "s.bak"],
        &[("EVNX_PASSWORD", "abc")],
        None,
    )
    .assert_code(2)
    .assert_contains("EVNX_PASSWORD")
    .assert_contains("at least 8 characters (got 3)");
    assert!(!p.exists("s.bak"));
}

/// Characters, not bytes: eight characters of a non-ASCII passphrase pass,
/// as the cloud master password's floor counts them.
#[test]
fn t_i147_the_floor_counts_characters() {
    let p = project().file("pw8", "äöüßäöüß\n");
    run(
        p.path(),
        &["backup", "--key-file", "pw8", "--output", "u.bak"],
    )
    .assert_code(0);
}

// ── I-155: both sources set ────────────────────────────────────────────────

/// I-155: the key file won and nothing said so. Now a warning names the
/// variable that was not used — and the backup still opens with the file.
#[test]
fn t_i155_key_file_and_evnx_password_both_set_says_which_wins() {
    let p = project();
    let out = run_with(
        p.path(),
        &["backup", "--key-file", "pw", "--output", "b.bak"],
        &[("EVNX_PASSWORD", "another-passphrase-456")],
        None,
    );
    out.assert_code(0)
        .assert_contains("EVNX_PASSWORD")
        .assert_contains("not used");
    run(
        p.path(),
        &["restore", "b.bak", "--key-file", "pw", "--output", "b.env"],
    )
    .assert_code(0);
    assert_eq!(p.read("b.env"), ENV);
}

// ── No terminal, no secret (T-BR-07) ─────────────────────────────────────────

/// "IO error: not a terminal: not a terminal" said nothing about what to do.
/// The line now names the two ways to give the passphrase without a prompt.
#[test]
fn t_br07_no_terminal_and_no_secret_names_the_ways_in() {
    let p = project();
    run(p.path(), &["backup"])
        .assert_code(2)
        .assert_contains("--key-file")
        .assert_contains("EVNX_PASSWORD")
        .assert_not_contains("IO error");
    assert!(!p.exists(".env.backup"));
}

#[test]
fn t_br09_an_empty_key_file_and_an_empty_evnx_password_are_refused() {
    let p = project().file("empty", "  \n");
    run(p.path(), &["backup", "--key-file", "empty"])
        .assert_code(2)
        .assert_contains("empty");
    run(p.path(), &["backup", "--key-file", "missing"])
        .assert_code(2)
        .assert_contains("not found");
    run_with(p.path(), &["backup"], &[("EVNX_PASSWORD", "")], None)
        .assert_code(2)
        .assert_contains("EVNX_PASSWORD is set but empty");
    assert!(!p.exists(".env.backup"));
}

// ── Exit codes through the process (T-BR-43, DEC-B9) ────────────────────────

#[test]
fn t_br43_source_problems_exit_2() {
    let p = Project::new().file("pw", PW).dir("adir");
    run(p.path(), &["backup", "--key-file", "pw"])
        .assert_code(2)
        .assert_contains("File not found");
    run(p.path(), &["backup", "adir", "--key-file", "pw"])
        .assert_code(2)
        .assert_contains("Not a regular file");
}

/// A missing output directory is a write failure, exit 5, named plainly
/// (I-152, fixed in 0.10.3).
#[test]
fn t_br43_a_missing_output_directory_is_exit_5() {
    let p = project();
    run(
        p.path(),
        &["backup", "--key-file", "pw", "--output", "nodir/x.bak"],
    )
    .assert_code(5)
    .assert_contains("nodir")
    .assert_not_contains("symlink");
}

/// The help no longer promises an exit 1 that is never produced (DEC-B9).
#[test]
fn t_dec_b9_help_lists_the_codes_backup_uses() {
    let out = run(Project::new().path(), &["backup", "--help"]);
    out.assert_code(0)
        .assert_contains("2  Could not run")
        .assert_contains("6  ")
        .assert_not_contains("1  Generic");
}

// ── Rotation (T-BR-13, T-BR-14, T-BR-15) ─────────────────────────────────────

fn restored(p: &Project, backup: &str) -> String {
    run(
        p.path(),
        &[
            "restore",
            backup,
            "--key-file",
            "pw",
            "--output",
            "check.env",
        ],
    )
    .assert_code(0);
    let s = p.read("check.env");
    std::fs::remove_file(p.join("check.env")).unwrap();
    s
}

#[test]
fn t_br13_rotation_keeps_the_newest_first() {
    let p = project();
    for i in 1..=5 {
        std::fs::write(p.join(".env"), format!("N={i}\n")).unwrap();
        run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    }
    assert_eq!(restored(&p, ".env.backup"), "N=5\n");
    assert_eq!(restored(&p, ".env.backup.1"), "N=4\n");
    assert_eq!(restored(&p, ".env.backup.2"), "N=3\n");
    assert_eq!(restored(&p, ".env.backup.3"), "N=2\n");
    assert!(!p.exists(".env.backup.4"));
}

#[test]
fn t_br14_keep_0_overwrites_and_keep_1_warns() {
    let p = project();
    run(p.path(), &["backup", "--key-file", "pw", "--keep", "0"]).assert_code(0);
    run(p.path(), &["backup", "--key-file", "pw", "--keep", "0"]).assert_code(0);
    assert!(!p.exists(".env.backup.1"), "--keep 0 rotated");

    run(p.path(), &["backup", "--key-file", "pw", "--keep", "1"]).assert_code(0);
    assert!(p.exists(".env.backup.1"));
    run(p.path(), &["backup", "--key-file", "pw", "--keep", "1"])
        .assert_code(0)
        .assert_contains("will be overwritten by rotation");
    assert!(!p.exists(".env.backup.2"));
}

#[test]
fn t_br15_backup_keep_in_config_and_the_flag_wins() {
    let p = project().toml("[backup]\nkeep = 1\n");
    for _ in 0..3 {
        run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    }
    assert!(p.exists(".env.backup.1") && !p.exists(".env.backup.2"));

    for _ in 0..2 {
        run(p.path(), &["backup", "--key-file", "pw", "--keep", "2"]).assert_code(0);
    }
    assert!(
        p.exists(".env.backup.2"),
        "--keep 2 did not win over keep = 1"
    );
}

// ── Output file (T-BR-01, T-031) ─────────────────────────────────────────────

/// T-031: the old unit test checked `write_private` on a scratch file, so it
/// would pass with backup writing some other way. This checks the backup, and
/// that it holds no value or name in clear (T-BR-42).
#[cfg(unix)]
#[test]
fn t_br01_the_backup_is_0600_and_holds_nothing_in_clear() {
    let p = project();
    let out = run(p.path(), &["backup", "--key-file", "pw"]);
    out.assert_code(0)
        .assert_not_contains("sk_demo_000")
        .assert_not_contains("postgres://");
    assert_eq!(mode(&p, ".env.backup"), 0o600);
    let blob = p.read(".env.backup");
    for clear in ["sk_demo_000", "DB_URL", "API_SECRET", ".env"] {
        assert!(!blob.contains(clear), "{clear} in clear");
    }
}

/// T-036: the README promises `--env-name` for backup.
#[test]
fn t_br03_env_name_picks_the_file() {
    let p = project().file(".env.production", "P=1\n");
    run(
        p.path(),
        &["backup", "--env-name", "production", "--key-file", "pw"],
    )
    .assert_code(0);
    assert!(p.exists(".env.production.backup"));
    assert_eq!(restored(&p, ".env.production.backup"), "P=1\n");
}

// ── I-272: copies are not environments ───────────────────────────────────────

/// I-272: `validate --fix` leaves `.env.bak`, `restore --yes` leaves
/// `.env.bak` too, and a fallback restore writes `.env.restored`. The
/// "Available" list named them as environments ("Available: 1, bak").
#[test]
fn t_i272_copies_are_not_listed_as_environments() {
    let p = project()
        .file(".env.production", "P=1\n")
        .file(".env.bak", "A=1\n")
        .file(".env.bak.2", "A=1\n")
        .file(".env.1", "A=1\n")
        .file(".env.restored", "A=1\n")
        .file(".env.production.bak", "A=1\n");
    let out = run(
        p.path(),
        &["backup", "--env-name", "nope", "--key-file", "pw"],
    );
    out.assert_code(2)
        .assert_contains("Available: production\n");
}

// ── Docs drift (T-BR-44, D-99, D-107) ────────────────────────────────────────

/// Every flag the README's backup/restore examples use exists in that
/// command's `--help`. The docs taught `backup --env .env.production`, a flag
/// that never existed (D-99, D-107), and the README's section was two lines.
#[test]
fn t_br44_readme_examples_use_flags_that_exist() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("### `evnx backup` / `evnx restore`")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .expect("README has a backup/restore section");

    let help = |cmd: &str| run(Project::new().path(), &[cmd, "--help"]).stdout;
    let (backup_help, restore_help) = (help("backup"), help("restore"));
    let mut in_code = false;
    let mut checked = 0;
    for line in section.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        let code = line.split(" #").next().unwrap();
        let help = if !in_code {
            continue;
        } else if code.contains("evnx backup") {
            &backup_help
        } else if code.contains("evnx restore") {
            &restore_help
        } else {
            continue;
        };
        for flag in code.split_whitespace().filter(|w| w.starts_with("--")) {
            let flag = flag.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
            assert!(
                help.contains(flag),
                "README uses {flag}, not in --help: {line}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 10, "only {checked} flags found");
    assert!(
        !section.contains("--env ") && !section.contains("Argon2 "),
        "D-99, D-111"
    );
}
