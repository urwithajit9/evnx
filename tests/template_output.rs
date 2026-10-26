//! How `evnx template` writes its output (U2 part 2).
//!
//! The rendered file holds real values. It used to be written with `fs::write`:
//! 0644 (I-130), over the template itself when `--output` named it (I-131), and
//! through a symlink without a word (I-140). Now it goes through
//! `core::safe_write`:
//!
//! - a new output is 0600; `--mode` sets another on request (DEC-T6, DEC-G6);
//! - an existing output is replaced, keeps its mode and says `Replaced FILE`
//!   (DEC-T4), with a warning if others can read it (DEC-G6);
//! - the template and the env file are never the output, by any name (DEC-T4);
//! - a symlink is written through with a notice (DEC-T12);
//! - `--output -` prints the rendered text on stdout and nothing else (DEC-T8).

mod support;

use support::{run, Project};

fn project() -> Project {
    Project::new()
        .env("DB_HOST=db.internal\nDB_PASSWORD=s3cr3t\n")
        .file(
            "config.tpl",
            "host: {{DB_HOST}}\npassword: {{DB_PASSWORD}}\n",
        )
}

const RENDER: &[&str] = &[
    "template",
    "--input",
    "config.tpl",
    "--output",
    "out.yml",
    "--no-gitignore",
];

fn with<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = RENDER.to_vec();
    v.extend_from_slice(extra);
    v
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

#[test]
#[cfg(unix)]
fn t_i130_a_new_output_is_private() {
    let p = project();
    run(p.path(), RENDER).assert_code(0);
    assert_eq!(mode(&p, "out.yml"), 0o600);
    assert!(p.read("out.yml").contains("s3cr3t"));
}

#[test]
#[cfg(unix)]
fn t_dec_t6_mode_sets_the_mode_new_or_existing() {
    let p = project();
    run(p.path(), &with(&["--mode", "0640"])).assert_code(0);
    assert_eq!(mode(&p, "out.yml"), 0o640);
    std::fs::set_permissions(
        p.join("out.yml"),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    run(p.path(), &with(&["--mode", "644"])).assert_code(0);
    assert_eq!(mode(&p, "out.yml"), 0o644);
}

#[test]
fn t_dec_t6_a_bad_mode_is_refused() {
    let p = project();
    for bad in ["999", "rw-r--r--", "07777"] {
        run(p.path(), &with(&["--mode", bad])).assert_code(2);
    }
    assert!(!p.exists("out.yml"));
}

#[test]
#[cfg(unix)]
fn t_dec_t4_an_existing_output_is_replaced_and_keeps_its_mode() {
    let p = project().file("out.yml", "old\n").mode("out.yml", 0o644);
    let out = run(p.path(), RENDER);
    out.assert_code(0).assert_contains("Replaced out.yml");
    assert_eq!(mode(&p, "out.yml"), 0o644, "DEC-G6");
    assert!(
        out.stderr.contains("readable by other users"),
        "{}",
        out.stderr
    );
    assert!(p.read("out.yml").contains("db.internal"));
}

#[test]
fn t_i131_the_template_is_never_the_output() {
    let p = project();
    let before = p.read("config.tpl");
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "./config.tpl",
            "--no-gitignore",
        ],
    )
    .assert_code(2)
    .assert_contains("same file");
    assert_eq!(p.read("config.tpl"), before);
}

#[test]
fn t_i131_the_env_file_is_never_the_output() {
    let p = project();
    let before = p.read(".env");
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            ".env",
            "--no-gitignore",
        ],
    )
    .assert_code(2)
    .assert_contains("same file");
    assert_eq!(p.read(".env"), before);
}

#[test]
#[cfg(unix)]
fn t_i131_the_template_by_another_name_is_refused() {
    let p = project();
    std::os::unix::fs::symlink("config.tpl", p.join("link.yml")).unwrap();
    std::fs::hard_link(p.join("config.tpl"), p.join("hard.yml")).unwrap();
    let before = p.read("config.tpl");
    for out in ["link.yml", "hard.yml"] {
        run(
            p.path(),
            &[
                "template",
                "--input",
                "config.tpl",
                "--output",
                out,
                "--no-gitignore",
            ],
        )
        .assert_code(2)
        .assert_contains("same file");
    }
    assert_eq!(p.read("config.tpl"), before);
}

