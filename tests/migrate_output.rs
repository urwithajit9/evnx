//! What `evnx migrate` reads, and what it writes where (R-migrate).
//!
//! - I-104: `--strip-prefix` / `--add-prefix` could turn two names into one;
//!   one value was dropped and the run said only "1 of 2 secrets remain".
//!   Refused now, exit 2, naming both; `--allow-collisions` keeps the last
//!   (DEC-PL2, the rule convert follows).
//! - I-111: the commands shared stdout with the banner, status lines and the
//!   summary, so `> run.sh` saved something that was not a script.
//!   `--print-only` writes only commands to stdout.
//! - I-105: `-q` left the destination's own status lines in place; the
//!   `--from` help did not say what happens without a terminal.
//! - I-108: `--github-token` puts the token in `ps` and shell history; evnx
//!   now says so when it is used, and not when the token comes from
//!   `GITHUB_TOKEN`.
//! - I-031, S-M1: migrate had no `--env-name` and ignored
//!   `[defaults] env_name`, so it pushed `.env` while every other command
//!   read `.env.staging`.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use support::{run, run_with, Project};

fn migrate(p: &Project, args: &[&str]) -> support::Output {
    let mut full = vec!["migrate", "--from", "env-file"];
    full.extend_from_slice(args);
    run(p.path(), &full)
}

// ── collisions (I-104) ──────────────────────────────────────────────────────

#[test]
fn t_i104_a_rename_that_merges_two_names_is_refused() {
    let p = Project::new().env("API_X=1\nX=2\n");
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
    assert!(out.stderr.contains("API_X, X → X"), "{}", out.stderr);
    assert!(out.stderr.contains("--allow-collisions"), "{}", out.stderr);
    assert!(!out.stdout.contains("config:set"), "{}", out.stdout);
}

#[test]
fn t_i104_allow_collisions_keeps_the_last_value_and_says_so() {
    let p = Project::new().env("API_X=1\nX=2\n");
    let out = migrate(
        &p,
        &[
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
            "--strip-prefix",
            "API_",
            "--allow-collisions",
        ],
    );
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("heroku config:set --app demo-app \\\n  X='2'\n"),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("kept the value of X"), "{}", out.stderr);
}

/// Azure Key Vault ignores case and turns `_` into `-`: `A_B` and `a_b` are
/// one secret there, and `az keyvault secret set` would overwrite in silence.
#[test]
fn t_i104_names_the_destination_folds_together_are_refused() {
    let p = Project::new().env("A_B=1\na_b=2\n");
    let out = migrate(
        &p,
        &["--to", "azure-keyvault", "--vault-name", "demo-vault"],
    );
    out.assert_code(2);
    assert!(out.stderr.contains("A_B"), "{}", out.stderr);
    assert!(out.stderr.contains("a_b"), "{}", out.stderr);
    assert!(!out.stdout.contains("az keyvault"), "{}", out.stdout);
}

// ── --print-only (I-111) ────────────────────────────────────────────────────

const PRINTERS: &[&[&str]] = &[
    &["--to", "aws-secrets-manager", "--secret-name", "prod/app"],
    &["--to", "gcp-secret-manager", "--project", "my-proj"],
    &["--to", "azure-keyvault", "--vault-name", "demo-vault"],
    &["--to", "heroku", "--heroku-app", "demo-app"],
    &["--to", "vercel", "--vercel-project", "prj_1"],
    &[
        "--to",
        "doppler",
        "--project",
        "p",
        "--doppler-config",
        "dev",
    ],
    &[
        "--to",
        "infisical",
        "--project",
        "p",
        "--infisical-env",
        "dev",
    ],
    &[
        "--to",
        "railway",
        "--railway-project",
        "p-1",
        "--railway-environment",
        "production",
    ],
];

