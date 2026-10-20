//! What `evnx migrate` prints is a command the destination's own CLI accepts
//! (R-migrate).
//!
//! The quoting tests (`migrate_quoting.rs`) prove what a stub tool receives;
//! they cannot tell whether the real tool takes that shape. Two of nine did
//! not, checked on 2026-10-20 against each tool's latest release (review note
//! `local/migrate/review/18a-target-status.md`):
//!
//! - I-268: `vercel env add … --project-id P` — vercel has no `--project-id`
//!   (64.7.0, `env/command.ts`: `--project`), and an existing variable is an
//!   error without `--force`.
//! - I-269: `railway --project P variables set …` — the root `railway` command
//!   takes no arguments (v5.71.0); with `--project`, `--environment` is
//!   required (DEC-MG1).
//!
//! And three printed what the tool would misread:
//!
//! - I-100: gcp printed `echo -n "${KEY}"`, a shell variable the user had to
//!   export, created only (a re-run failed), and dropped `--project`
//!   (DEC-MG2: required, as in convert).
//! - I-270: infisical reads a value starting with `@` as a file path under
//!   `infisical login`, and rejects an empty value (DEC-MG4: refused).
//! - I-267: `--add-prefix` / `--strip-prefix` could put a `;` into a gcp
//!   command, or leave a name empty.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use support::{run, run_with, Project};

fn migrate(p: &Project, args: &[&str]) -> support::Output {
    let mut full = vec!["migrate", "--from", "env-file"];
    full.extend_from_slice(args);
    run(p.path(), &full)
}

/// The lines of `stdout` that start with `prefix`.
fn lines_starting<'a>(stdout: &'a str, prefix: &str) -> Vec<&'a str> {
    stdout.lines().filter(|l| l.starts_with(prefix)).collect()
}

// ── vercel (I-268) ──────────────────────────────────────────────────────────

#[test]
fn t_i268_vercel_names_the_project_with_project() {
    let p = Project::new().env("A=1\nB=two words\n");
    let out = migrate(&p, &["--to", "vercel", "--vercel-project", "prj_1"]);
    out.assert_code(0);
    assert_eq!(
        lines_starting(&out.stdout, "printf "),
        [
            "printf '%s' '1' | vercel env add A production --project prj_1 --force --yes",
            "printf '%s' 'two words' | vercel env add B production --project prj_1 --force --yes",
        ],
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("--project-id"), "{}", out.stdout);
}

#[test]
fn t_i268_vercel_without_a_project_uses_the_linked_one() {
    let p = Project::new().env("A=1\n");
    let out = migrate(&p, &["--to", "vercel"]);
    out.assert_code(0);
    assert_eq!(
        lines_starting(&out.stdout, "printf "),
        ["printf '%s' '1' | vercel env add A production --force --yes"],
        "{}",
        out.stdout
    );
}

// ── railway (I-269, DEC-MG1) ────────────────────────────────────────────────

#[test]
fn t_i269_railway_prints_variable_set_with_project_and_environment() {
    let p = Project::new().env("A=1\nB=2\n");
    let out = migrate(
        &p,
        &[
            "--to",
            "railway",
            "--railway-project",
            "p-1",
            "--railway-environment",
            "production",
            "--railway-service",
            "web",
        ],
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains(
            "railway variable set --project p-1 --environment production --service web \\\n  A='1' \\\n  B='2'\n"
        ),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("railway --project"), "{}", out.stdout);
}

#[test]
fn t_i269_railway_without_a_project_is_for_a_linked_directory() {
    let p = Project::new().env("A=1\n");
    let out = migrate(&p, &["--to", "railway"]);
    out.assert_code(0);
    assert!(
        out.stdout.contains("railway variable set \\\n  A='1'\n"),
        "{}",
        out.stdout
    );
}

/// Railway: "--environment is required when using --project".
#[test]
fn t_i269_railway_project_needs_an_environment() {
    let p = Project::new().env("A=1\n");
    for extra in [&[][..], &["--dry-run"][..]] {
        let mut args = vec!["--to", "railway", "--railway-project", "p-1"];
        args.extend_from_slice(extra);
        let out = migrate(&p, &args);
        out.assert_code(2);
        assert!(
            out.stderr.contains("--railway-environment"),
            "{}",
            out.stderr
        );
        assert!(!out.stdout.contains("variable set"), "{}", out.stdout);
    }
}

// ── gcp (I-100, DEC-MG2) ────────────────────────────────────────────────────

#[test]
fn t_i100_gcp_requires_project_like_convert() {
    let p = Project::new().env("A=1\n");
    for extra in [&[][..], &["--dry-run"][..]] {
        let mut args = vec!["--to", "gcp-secret-manager"];
        args.extend_from_slice(extra);
        let out = migrate(&p, &args);
        out.assert_code(2);
        assert!(out.stderr.contains("--project"), "{}", out.stderr);
        assert!(!out.stdout.contains("gcloud"), "{}", out.stdout);
    }
}