#[test]
#[cfg(unix)]
fn t_i140_a_symlinked_output_is_written_through() {
    let p = project().dir("deploy");
    std::fs::write(p.join("deploy/real.yml"), "old\n").unwrap();
    std::os::unix::fs::symlink("deploy/real.yml", p.join("out.yml")).unwrap();
    run(p.path(), RENDER)
        .assert_code(0)
        .assert_contains("wrote through out.yml -> deploy/real.yml");
    assert!(std::fs::symlink_metadata(p.join("out.yml"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(p.read("deploy/real.yml").contains("db.internal"));
}

#[test]
fn t_dec_t8_output_dash_prints_only_the_rendered_text() {
    let p = project();
    let out = run(
        p.path(),
        &["template", "--input", "config.tpl", "--output", "-"],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "host: db.internal\npassword: s3cr3t\n");
    assert!(!p.exists("-"), "no file named -");
}

const RENDER_GITIGNORE: &[&str] = &[
    "template",
    "--input",
    "config.tpl",
    "--output",
    "out.yml",
    "--gitignore",
];

/// I-139: `*.yml` already covers the output. The exact-line match did not see
/// that and added a redundant `out.yml` line.
#[test]
fn t_i139_a_wildcard_already_covers_the_output() {
    let p = project().git().file(".gitignore", "*.yml\n");
    run(p.path(), RENDER_GITIGNORE)
        .assert_code(0)
        .assert_contains("already ignored by git");
    assert_eq!(p.read(".gitignore"), "*.yml\n", "no redundant line");
}

/// I-139: a later `!out.yml` re-includes it. The exact-line match said "already
/// in .gitignore" and did nothing while git would commit the file. Now git is
/// asked; with `--gitignore` the entry is added after the negation, which wins
/// (last match), and the user's own line stays.
#[test]
fn t_i139_a_later_negation_is_seen_and_overridden_on_request() {
    let p = project().git().file(".gitignore", "out.yml\n!out.yml\n");
    run(p.path(), RENDER_GITIGNORE)
        .assert_code(0)
        .assert_contains("Added 'out.yml' to .gitignore");
    assert!(p.is_ignored("out.yml"), "git now ignores it");
    assert!(
        p.read(".gitignore").contains("!out.yml\n"),
        "the user's line stays"
    );
}

/// DEC-T11: a tracked output gets `git rm --cached`, not a .gitignore line.
#[test]
fn t_dec_t11_a_tracked_output_is_reported() {
    let p = project().git().file("out.yml", "old\n");
    p.track("out.yml");
    p.commit_all("x");
    let out = run(p.path(), RENDER_GITIGNORE);
    out.assert_code(0);
    assert!(out.stderr.contains("tracked by git"), "{}", out.stderr);
    assert!(
        out.stderr.contains("git rm --cached out.yml"),
        "{}",
        out.stderr
    );
    assert!(!p.exists(".gitignore") || !p.read(".gitignore").contains("out.yml"));
}

/// I-144 (O-12): the file was named `./.env.production` while `.env` had no
/// `./`, because `--env-name` joined the name onto `.`. Fixed in
/// `core::env_name`, so every command that takes `--env-name` agrees.
#[test]
fn t_i144_env_name_label_has_no_dot_slash() {
    let p = project().file(".env.production", "DB_HOST=prod.db\nDB_PASSWORD=p\n");
    let out = run(p.path(), &with(&["--env-name", "production"]));
    out.assert_code(0)
        .assert_contains("Loaded 2 variables from .env.production")
        .assert_not_contains("./.env.production");
}

/// I-144: a missing output directory was a clear error with no way forward.
/// It now names `--mkdir`, and still creates nothing without it.
#[test]
fn t_i144_a_missing_directory_names_mkdir() {
    let p = project();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "deploy/out.yml",
            "--no-gitignore",
        ],
    )
    .assert_code(2)
    .assert_contains("does not exist")
    .assert_contains("--mkdir");
    assert!(!p.exists("deploy"));
}

/// I-144: `--mkdir` creates the missing directories, owner-only like
/// `cloud export` (owner, 2026-10-17), and says so.
#[test]
fn t_i144_mkdir_creates_the_directories() {
    let p = project();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "deploy/prod/out.yml",
            "--mkdir",
            "--no-gitignore",
        ],
    )
    .assert_code(0)
    .assert_contains("deploy/prod");
    assert!(p.read("deploy/prod/out.yml").contains("db.internal"));
    #[cfg(unix)]
    {
        assert_eq!(mode(&p, "deploy/prod/out.yml"), 0o600);
        assert_eq!(mode(&p, "deploy/prod"), 0o700);
    }
}

fn dry(extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = with(&["--dry-run"]).iter().map(|s| s.to_string()).collect();
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

fn run_dry(p: &Project, extra: &[&str]) -> support::Output {
    let args = dry(extra);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(p.path(), &args)
}

/// DEC-T9: a dry run renders, reports, and writes nothing — not the output,
/// not `.gitignore` — and never prints a value.
#[test]
fn t_dec_t9_dry_run_writes_nothing_and_shows_no_value() {
    let p = project().git();
    let out = run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "out.yml",
            "--dry-run",
            "--gitignore",
        ],
    );
    out.assert_code(0)
        .assert_contains("Dry run")
        .assert_contains("would create out.yml")
        .assert_contains("nothing was written")
        .assert_not_contains("s3cr3t")
        .assert_not_contains("db.internal");
    assert!(!p.exists("out.yml"));
    assert!(!p.exists(".gitignore"), "a dry run leaves .gitignore alone");
}