/// `evnx migrate … --print-only > run.sh` saves a script: stdout holds the
/// commands and nothing else, and `sh -n` reads it.
#[test]
fn t_i111_print_only_writes_only_commands_to_stdout() {
    let p = Project::new().env("APP_NAME=demo app\nAPI_SECRET=abc123\nPORT=8080\n");
    for args in PRINTERS {
        let mut full = args.to_vec();
        full.push("--print-only");
        let out = migrate(&p, &full);
        out.assert_code(0);
        for noise in [
            "Loaded",
            "Summary",
            "Next steps",
            "Install:",
            "Requires",
            "migration",
            "Run ",
            "Docs:",
        ] {
            assert!(
                !out.stdout.contains(noise),
                "{args:?}: {noise:?} on stdout:\n{}",
                out.stdout
            );
        }
        assert!(
            out.stderr.contains("Summary"),
            "{args:?}: the summary moves to stderr:\n{}",
            out.stderr
        );
        std::fs::write(p.path().join("run.sh"), &out.stdout).unwrap();
        let check = std::process::Command::new("sh")
            .args(["-n", "run.sh"])
            .current_dir(p.path())
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{args:?}: not a script:\n{}\n{}",
            out.stdout,
            String::from_utf8_lossy(&check.stderr)
        );
    }
}

/// Without `--print-only` the output is what it was.
#[test]
fn t_i111_default_output_keeps_status_on_stdout() {
    let p = Project::new().env("A=1\n");
    let out = migrate(&p, &["--to", "heroku", "--heroku-app", "demo-app"]);
    out.assert_code(0);
    assert!(out.stdout.contains("Loaded 1 secret"), "{}", out.stdout);
    assert!(out.stdout.contains("Summary"), "{}", out.stdout);
    assert!(out.stdout.contains("config:set"), "{}", out.stdout);
}

#[test]
fn t_i111_print_only_is_refused_where_nothing_is_printed() {
    let p = Project::new().env("A=1\n");
    let github = migrate(
        &p,
        &["--to", "github-actions", "--repo", "o/r", "--print-only"],
    );
    github.assert_code(2);
    assert!(github.stderr.contains("--print-only"), "{}", github.stderr);

    let dry = migrate(
        &p,
        &[
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
            "--print-only",
            "--dry-run",
        ],
    );
    dry.assert_code(2);
    assert!(dry.stderr.contains("--print-only"), "{}", dry.stderr);
}

// ── -q and --from (I-105) ───────────────────────────────────────────────────

#[test]
fn t_i105_quiet_leaves_the_commands_and_the_summary() {
    let p = Project::new().env("A=1\n");
    let out = run(
        p.path(),
        &[
            "-q",
            "migrate",
            "--from",
            "env-file",
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
        ],
    );
    out.assert_code(0);
    for gone in [
        "Heroku Config Vars migration",
        "Requires",
        "Install:",
        "Run this",
    ] {
        assert!(!out.stdout.contains(gone), "{gone:?}:\n{}", out.stdout);
    }
    assert!(out.stdout.contains("heroku config:set"), "{}", out.stdout);
    assert!(
        out.stdout.contains("command(s) generated"),
        "{}",
        out.stdout
    );
}

#[test]
fn t_i105_from_help_says_what_happens_without_a_terminal() {
    let p = Project::new();
    let out = run(p.path(), &["migrate", "--help"]);
    out.assert_code(0);
    let help = out.stdout.replace('\n', " ");
    assert!(help.contains("asks in a terminal"), "{}", out.stdout);
    assert!(help.contains("`env-file` without one"), "{}", out.stdout);
}

// ── --github-token (I-108) ──────────────────────────────────────────────────

#[test]
fn t_i108_a_token_on_the_command_line_is_warned_about() {
    let p = Project::new().env("A=1\n");
    let out = migrate(
        &p,
        &[
            "--to",
            "github-actions",
            "--repo",
            "o/r",
            "--github-token",
            "tok-sentinel",
            "--dry-run",
        ],
    );
    out.assert_code(0);
    assert!(out.stderr.contains("GITHUB_TOKEN"), "{}", out.stderr);
    assert!(out.stderr.contains("shell history"), "{}", out.stderr);
    assert!(!out.all().contains("tok-sentinel"), "{}", out.all());
}

#[test]
fn t_i108_a_token_from_the_environment_is_not_warned_about() {
    let p = Project::new().env("A=1\n");
    let out = run_with(
        p.path(),
        &[
            "migrate",
            "--from",
            "env-file",
            "--to",
            "github-actions",
            "--repo",
            "o/r",
            "--dry-run",
        ],
        &[("GITHUB_TOKEN", "tok-sentinel")],
        None,
    );
    out.assert_code(0);
    assert!(!out.stderr.contains("shell history"), "{}", out.stderr);
    assert!(!out.all().contains("tok-sentinel"), "{}", out.all());
}

