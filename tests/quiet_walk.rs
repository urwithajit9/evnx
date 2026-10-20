//! `-q` is silent on success, for every command (T-SHARE-60, I-007, G1).
//!
//! # Why a walk
//!
//! `-q` was wired to one banner and nothing else (I-007); then it was fixed
//! command by command, and each fix left the next command behind. This test
//! reads the command list from `evnx commands --json`, so a command added
//! later fails here until it is given a quiet success case or a stated reason
//! to have none.
//!
//! Each case runs in a healthy project (a real git repository, `.env` at 0600
//! and ignored, `.env.example` tracked) and must exit 0 with nothing on stdout
//! and nothing on stderr, unless it names the one warning it is allowed.
//! Warnings always print (DEC-G2); data a command was asked for always prints
//! (DEC-SF5, DEC-G3).
#![cfg(unix)]

mod support;

use std::collections::BTreeSet;

use support::{run, Project};

/// A success path that `-q` must make silent.
struct Case {
    path: &'static str,
    args: &'static [&'static str],
    /// Text a warning is allowed to print on stderr (`None`: stderr empty).
    warning: Option<&'static str>,
    /// Make the backup that `restore` reads, before the run.
    needs_backup: bool,
}

const fn case(path: &'static str, args: &'static [&'static str]) -> Case {
    Case {
        path,
        args,
        warning: None,
        needs_backup: false,
    }
}

fn cases() -> Vec<Case> {
    let mut v = vec![
        case("init", &["init", "--with", "nextjs", "--yes"]),
        case("add service", &["add", "service", "redis", "--yes"]),
        case("add framework", &["add", "framework", "django", "--yes"]),
        case("add blueprint", &["add", "blueprint", "t3_modern", "--yes"]),
        case("add custom", &["add", "custom", "MY_FLAG=1", "--yes"]),
        case("validate", &["validate"]),
        case("scan", &["scan", "."]),
        case("diff", &["diff"]),
        // `-o` in a git repository checks `.gitignore` since 0.10.9 and warns
        // when out.json is not covered — a warning `-q` keeps (DEC-G2), tested
        // in convert_contract.rs. `--no-gitignore`, as for template below.
        case(
            "convert",
            &[
                "convert",
                "--to",
                "json",
                "-o",
                "out.json",
                "--no-gitignore",
            ],
        ),
        case("sync", &["sync", "-f"]),
        case(
            "template",
            &[
                "template",
                "--input",
                "t.tpl",
                "--output",
                "t.out",
                "--no-gitignore",
            ],
        ),
        case("doctor", &["doctor"]),
        case("spec init", &["spec", "init"]),
    ];
    if cfg!(feature = "backup") {
        v.push(case("backup", &["backup", "--key-file", "pw"]));
        v.push(Case {
            path: "restore",
            // `--no-gitignore`, as for convert and template above: the restored
            // file gets the same `.gitignore` check since 0.10.11, and its
            // warning is one `-q` keeps (tested in restore_integration.rs).
            // The key-file line is backup's info line since 0.10.11 (DEC-B13),
            // not a warning, so `-q` hides it and stderr is empty.
            args: &[
                "restore",
                ".env.backup",
                "--key-file",
                "pw",
                "--output",
                "restored.env",
                "--no-gitignore",
            ],
            warning: None,
            needs_backup: true,
        });
    }
    v
}

/// Commands with no quiet success case here, each with the reason.
const EXEMPT: &[(&str, &str)] = &[
    // Data: `-q` never suppresses the answer (DEC-SF5, DEC-G3).
    ("commands", "data"),
    ("completions", "data"),
    (
        "cloud status",
        "data: the answer stays under -q (DEC-G3); see t_cloud_status_keeps_the_answer",
    ),
    ("auth status", "data: the answer stays under -q (DEC-G3)"),
    (
        "cloud unlink",
        "with no binding the answer stays; see t_cloud_status_keeps_the_answer",
    ),
    // Known residual, moves with `--print-only` (I-111) in U6.
    (
        "migrate",
        "residual: destination intro lines stay until --print-only (I-111, U6)",
    ),
    // Network.
    ("update", "needs the network"),
];

/// The cloud group needs a server; its `-q` is the output handle in U7 (A-44,
/// DEC-CL3).
const NEEDS_SERVER: &[&str] = &["auth", "org", "vault", "cloud"];

fn healthy_project() -> Project {
    let p = Project::new()
        .git()
        .example("APP_NAME=demo\nPORT=8080\n")
        .env("APP_NAME=demo\nPORT=8080\n")
        .mode(".env", 0o600)
        .file(".gitignore", ".env\n")
        .file("pw", "pw-for-demo-only-123456\n")
        .file("t.tpl", "port: {{PORT}}\n");
    p.track(".env.example").track(".gitignore");
    p.commit_all("init");
    p
}

fn leaves() -> Vec<String> {
    let paths: Vec<String> = support::surface::entries()
        .iter()
        .map(|c| c.name())
        .collect();
    // A path that prefixes another is a group (`add`, `auth token`), not a leaf.
    paths
        .iter()
        .filter(|p| !paths.iter().any(|o| o.starts_with(&format!("{p} "))))
        .cloned()
        .collect()
}

/// Every leaf has a case, an exemption, or belongs to the server-side group.
#[test]
fn t_share_60_every_command_is_classified() {
    let covered: BTreeSet<&str> = cases().iter().map(|c| c.path).collect();
    let exempt: BTreeSet<&str> = EXEMPT.iter().map(|(p, _)| *p).collect();
    let unclassified: Vec<String> = leaves()
        .into_iter()
        .filter(|p| !covered.contains(p.as_str()) && !exempt.contains(p.as_str()))
        .filter(|p| !NEEDS_SERVER.contains(&p.split(' ').next().unwrap()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "commands with no -q case and no stated reason: {unclassified:?}\n\
         Add a quiet success case to tests/quiet_walk.rs, or an EXEMPT entry saying why."
    );
}

/// The walk itself: each case is silent under `-q`.
#[test]
fn t_share_60_quiet_is_silent_on_success() {
    let mut failures = Vec::new();
    for c in cases() {
        let p = healthy_project();
        if c.path == "init" {
            std::fs::remove_file(p.join(".env")).unwrap();
            std::fs::remove_file(p.join(".env.example")).unwrap();
        }
        if c.needs_backup {
            run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
        }
        let mut args = vec!["-q"];
        args.extend_from_slice(c.args);
        let out = run(p.path(), &args);
        let stderr_ok = match c.warning {
            None => out.stderr.trim().is_empty(),
            Some(w) => out
                .stderr
                .lines()
                .filter(|l| !l.trim().is_empty())
                .all(|l| l.contains(w) || l.starts_with("  ")),
        };
        if out.code != Some(0) || !out.stdout.trim().is_empty() || !stderr_ok {
            failures.push(format!(
                "evnx {}  exit {:?}\n--- stdout\n{}--- stderr\n{}",
                args.join(" "),
                out.code,
                out.stdout,
                out.stderr
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `-q cloud status` keeps the answer and drops the heading, hints and Docs
/// line (DEC-G3); `-q cloud unlink` keeps "no binding" (DEC-CL3). No network:
/// signed out, with an empty config directory.
#[cfg(feature = "cloud")]
#[test]
fn t_cloud_status_keeps_the_answer() {
    let p = Project::new();
    let home = p.join("home");
    std::fs::create_dir(&home).unwrap();
    let env = [
        ("HOME", home.to_str().unwrap()),
        ("EVNX_CONFIG_DIR", home.to_str().unwrap()),
    ];
    let out = support::run_with(p.path(), &["-q", "cloud", "status"], &env, None);
    out.assert_code(0);
    assert!(out.stdout.contains("server"), "{}", out.stdout);
    assert!(out.stdout.contains("not signed in"), "{}", out.stdout);
    for gone in ["evnx cloud", "auth register", "Docs:"] {
        assert!(
            !out.all().contains(gone),
            "{gone:?} under -q:\n{}",
            out.all()
        );
    }
    let out = support::run_with(p.path(), &["-q", "cloud", "unlink"], &env, None);
    out.assert_code(0);
    assert!(out.stdout.contains("no vault binding"), "{}", out.stdout);
}

/// The other half of G1: `-q` hides success, never a finding. With a problem
/// to report, doctor and scan still report it and still exit 1.
#[test]
fn t_share_60_quiet_still_prints_findings() {
    // doctor: `.env` that git would commit.
    let p = healthy_project();
    std::fs::write(p.join(".gitignore"), "").unwrap();
    let out = run(p.path(), &["-q", "doctor"]);
    out.assert_code(1);
    out.assert_contains(".gitignore");

    // scan: a key-shaped value.
    let p = healthy_project();
    std::fs::write(p.join(".env"), "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n").unwrap();
    let out = run(p.path(), &["-q", "scan", "."]);
    out.assert_code(1);
    out.assert_contains("AWS Access Key");
}

/// I-226 (DEC-CL4): the "using <vault> from <file>" notice is on stderr, so
/// `cloud pull` to stdout and `cloud run` output carry only data, and `-q`
/// hides it. Offline: the run stops at "not signed in", after the notice.
#[cfg(feature = "cloud")]
#[test]
fn t_i226_vault_notice_is_on_stderr() {
    let p = Project::new().toml("[cloud]\nvault = \"app/dev\"\n");
    let home = p.join("home");
    std::fs::create_dir(&home).unwrap();
    let env = [
        ("HOME", home.to_str().unwrap()),
        ("EVNX_CONFIG_DIR", home.to_str().unwrap()),
    ];
    let out = support::run_with(p.path(), &["cloud", "pull"], &env, None);
    assert!(out.stderr.contains("using app/dev from"), "{}", out.stderr);
    assert!(!out.stdout.contains("using"), "{}", out.stdout);
    let out = support::run_with(p.path(), &["-q", "cloud", "pull"], &env, None);
    assert!(!out.all().contains("using app/dev"), "{}", out.all());
}
