//! The commands `evnx migrate` prints pass every value to the tool **exactly**
//! (U6, A-16; T-MIG-01..04, 07).
//!
//! The printed commands are run through `sh` with stub tools on `PATH` that
//! record their arguments and stdin, so the test checks what the tool would
//! receive, not what the text looks like.
//!
//! - I-094: aws escaped `'` as `\'` inside single quotes, which ends the string:
//!   the command was a syntax error, or ran the rest of the value.
//! - I-101: vercel and azure put values in **double quotes**, so `$5` came out
//!   empty and `$(...)` ran when the user ran the printed command.
//! - I-102 (quoting half): names printed into commands (app, project, vault,
//!   secret name, profile) were not quoted, so `;` started a second command.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use std::path::Path;

use support::{run_with, Project};

/// Values that each break one quoting mistake or another.
const VALUES: &[(&str, &str)] = &[
    ("APOS", "it's"),
    ("PRICE", "$5 off"),
    ("SUBST", "$(touch pwned)"),
    ("TICKS", "`touch pwned2`"),
    ("BSLASH", "back\\slash"),
    ("SPACE", "two words"),
    ("DQUOTE", "say \"hi\""),
    ("HASH", "#hash"),
    ("BANG", "!bang"),
    ("GLOB", "*"),
];

const TOOLS: &[&str] = &[
    "aws",
    "az",
    "vercel",
    "heroku",
    "doppler",
    "infisical",
    "railway",
];

fn env_file() -> String {
    VALUES
        .iter()
        // single quotes in the .env: literal for the parser
        .map(|(k, v)| {
            if v.contains('\'') {
                format!("{k}=\"{v}\"\n")
            } else {
                format!("{k}='{v}'\n")
            }
        })
        .collect()
}

/// Stub tools: each invocation appends `tool␟arg…␟STDIN=…␞` to `log`.
fn stubs(dir: &Path) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for t in TOOLS {
        let script = format!(
            "#!/bin/sh\n\
             in=$(cat)\n\
             {{ printf '%s' '{t}'; for a; do printf '\\037%s' \"$a\"; done; \
             printf '\\037STDIN=%s\\036' \"$in\"; }} >> \"$STUB_LOG\"\n"
        );
        support::stub::write_stub(&bin.join(t), &script);
    }
}

/// The command paragraphs of migrate's stdout: blank-line-separated blocks,
/// each from its first command or comment line on, kept only if it holds a
/// command. Headings, summaries and "Next steps" are dropped.
fn commands(stdout: &str) -> String {
    let is_cmd = |l: &str| {
        TOOLS.iter().any(|t| l.starts_with(&format!("{t} ")))
            || l.starts_with("echo ")
            || l.starts_with("printf ")
    };
    let mut out = String::new();
    for para in stdout.split("\n\n") {
        let lines: Vec<&str> = para.lines().collect();
        let Some(start) = lines.iter().position(|l| is_cmd(l) || l.starts_with('#')) else {
            continue;
        };
        if lines[start..].iter().any(|l| is_cmd(l)) {
            out.push_str(&lines[start..].join("\n"));
            out.push_str("\n\n");
        }
    }
    out
}

struct Call {
    args: Vec<String>,
    stdin: String,
}