// ── --env-name and [defaults] env_name (I-031, S-M1) ────────────────────────

fn two_environments() -> Project {
    Project::new()
        .env("DEV_ONLY=d\n")
        .file(".env.staging", "STAGING_ONLY=s\n")
}

const HEROKU: &[&str] = &["--to", "heroku", "--heroku-app", "demo-app"];

#[test]
fn t_i031_env_name_reads_that_environment() {
    let p = two_environments();
    let mut args = vec!["--env-name", "staging"];
    args.extend_from_slice(HEROKU);
    let out = migrate(&p, &args);
    out.assert_code(0);
    assert!(out.stdout.contains("STAGING_ONLY='s'"), "{}", out.stdout);
    assert!(!out.stdout.contains("DEV_ONLY"), "{}", out.stdout);
}

#[test]
fn t_i031_defaults_env_name_is_followed() {
    let p = two_environments().toml("[defaults]\nenv_name = \"staging\"\n");
    let out = migrate(&p, HEROKU);
    out.assert_code(0);
    assert!(out.stdout.contains("STAGING_ONLY='s'"), "{}", out.stdout);
    assert!(!out.stdout.contains("DEV_ONLY"), "{}", out.stdout);
}

/// An explicit file beats the config, as `--env` does for the other commands.
#[test]
fn t_i031_source_file_beats_the_config() {
    let p = two_environments().toml("[defaults]\nenv_name = \"staging\"\n");
    let mut args = vec!["--source-file", ".env"];
    args.extend_from_slice(HEROKU);
    let out = migrate(&p, &args);
    out.assert_code(0);
    assert!(out.stdout.contains("DEV_ONLY='d'"), "{}", out.stdout);
}

#[test]
fn t_i031_an_unknown_environment_lists_the_ones_there_are() {
    let p = two_environments();
    let mut args = vec!["--env-name", "prod"];
    args.extend_from_slice(HEROKU);
    let out = migrate(&p, &args);
    out.assert_code(2);
    assert!(out.stderr.contains("staging"), "{}", out.stderr);
    assert!(!out.stdout.contains("config:set"), "{}", out.stdout);
}

#[test]
fn t_i031_env_name_does_not_apply_to_the_process_environment() {
    let p = two_environments();
    let out = run(
        p.path(),
        &[
            "migrate",
            "--from",
            "environment",
            "--include",
            "X",
            "--env-name",
            "staging",
            "--to",
            "heroku",
            "--heroku-app",
            "demo-app",
        ],
    );
    out.assert_code(2);
    assert!(out.stderr.contains("--env-name"), "{}", out.stderr);
}

// ── README examples (T-MIG-49; D-54, D-56, D-58) ────────────────────────────

/// `line` split as `sh` would split it, for the quoting the README uses.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, ' ') => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Every one-line `evnx migrate` example in the README runs and exits 0,
/// without any destination's tool installed. The README said migrate
/// "migrates secrets directly to cloud secret managers" — true for one of nine
/// — and taught `--github-token $GITHUB_TOKEN` (D-54, D-56). The GitHub
/// example uploads, so it is left out here; `migrate_exit.rs` covers it.
#[test]
fn t_mig_49_readme_migrate_examples_run() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("### `evnx migrate`")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .expect("README has a migrate section");
    assert!(!section.contains("directly to cloud"), "D-54");
    assert!(!section.contains("--github-token $GITHUB_TOKEN"), "D-56");

    let env = "APP_NAME=demo\nDATABASE_URL=postgres://u:p@db/app\n";
    let p = Project::new().env(env).file(".env.production", env);
    let mut ran = 0;
    for line in section.lines().map(str::trim) {
        if !line.starts_with("evnx migrate") || line.ends_with('\\') {
            continue;
        }
        if line.contains("github-actions") {
            continue;
        }
        let line = line.split(" #").next().unwrap();
        let line = line.split(" >").next().unwrap();
        let args = words(line);
        let args: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
        let out = run_with(
            p.path(),
            &args,
            &[("APP_NAME", "demo"), ("PATH", "/nonexistent")],
            None,
        );
        assert_eq!(
            out.code,
            Some(0),
            "README example failed: {line}\n{}",
            out.stderr
        );
        ran += 1;
    }
    assert!(ran >= 5, "only {ran} examples found");
}
