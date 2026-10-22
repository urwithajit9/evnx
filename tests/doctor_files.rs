//! `evnx doctor`: which files it reads, and whose permissions it checks
//! (R-doctor, 0.10.15).
//!
//! - I-173, DEC-D3: `.env` and `.env.<name>` are env files. `.envrc` (direnv),
//!   `.environment` and `.env-notes.md` only look like one; an unignored
//!   `.envrc` was a security error with exit 1.
//! - I-160, DEC-D5, DEC-D18: copies (`.env.backup`, `.bak`, `.old`, `.save`,
//!   `~`) are read. A plaintext copy is checked like the file it copies; an
//!   `evnx backup` file is ciphertext and skipped. `.env.backup` used to be
//!   skipped by name, so `cp .env .env.backup` was never checked.
//! - DEC-D19: `.env.yaml` and other structured files are env files for ignore
//!   and permissions, but are not dotenv, so no syntax check.
//! - I-158, I-170, DEC-D4: every env file's permissions, each named. Private
//!   means no group or other bits, so 700 passes; 640 does not.
//! - I-159, DEC-D6: a symlinked env file is seen ("symlink to …"), and `--fix`
//!   changes its target only inside the project.

mod support;

use support::{run, Project};

const IGNORE: &str = ".env*\n!.env.example\n!.env.sample\n!.env.template\n";

/// A project doctor has nothing to say about, but with no `.env` yet.
fn project() -> Project {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .example("A=\n")
        .file("package.json", "{}\n");
    p.track(".env.example");
    p
}