#[test]
fn t_dec_t9_dry_run_says_an_existing_output_would_be_replaced() {
    let p = project().file("out.yml", "old\n");
    run_dry(&p, &[])
        .assert_code(0)
        .assert_contains("would replace out.yml");
    assert_eq!(p.read("out.yml"), "old\n");
}

/// Same exit rules as the real run (DEC-T2, error by default, is 0.11.0).
#[test]
fn t_dec_t9_dry_run_reports_unresolved_names_and_strict_fails() {
    let p = project().file("config.tpl", "host: {{DB_HOST}}\nurl: {{MISSING}}\n");
    run_dry(&p, &[]).assert_code(0).assert_contains("MISSING");
    run_dry(&p, &["--strict"])
        .assert_code(2)
        .assert_contains("MISSING");
    assert!(!p.exists("out.yml"));
}

#[test]
fn t_dec_t9_dry_run_fails_where_the_run_would() {
    let p = project().file("config.tpl", "x: {{DB_HOST|bogus}}\n");
    run_dry(&p, &[]).assert_code(2).assert_contains("bogus");
    let p = project();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "deploy/out.yml",
            "--dry-run",
            "--no-gitignore",
        ],
    )
    .assert_code(2)
    .assert_contains("--mkdir");
}

#[test]
fn t_dec_t9_dry_run_with_mkdir_creates_no_directory() {
    let p = project();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "deploy/out.yml",
            "--dry-run",
            "--mkdir",
            "--no-gitignore",
        ],
    )
    .assert_code(0)
    .assert_contains("would create the directory deploy");
    assert!(!p.exists("deploy"));
}

#[test]
fn t_dec_t9_dry_run_to_stdout_prints_nothing_there() {
    let p = project();
    let out = run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "-",
            "--dry-run",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "", "the rendered text is not printed");
    assert!(out.stderr.contains("would print"), "{}", out.stderr);
}

/// I-144: `--mkdir` made only the innermost directory 0700; a parent it
/// created kept the umask's mode, while `--help` promised 0700 for every one.
/// A directory that already existed is left as it is.
#[test]
#[cfg(unix)]
fn t_i144_mkdir_makes_every_directory_it_creates_private() {
    use std::os::unix::fs::PermissionsExt;
    let p = project().dir("existing");
    std::fs::set_permissions(p.join("existing"), std::fs::Permissions::from_mode(0o755)).unwrap();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "existing/a/b/out.yml",
            "--mkdir",
            "--no-gitignore",
        ],
    )
    .assert_code(0);
    assert_eq!(
        mode(&p, "existing"),
        0o755,
        "not created by evnx: untouched"
    );
    assert_eq!(mode(&p, "existing/a"), 0o700);
    assert_eq!(mode(&p, "existing/a/b"), 0o700);
}

/// I-252: an `--output` with `..` in it went into `.gitignore` as written
/// (`sub/../out.yml`), a line git matches to nothing, and evnx said "Added".
/// `../x` from the repository root got the same treatment instead of the
/// "outside the git repository" warning.
#[test]
fn t_i252_dot_dot_in_the_output_is_resolved_for_gitignore() {
    let p = project().git().dir("sub");
    run(
        p.path(),
        &[
            "template",
            "--input",
            "config.tpl",
            "--output",
            "sub/../out.yml",
            "--gitignore",
        ],
    )
    .assert_code(0)
    .assert_contains("Added 'out.yml'");
    assert!(p.is_ignored("out.yml"));

    // From a subdirectory, pointing back up into the repository.
    run(
        &p.join("sub"),
        &[
            "template",
            "--input",
            "../config.tpl",
            "--env",
            "../.env",
            "--output",
            "../out2.yml",
            "--gitignore",
        ],
    )
    .assert_code(0)
    .assert_contains("Added 'out2.yml'");
    assert!(p.is_ignored("out2.yml"));
    assert!(
        !p.read(".gitignore").contains(".."),
        "{}",
        p.read(".gitignore")
    );
}

/// RM-13: `--output -` is stdout. A refusal said "Nothing was written to -.",
/// which names no file; it now says what did not happen.
#[test]
fn rm13_a_refusal_to_stdout_says_nothing_was_printed() {
    for (tpl, extra) in [
        ("url: {{MISSING}}\n", &["--strict"][..]),
        ("x: {{DB_HOST|bogus}}\n", &[][..]),
    ] {
        let p = project().file("config.tpl", tpl);
        let mut args = vec!["template", "--input", "config.tpl", "--output", "-"];
        args.extend_from_slice(extra);
        let out = run(p.path(), &args);
        out.assert_code(2).assert_contains("Nothing was printed.");
        assert!(!out.stderr.contains("written to -"), "{}", out.stderr);
    }
}
