//! `evnx restore` through the binary (T-BR, package R-backup-restore).
//!
//! Each test makes its backup with the binary under test and restores it as a
//! script would: no terminal, no colour. The overwrite prompt itself is in
//! `backup_pty.rs`.
#![cfg(feature = "backup")]

mod support;

use support::{run, run_with, Project};

const ENV: &str = "DB_URL=postgres://user:pw@db.internal/app\nAPI_SECRET=sk_demo_000\n";
const PW: &str = "demo-passphrase-123\n";
const OLD: &str = "DB_URL=postgres://old\n";

/// A project holding `.env.backup` (of [`ENV`]) and the key file `pw`, with
/// no `.env` left behind.
fn backed_up() -> Project {
    let p = Project::new().env(ENV).file("pw", PW);
    run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    std::fs::remove_file(p.join(".env")).unwrap();
    p
}

fn restore(p: &Project, extra: &[&str]) -> support::Output {
    let mut args = vec!["restore", ".env.backup", "--key-file", "pw"];
    args.extend_from_slice(extra);
    run(p.path(), &args)
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

// ── I-148 / DEC-B1 / DEC-B11: replacing an existing output ──────────────────

/// I-148: with no terminal the only answer was dialoguer's "IO error: not a
/// terminal: not a terminal". Now one line names `--yes` and `--output`, and
/// the file is left alone.
#[test]
fn t_i148_an_existing_output_without_a_terminal_names_yes_and_output() {
    let p = backed_up().env(OLD);
    restore(&p, &[])
        .assert_code(2)
        .assert_contains("--yes")
        .assert_contains("--output")
        .assert_not_contains("IO error");
    assert_eq!(p.read(".env"), OLD);
    assert!(!p.exists(".env.bak"));
}

/// DEC-B1 + DEC-B11: `--yes` replaces the output and keeps what it held as
/// `.env.bak`, through the same helper `validate --fix` uses.
#[test]
fn t_i148_yes_replaces_and_keeps_the_previous_file() {
    let p = backed_up().env(OLD);
    restore(&p, &["--yes"])
        .assert_code(0)
        .assert_contains(".env.bak");
    assert_eq!(p.read(".env"), ENV);
    assert_eq!(p.read(".env.bak"), OLD);
}

/// `--force` is the spelling `cloud pull` uses; it is the same flag.
#[test]
fn t_i148_force_is_yes() {
    let p = backed_up().env(OLD);
    restore(&p, &["--force"]).assert_code(0);
    assert_eq!(p.read(".env"), ENV);
    assert_eq!(p.read(".env.bak"), OLD);
}

/// The copy keeps the replaced file's mode: a 0640 `.env` is not left
/// readable by more people as its copy, nor less (it is still theirs).
#[cfg(unix)]
#[test]
fn t_dec_b11_the_kept_copy_has_the_old_mode() {
    let p = backed_up().env(OLD).mode(".env", 0o640);
    restore(&p, &["-y"]).assert_code(0);
    assert_eq!(mode(&p, ".env.bak"), 0o640);
    assert_eq!(mode(&p, ".env"), 0o640, "an existing output keeps its mode");
}

/// A second replacement never overwrites the first copy.
#[test]
fn t_dec_b11_an_earlier_copy_is_never_overwritten() {
    let p = backed_up().env(OLD);
    restore(&p, &["--yes"]).assert_code(0);
    std::fs::write(p.join(".env"), "SECOND=1\n").unwrap();
    restore(&p, &["--yes"]).assert_code(0);
    assert_eq!(p.read(".env.bak"), OLD);
    assert_eq!(p.read(".env.bak.2"), "SECOND=1\n");
}

// ── DEC-B12 (S-B4): nothing to do ────────────────────────────────────────────

/// The output already holds exactly what the backup holds: no question, no
/// write, no copy, exit 0 — with or without a terminal.
#[test]
fn t_dec_b12_an_identical_output_is_already_up_to_date() {
    let p = backed_up().env(ENV);
    let before = std::fs::metadata(p.join(".env"))
        .unwrap()
        .modified()
        .unwrap();
    restore(&p, &[])
        .assert_code(0)
        .assert_contains("already up to date");
    assert!(!p.exists(".env.bak"));
    let after = std::fs::metadata(p.join(".env"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "the file was rewritten");
}

// ── DEC-B3: the output must not be the backup ───────────────────────────────

/// Restoring a backup over itself would replace the only copy of the
/// ciphertext with the plaintext. Refused before decrypting, even with --yes.
#[test]
fn t_dec_b3_output_equal_to_the_backup_is_refused() {
    let p = backed_up();
    let blob = p.read(".env.backup");
    for flags in [&[][..], &["--yes"][..]] {
        let mut extra = vec!["--output", ".env.backup"];
        extra.extend_from_slice(flags);
        restore(&p, &extra)
            .assert_code(2)
            .assert_contains("same file");
        assert_eq!(p.read(".env.backup"), blob, "{flags:?}: the backup changed");
    }
}

#[test]
fn t_dec_b3_a_directory_at_the_output_is_refused() {
    let p = backed_up().dir("adir");
    restore(&p, &["--output", "adir", "--yes"])
        .assert_code(2)
        .assert_contains("adir");
    assert!(p.join("adir").is_dir());
}

/// `--inspect` and `--dry-run` write nothing, so they do not care where
/// `--output` points.
#[test]
fn t_dec_b3_inspect_and_dry_run_ignore_the_output() {
    let p = backed_up();
    restore(&p, &["--output", ".env.backup", "--inspect"]).assert_code(0);
    restore(&p, &["--output", ".env.backup", "--dry-run"]).assert_code(0);
}

// ── I-149 / DEC-B6: say what is wrong when it is known ──────────────────────

fn decoded(p: &Project) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(p.read(".env.backup").trim())
        .unwrap()
}

fn encoded(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A newer format read as a wrong password, so the one thing that helps —
/// upgrading evnx — was never suggested.
#[test]
fn t_i149_a_newer_format_says_upgrade() {
    let p = backed_up();
    let mut raw = decoded(&p);
    raw[0] = 2;
    let p = p.file("v2.bak", &encoded(&raw));
    run(
        p.path(),
        &["restore", "v2.bak", "--key-file", "pw", "--inspect"],
    )
    .assert_code(2)
    .assert_contains("newer evnx")
    .assert_contains("format version 2")
    .assert_not_contains("Wrong password");
}

#[test]
fn t_i149_a_truncated_file_says_truncated() {
    let p = backed_up();
    let blob = p.read(".env.backup");
    // Cut on a Base64 boundary, and off one, so both decoders' paths are seen.
    let p = p
        .file("t100.bak", &blob[..100])
        .file("t101.bak", &blob[..101]);
    for f in ["t100.bak", "t101.bak"] {
        run(p.path(), &["restore", f, "--key-file", "pw", "--inspect"])
            .assert_code(2)
            .assert_contains("truncated")
            .assert_not_contains("Wrong password");
    }
}

#[test]
fn t_i149_a_file_that_is_not_a_backup_says_so() {
    let p = backed_up().file("garbage.bak", "this is not a backup!!\n");
    run(
        p.path(),
        &["restore", "garbage.bak", "--key-file", "pw", "--inspect"],
    )
    .assert_code(2)
    .assert_contains("not an evnx backup")
    .assert_not_contains("Wrong password");
}

/// A wrong password and a tampered file stay one message (DEC-B6): telling
/// them apart would say which part of the authentication failed.
#[test]
fn t_i149_tamper_and_a_wrong_password_stay_one_message() {
    let p = backed_up().file("bad", "wrong-passphrase-999\n");
    let mut raw = decoded(&p);
    raw[60] ^= 1;
    let p = p.file("flip.bak", &encoded(&raw));
    let wrong = run(
        p.path(),
        &["restore", ".env.backup", "--key-file", "bad", "--inspect"],
    );
    let flip = run(
        p.path(),
        &["restore", "flip.bak", "--key-file", "pw", "--inspect"],
    );
    wrong
        .assert_code(2)
        .assert_contains("Wrong password or corrupt backup");
    flip.assert_code(2)
        .assert_contains("Wrong password or corrupt backup");
}

// ── I-154 / DEC-B5: the fallback is not a success ───────────────────────────

/// I-154: "✓ Restored 0 variable(s) to x.restored" printed beside exit 5.
#[test]
fn t_i154_the_fallback_has_no_success_tick() {
    let p = Project::new()
        .file("prose.env", "This is a letter.\nIt has no assignments.\n")
        .file("pw", PW);
    run(
        p.path(),
        &[
            "backup",
            "prose.env",
            "--key-file",
            "pw",
            "--output",
            "prose.bak",
        ],
    )
    .assert_code(0);
    for quiet in [false, true] {
        let mut args = vec![
            "restore",
            "prose.bak",
            "--key-file",
            "pw",
            "--output",
            "out.env",
        ];
        if quiet {
            args.insert(0, "-q");
        }
        run(p.path(), &args)
            .assert_code(5)
            .assert_contains("out.env.restored")
            .assert_not_contains("✓ Restored");
        std::fs::remove_file(p.join("out.env.restored")).unwrap();
    }
}

// ── DEC-B4: no cloud:// in restore ───────────────────────────────────────────

/// `cloud://` was parsed and then refused with exit 6 "not yet available".
/// It is now an ordinary missing file, exit 3, with the command that does it.
#[test]
fn t_dec_b4_a_cloud_reference_is_not_found_and_points_at_cloud_pull() {
    let p = backed_up();
    run(
        p.path(),
        &["restore", "cloud://myproject", "--key-file", "pw"],
    )
    .assert_code(3)
    .assert_contains("cloud://myproject")
    .assert_contains("evnx cloud pull")
    .assert_not_contains("not yet available");
    assert!(!p.exists(".env"));
}

// ── I-155: both sources set ─────────────────────────────────────────────────

#[test]
fn t_i155_restore_says_the_key_file_won() {
    let p = backed_up();
    run_with(
        p.path(),
        &["restore", ".env.backup", "--password-file", "pw"],
        &[("EVNX_PASSWORD", "wrong-passphrase-999")],
        None,
    )
    .assert_code(0)
    .assert_contains("not used");
    assert_eq!(p.read(".env"), ENV);
}

// ── DEC-B2: restore opens what older versions made ──────────────────────────

/// DEC-B2: the 8-character floor applies to new backups only. A backup made
/// by 0.10.10 with a 3-character key file still opens.
#[test]
fn t_dec_b2_a_backup_made_with_a_short_secret_still_restores() {
    let golden = |n: &str| {
        std::fs::read_to_string(support::snapshot::golden_path(&format!("backup/{n}"))).unwrap()
    };
    let p = Project::new()
        .file("short.bak", &golden("golden-0.10.10-short-key.backup"))
        .file("short.key", &golden("golden-short.key"));
    run(
        p.path(),
        &[
            "restore",
            "short.bak",
            "--key-file",
            "short.key",
            "--output",
            "out.env",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read("out.env"), golden("golden.env"));
}

// ── The restored file and git (R-template's protect-output step) ────────────

/// The restored file holds plaintext. In a repository where it is not
/// ignored, `--gitignore` adds it; without a flag and a terminal, a warning.
#[test]
fn t_br_gitignore_restored_output_is_protected() {
    let p = backed_up().git();
    restore(&p, &["--output", "prod.env"])
        .assert_code(0)
        .assert_contains("'prod.env' may contain secrets and is not in .gitignore");
    assert!(!p.is_ignored("prod.env"));

    restore(&p, &["--output", "prod2.env", "--gitignore"]).assert_code(0);
    assert!(p.is_ignored("prod2.env"));

    restore(&p, &["--output", "prod3.env", "--no-gitignore"])
        .assert_code(0)
        .assert_not_contains("gitignore");
}

// ── Exit codes and help (T-BR-24, T-BR-43, T-BR-45, D-108) ──────────────────

#[test]
fn t_br24_missing_and_directory_backups_exit_3() {
    let p = backed_up().dir("adir");
    run(p.path(), &["restore", "nope.backup", "--key-file", "pw"]).assert_code(3);
    run(p.path(), &["restore", "adir", "--key-file", "pw"]).assert_code(3);
}

/// D-108, D-109: every argument has a description, and the overwrite rule and
/// exit codes are in `--help`, not only in the source.
#[test]
fn t_d108_help_describes_arguments_overwrite_and_exits() {
    let out = run(Project::new().path(), &["restore", "--help"]);
    out.assert_code(0)
        .assert_contains("The backup file to decrypt")
        .assert_contains("Where to write the restored file")
        .assert_contains("--yes")
        .assert_contains("4  ")
        .assert_contains("5  ")
        .assert_not_contains("cloud://")
        .assert_not_contains("1  Generic");
}

/// T-BR-23: a new restored file is 0600.
#[cfg(unix)]
#[test]
fn t_br23_a_new_restored_file_is_0600() {
    let p = backed_up();
    restore(&p, &["--output", "new.env"]).assert_code(0);
    assert_eq!(mode(&p, "new.env"), 0o600);
}