/// [`project`] with a private `.env`.
fn healthy() -> Project {
    let p = project().env("A=1\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// ── Which files count (I-173, DEC-D3, DEC-D19) ───────────────────────────────

#[test]
fn t_i173_envrc_and_lookalikes_are_not_env_files() {
    let p = healthy()
        .file(".gitignore", ".env\n")
        .file(".envrc", "layout python\nuse flake\n")
        .file(".environment", "notes\n")
        .file(".env-notes.md", "# notes\nsome prose\n");

    let out = run(p.path(), &["doctor", "--verbose"]);
    out.assert_code(0);
    for name in [".envrc", ".environment", ".env-notes.md"] {
        out.assert_not_contains(name);
    }
}

#[test]
fn t_dec_d19_structured_env_files_get_no_dotenv_syntax_check() {
    let p = healthy()
        .file(".env.json", "{\"a\": 1}\n")
        .file(".env.yaml", "a: 1\nb:\n  - c\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_not_contains("invalid syntax");
}

#[test]
fn t_dec_d19_a_structured_env_file_is_still_checked_for_ignore() {
    let p = healthy()
        .file(".gitignore", ".env\n")
        .file(".env.json", "{}\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env.json is NOT in .gitignore");
}

// ── Copies (I-160, DEC-D5, DEC-D18) ──────────────────────────────────────────

#[test]
fn t_i160_a_plaintext_env_backup_is_checked() {
    let p = healthy()
        .file(".gitignore", ".env\n")
        .file(".env.backup", "A=1\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env.backup is NOT in .gitignore");
    out.assert_contains("plaintext copy");
}

#[test]
fn t_dec_d18_every_plaintext_copy_is_an_error_until_ignored() {
    const COPIES: &[&str] = &[
        ".env.bak",
        ".env.old",
        ".env.save",
        ".env~",
        ".env.production.backup",
    ];
    let p = COPIES
        .iter()
        .fold(healthy().file(".gitignore", ".env\n"), |p, name| {
            p.file(name, "A=1\n")
        });

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    for name in COPIES {
        out.assert_contains(&format!("{name} is NOT in .gitignore"));
    }

    // `.env*` covers every one of them.
    run(p.path(), &["doctor", "--fix"]).assert_code(0);
    for name in COPIES {
        assert!(p.is_ignored(name), "{name}");
    }
}

/// The format `evnx backup` writes: Base64 of a version byte, then the rest.
/// Shaped by hand so the test runs without the `backup` feature.
#[test]
fn t_i160_an_evnx_backup_is_not_a_finding() {
    let mut raw = vec![1u8];
    raw.extend((0..200u8).map(|i| i.wrapping_mul(37)));
    use base64::{engine::general_purpose, Engine as _};
    let blob = general_purpose::STANDARD.encode(raw);

    let p = healthy()
        .file(".gitignore", ".env\n")
        .file(".env.backup", &blob);

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_not_contains(".env.backup");
}

/// The same, with a file `evnx backup` really wrote.
#[cfg(feature = "backup")]
#[test]
fn t_i160_a_real_evnx_backup_is_not_a_finding() {
    let p = healthy().file(".gitignore", ".env\n");
    support::run_with(
        p.path(),
        &["backup", ".env", "--output", ".env.backup"],
        &[("EVNX_PASSWORD", "correct horse battery staple")],
        None,
    )
    .assert_code(0);
    assert!(p.exists(".env.backup"));

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_not_contains(".env.backup");
}

// ── Permissions (I-158, I-170, DEC-D4) ───────────────────────────────────────

#[cfg(unix)]
#[test]
fn t_i158_every_env_file_is_permission_checked() {
    let p = healthy()
        .file(".env.production", "P=1\n")
        .mode(".env.production", 0o644)
        .mode(".env.example", 0o644);

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_contains(".env.production: permissions 644 (recommended: 600)");
    run(p.path(), &["doctor", "--strict"]).assert_code(1);

    let fixed = run(p.path(), &["doctor", "--fix"]);
    fixed.assert_code(0);
    assert!(
        fixed
            .stderr
            .contains("Fixed: .env.production mode 644 → 600"),
        "{}",
        fixed.stderr
    );
    assert_eq!(mode(&p.join(".env.production")), 0o600);
    assert_eq!(
        mode(&p.join(".env.example")),
        0o644,
        "a template is never touched"
    );
}

#[cfg(unix)]
#[test]
fn t_i158_a_plaintext_copy_is_permission_checked_too() {
    let p = healthy().file(".env.bak", "A=1\n").mode(".env.bak", 0o644);

    let out = run(p.path(), &["doctor"]);
    out.assert_contains(".env.bak: permissions 644");
    run(p.path(), &["doctor", "--fix"]).assert_code(0);
    assert_eq!(mode(&p.join(".env.bak")), 0o600);
}

#[cfg(unix)]
#[test]
fn t_i170_private_means_no_group_or_other_bits() {
    for (m, private) in [
        (0o600, true),
        (0o400, true),
        (0o700, true),
        (0o500, true),
        (0o640, false),
        (0o644, false),
        (0o660, false),
        (0o604, false),
        (0o777, false),
    ] {
        let p = healthy().mode(".env", m);
        let out = run(p.path(), &["doctor", "--strict"]);
        assert_eq!(
            out.code,
            Some(if private { 0 } else { 1 }),
            "mode {m:o}\n{}",
            out.all()
        );
    }
}

// ── Symlinks (I-159, DEC-D6) ─────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn t_i159_a_symlinked_env_is_seen_and_an_outside_target_left_alone() {
    let outside = tempfile::TempDir::new().unwrap();
    let target = outside.path().join("real.env");
    std::fs::write(&target, "S=1\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let p = project().symlink(target.to_str().unwrap(), ".env");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_not_contains(".env file not found");
    out.assert_contains("symlink to");

    let fixed = run(p.path(), &["doctor", "--fix"]);
    assert_eq!(mode(&target), 0o644, "outside the project: never changed");
    fixed.assert_contains(&format!("chmod 600 {}", target.display()));
    assert!(!fixed.stderr.contains("Fixed:"), "{}", fixed.stderr);
}

#[cfg(unix)]
#[test]
fn t_i159_a_target_inside_the_project_is_fixed() {
    let p = project()
        .file("config/real.env", "S=1\n")
        .mode("config/real.env", 0o644)
        .symlink("config/real.env", ".env");

    let fixed = run(p.path(), &["doctor", "--fix"]);
    fixed.assert_code(0);
    assert_eq!(mode(&p.join("config/real.env")), 0o600);
}

#[cfg(unix)]
#[test]
fn t_i159_a_symlinked_env_is_checked_for_syntax_and_ignore() {
    let p = project()
        .file("config/real.env", "not valid\n")
        .mode("config/real.env", 0o600)
        .symlink("config/real.env", ".env");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains("Line 1: invalid syntax");

    let p = project()
        .file(".gitignore", "node_modules/\n")
        .file("config/real.env", "S=1\n")
        .mode("config/real.env", 0o600)
        .symlink("config/real.env", ".env");
    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env is NOT in .gitignore");
}