/// Run `evnx migrate <args>`, then its printed commands through `sh`.
fn run_printed(args: &[&str]) -> (Project, Vec<Call>, String) {
    let p = Project::new().env(&env_file());
    stubs(p.path());
    let mut full = vec!["migrate", "--from", "env-file"];
    full.extend_from_slice(args);
    let path = format!(
        "{}:{}",
        p.path().join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // ⚠️ The stubs are on PATH for evnx too: doppler, infisical and railway
    // refuse a real run when their CLI is missing (I-110, a separate item).
    // The probe's own call lands in the log and matches no value.
    let probe_log = p.path().join("probe.log");
    let out = run_with(
        p.path(),
        &full,
        &[
            ("PATH", path.as_str()),
            ("STUB_LOG", probe_log.to_str().unwrap()),
        ],
        None,
    );
    out.assert_code(0);
    let script = commands(&out.stdout);
    assert!(
        !script.trim().is_empty(),
        "no commands found in:\n{}",
        out.stdout
    );
    std::fs::write(p.path().join("run.sh"), &script).unwrap();
    let log = p.path().join("stub.log");
    let sh = std::process::Command::new("sh")
        .arg("run.sh")
        .current_dir(p.path())
        .env("PATH", path)
        .env("STUB_LOG", &log)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let raw = std::fs::read_to_string(&log).unwrap_or_default();
    let calls = raw
        .split('\u{1e}')
        .filter(|r| !r.is_empty())
        .map(|r| {
            let mut parts: Vec<String> = r.split('\u{1f}').map(String::from).collect();
            let stdin = parts
                .pop()
                .unwrap()
                .trim_start_matches("STDIN=")
                .to_string();
            Call { args: parts, stdin }
        })
        .collect();
    let report = format!(
        "--- printed:\n{script}\n--- sh stderr:\n{}",
        String::from_utf8_lossy(&sh.stderr)
    );
    (p, calls, report)
}

/// Every value reached a tool exactly, and nothing in a value ran.
fn assert_exact(p: &Project, calls: &[Call], report: &str, key_of: &dyn Fn(&str) -> String) {
    assert!(!calls.is_empty(), "no tool ran\n{report}");
    for (k, v) in VALUES {
        let key = key_of(k);
        let arrived = calls.iter().any(|c| {
            c.stdin == *v
                || c.args.iter().any(|a| a == v || *a == format!("{key}={v}"))
                || c.args.iter().any(|a| {
                    serde_json::from_str::<serde_json::Value>(a)
                        .map(|j| j[*k] == **v)
                        .unwrap_or(false)
                })
        });
        assert!(arrived, "{k}={v:?} did not arrive exactly\n{report}");
    }
    for f in ["pwned", "pwned2"] {
        assert!(
            !p.path().join(f).exists(),
            "a value ran a command\n{report}"
        );
    }
}

fn same(k: &str) -> String {
    k.to_string()
}

#[test]
fn t_mig_01_i094_aws_passes_values_exactly() {
    let (p, calls, report) = run_printed(&["--to", "aws-secrets-manager", "--secret-name", "s"]);
    assert_exact(&p, &calls, &report, &same);
}

#[test]
fn t_mig_02_i101_vercel_passes_values_exactly() {
    let (p, calls, report) = run_printed(&["--to", "vercel", "--vercel-project", "p"]);
    assert_exact(&p, &calls, &report, &same);
}

#[test]
fn t_mig_03_i101_azure_passes_values_exactly() {
    let (p, calls, report) = run_printed(&["--to", "azure-keyvault", "--vault-name", "vlt"]);
    assert_exact(&p, &calls, &report, &same);
}

/// T-MIG-04: these four were right; this locks them.
#[test]
fn t_mig_04_heroku_doppler_infisical_railway_stay_exact() {
    for args in [
        &["--to", "heroku", "--heroku-app", "app"][..],
        &["--to", "doppler", "--project", "p"][..],
        &["--to", "infisical", "--project", "p"][..],
        &["--to", "railway", "--railway-project", "p"][..],
    ] {
        let (p, calls, report) = run_printed(args);
        assert_exact(&p, &calls, &report, &same);
    }
}

/// I-102, I-107 (A-17): a name that cannot be valid for the destination is
/// refused before anything is printed — exit 2, naming the flag. (Names that
/// pass are still quoted as shell words; `core::shell` tests that.)
#[test]
fn t_mig_09_i102_bad_names_are_refused() {
    let p = Project::new().env("A=1\n");
    for args in [
        &["--to", "heroku", "--heroku-app", "x;touch pwned"][..],
        &["--to", "azure-keyvault", "--vault-name", "v;touch pwned"][..],
        &[
            "--to",
            "aws-secrets-manager",
            "--secret-name",
            "s;touch pwned",
        ][..],
        &[
            "--to",
            "aws-secrets-manager",
            "--secret-name",
            "s",
            "--aws-profile",
            "a b",
        ][..],
        &["--to", "vercel", "--vercel-project", "p;touch pwned"][..],
        &["--to", "doppler", "--project", "p$(id)"][..],
        &[
            "--to",
            "infisical",
            "--project",
            "p",
            "--infisical-env",
            "dev`id`",
        ][..],
        &["--to", "railway", "--railway-project", "p|sh"][..],
        &["--to", "github-actions", "--repo", "notarepo", "--dry-run"][..],
        &["--to", "github-actions", "--repo", "o/r;x", "--dry-run"][..],
    ] {
        let flag = args[args.len() - 2];
        let flag = if flag == "--dry-run" { args[3] } else { flag };
        let mut full = vec!["migrate", "--from", "env-file"];
        full.extend_from_slice(args);
        let out = support::run(p.path(), &full);
        out.assert_code(2);
        assert!(out.stderr.contains(flag), "{args:?}: {}", out.stderr);
        assert!(
            !out.stdout.contains("set "),
            "{args:?} printed:\n{}",
            out.stdout
        );
    }
}

/// Real names pass unchanged (a lock against an over-strict rule).
#[test]
fn t_mig_32_real_names_are_accepted() {
    let p = Project::new().env("A=1\n");
    for args in [
        &["--to", "heroku", "--heroku-app", "my-app-01"][..],
        &["--to", "azure-keyvault", "--vault-name", "Prod-Vault-7"][..],
        &[
            "--to",
            "aws-secrets-manager",
            "--secret-name",
            "prod/my_app+v1=x.y@z-1",
            "--aws-profile",
            "dev.admin",
        ][..],
        &["--to", "vercel", "--vercel-project", "prj_AbC123"][..],
        &["--to", "github-actions", "--repo", "my-org/my.repo_1"][..],
    ] {
        let mut full = vec!["migrate", "--from", "env-file", "--dry-run"];
        full.extend_from_slice(args);
        let out = support::run(p.path(), &full);
        out.assert_code(0);
    }
}

/// I-103, S-M5 (A-18): `--from environment` sent the whole process
/// environment minus 14 names — in CI that includes tokens. It now needs
/// `--include`, and says which names it will send.
#[test]
fn t_mig_18_i103_environment_needs_include() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &[
            "migrate",
            "--from",
            "environment",
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
            "--dry-run",
        ],
        &[("API_TOKEN", "tok-sentinel"), ("APP_NAME", "demo")],
        None,
    );
    out.assert_code(2);
    assert!(out.stderr.contains("--include"), "{}", out.stderr);
    assert!(!out.all().contains("tok-sentinel"), "{}", out.all());
    assert!(!out.stdout.contains("API_TOKEN"), "{}", out.stdout);
}

#[test]
fn t_mig_17_i103_environment_sends_only_what_is_included_and_names_it() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &[
            "migrate",
            "--from",
            "environment",
            "--include",
            "APP_*",
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
        ],
        &[("API_TOKEN", "tok-sentinel"), ("APP_NAME", "demo")],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("APP_NAME='demo'"), "{}", out.stdout);
    assert!(!out.all().contains("API_TOKEN"), "{}", out.all());
    assert!(
        out.stderr.contains("APP_NAME") && out.stderr.contains("1 variable"),
        "the names to be sent are listed:\n{}",
        out.stderr
    );
}