/// The value is in the printed command (not a shell variable the user had to
/// export), reaches gcloud on stdin exactly, and a secret that exists gets a
/// new version instead of an error.
#[test]
fn t_i100_gcp_prints_each_value_and_creates_or_adds_a_version() {
    let p = Project::new().env("PRICE='$5 off'\nAPOS=\"it's\"\nNEW=fresh\n");
    let bin = p.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // `describe` succeeds for PRICE and APOS only: they exist already.
    support::stub::write_stub(
        &bin.join("gcloud"),
        "#!/bin/sh\n\
         if [ \"$2\" = describe ]; then case \"$3\" in PRICE|APOS) exit 0;; *) exit 1;; esac; fi\n\
         in=$(cat)\n\
         printf '%s|%s|%s\\n' \"$*\" \"$in\" >> \"$STUB_LOG\"\n",
    );
    let out = migrate(
        &p,
        &[
            "--to",
            "gcp-secret-manager",
            "--project",
            "my-proj",
            "--print-only",
        ],
    );
    out.assert_code(0);
    assert!(!out.stdout.contains("${"), "{}", out.stdout);
    std::fs::write(p.path().join("run.sh"), &out.stdout).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let log = p.path().join("stub.log");
    let sh = std::process::Command::new("sh")
        .arg("run.sh")
        .current_dir(p.path())
        .env("PATH", path)
        .env("STUB_LOG", &log)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        sh.status.success(),
        "{}",
        String::from_utf8_lossy(&sh.stderr)
    );
    let log = std::fs::read_to_string(&log).unwrap();
    for expected in [
        "secrets versions add PRICE --data-file=- --project=my-proj|$5 off|",
        "secrets versions add APOS --data-file=- --project=my-proj|it's|",
        "secrets create NEW --data-file=- --project=my-proj|fresh|",
    ] {
        assert!(log.contains(expected), "missing {expected:?} in\n{log}");
    }
}

// ── infisical (I-270, DEC-MG4) ──────────────────────────────────────────────

#[test]
fn t_i270_infisical_refuses_values_it_would_misread() {
    let p = Project::new().env("KEY=@/etc/passwd\nEMPTY=\nOK=1\n");
    for extra in [&[][..], &["--dry-run"][..]] {
        let mut args = vec!["--to", "infisical", "--project", "p"];
        args.extend_from_slice(extra);
        let out = migrate(&p, &args);
        out.assert_code(2);
        assert!(out.stderr.contains("KEY"), "{}", out.stderr);
        assert!(out.stderr.contains("EMPTY"), "{}", out.stderr);
        assert!(!out.stderr.contains("/etc/passwd"), "{}", out.stderr);
        assert!(!out.stdout.contains("secrets set"), "{}", out.stdout);
    }
}

// ── no tool needed to print (I-110) ─────────────────────────────────────────

/// Printing a command never needs the tool: dry runs did not, real runs of
/// doppler, infisical and railway did.
#[test]
fn t_i110_printing_needs_no_tool_installed() {
    let p = Project::new().env("A=1\n");
    for args in [
        &["--to", "doppler", "--project", "p"][..],
        &["--to", "infisical", "--project", "p"][..],
        &["--to", "railway"][..],
    ] {
        let mut full = vec!["migrate", "--from", "env-file"];
        full.extend_from_slice(args);
        let out = run_with(p.path(), &full, &[("PATH", "/nonexistent")], None);
        out.assert_code(0);
        assert!(out.stdout.contains("A='1'"), "{args:?}\n{}", out.all());
    }
}

// ── names (I-267) ───────────────────────────────────────────────────────────

/// `--add-prefix` must keep every name a variable name, as convert's
/// `--prefix` does (I-263): a `;` reached the bare name in gcp's command.
#[test]
fn t_i267_add_prefix_must_keep_names_variable_names() {
    let p = Project::new().env("PORT=8080\n");
    for prefix in ["X;touch PWNED;", "1", "A-"] {
        let out = migrate(
            &p,
            &[
                "--to",
                "gcp-secret-manager",
                "--project",
                "my-proj",
                "--add-prefix",
                prefix,
            ],
        );
        out.assert_code(2);
        assert!(out.stderr.contains("--add-prefix"), "{}", out.stderr);
        assert!(!out.stdout.contains("gcloud"), "{}", out.stdout);
    }
    assert!(!p.path().join("PWNED").exists());
}

/// A strip that takes a whole name, or leaves it starting with a digit,
/// leaves nothing a destination accepts.
#[test]
fn t_i267_strip_prefix_must_leave_variable_names() {
    let p = Project::new().env("API_=x\nAPI_1X=y\nB=1\n");
    let out = migrate(
        &p,
        &[
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
            "--strip-prefix",
            "API_",
        ],
    );
    out.assert_code(2);
    assert!(out.stderr.contains("API_"), "{}", out.stderr);
    assert!(out.stderr.contains("API_1X"), "{}", out.stderr);
    assert!(out.stderr.contains("--strip-prefix"), "{}", out.stderr);
    assert!(!out.stdout.contains("config:set"), "{}", out.stdout);
}
