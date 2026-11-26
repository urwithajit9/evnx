//! The exit-code contract, held the same way by every command (U4, C-04).
//!
//! | Code | Means |
//! |---|---|
//! | 0 | ran, nothing to report |
//! | 1 | ran, and the answer is "no": findings, differences, drift, failures |
//! | 2 | could not run — **no verdict**, and stderr says so |
//!
//! - I-116, DEC-S13: `sync --naming-policy error` exited **1** on a bad name, so
//!   `sync --check` read a configuration refusal as drift.
//! - I-074: `diff --format xml` fell back to `pretty` and exited 1 on the
//!   differences, so a typo in a CI step quietly broke the JSON contract.
//!   `sync --check --format` did the same.
//! - `diff`'s error path printed `Error:` without the "No verdict" line every
//!   other command prints.

mod support;

use support::{Project, run};

const NO_VERDICT: &str = "No verdict";

fn bad_name() -> Project {
    Project::new().env("my_key=1\nA=1\n").example("A=\n")
}

#[test]
fn t_i116_sync_naming_policy_error_is_no_verdict() {
    let p = bad_name();
    for extra in [&["--dry-run"][..], &["--check"][..]] {
        let mut args = vec!["sync", "--naming-policy", "error"];
        args.extend_from_slice(extra);
        let out = run(p.path(), &args);
        out.assert_code(2);
        assert!(out.stderr.contains("my_key"), "{extra:?}: {}", out.stderr);
        assert!(out.stderr.contains(NO_VERDICT), "{extra:?}: {}", out.stderr);
    }
}

#[test]
fn t_i116_sync_naming_policy_error_reverse_is_no_verdict() {
    let p = Project::new().env("A=1\n").example("A=\nmy_key=\n");
    let out = run(
        p.path(),
        &[
            "sync",
            "--direction",
            "reverse",
            "--naming-policy",
            "error",
            "--check",
        ],
    );
    out.assert_code(2);
    assert!(out.stderr.contains("my_key"), "{}", out.stderr);
}

/// Warn, the default, still syncs and still reports drift as 1.
#[test]
fn t_i116_naming_policy_warn_is_unchanged() {
    let p = bad_name();
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(1);
}

#[test]
fn t_i074_diff_unknown_format_is_refused() {
    let p = bad_name();
    let out = run(p.path(), &["diff", "--format", "xml"]);
    out.assert_code(2);
    assert!(out.stdout.is_empty(), "nothing rendered:\n{}", out.stdout);
    for allowed in ["pretty", "json", "patch"] {
        assert!(out.stderr.contains(allowed), "{}", out.stderr);
    }
}

#[test]
fn t_i074_sync_unknown_format_is_refused() {
    let p = bad_name();
    let out = run(p.path(), &["sync", "--check", "--format", "xml"]);
    out.assert_code(2);
    assert!(out.stderr.contains("json"), "{}", out.stderr);
}

#[test]
fn t_u4_diff_error_says_no_verdict() {
    let p = Project::new().example("A=\n");
    let out = run(p.path(), &["diff", "--env", "nope.env"]);
    out.assert_code(2);
    assert!(out.stderr.contains(NO_VERDICT), "{}", out.stderr);
}

// ── WP-9a: one exit contract, `main` alone maps results to codes ─────────────

/// D3: a command that needs a confirmation, has no terminal and was given no
/// `--yes` exits 4 — the code of a prompt answered no — and names `--yes`.
/// `migrate`'s own case is `migrate_exit::t_i109_*`.
#[test]
fn t_wp9_d3_a_confirmation_nobody_can_give_exits_4() {
    let p = Project::new().env("X=1\nY=2\n").example("X=\nOLD=\n");
    for args in [&["remove", "X"][..], &["sync", "--prune"][..]] {
        let out = run(p.path(), args);
        out.assert_code(4);
        assert!(out.stderr.contains("--yes"), "{args:?}: {}", out.stderr);
    }
    assert_eq!(p.read(".env"), "X=1\nY=2\n", "nothing was changed");
    assert_eq!(p.read(".env.example"), "X=\nOLD=\n", "nothing was changed");

    // A copy to put back or delete, then the same refusal from `undo`.
    run(p.path(), &["remove", "Y", "--yes"]).assert_code(0);
    let out = run(p.path(), &["undo", "--clear"]);
    out.assert_code(4);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);

    // And `init`'s menu: it is asked, and nobody can answer.
    let q = Project::new();
    let out = run(q.path(), &["init"]);
    out.assert_code(4);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);
}

/// D3's other half: a refusal that is not about confirming stays 2. `add
/// custom` with no arguments needs a terminal to *type* the variables, which
/// no `--yes` answers.
#[test]
fn t_wp9_d3_a_refusal_that_is_not_a_confirmation_stays_2() {
    let p = Project::new();
    run(p.path(), &["add", "custom", "--yes"]).assert_code(2);
}

/// `main` used to return `Err` for a working directory it could not read, which
/// Rust numbers **1** — "findings". It is 2, with the "did not run" footer.
#[cfg(unix)]
#[test]
fn t_wp9_an_unreadable_current_directory_is_exit_2_not_1() {
    let bin = env!("CARGO_BIN_EXE_evnx");
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "d=$(mktemp -d) && cd \"$d\" && mkdir gone && cd gone && rmdir ../gone && exec '{bin}' validate"
        ))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("reading the current directory"), "{err}");
    assert!(err.contains("did not run"), "{err}");
}

// ── Refusals that already name the next command ────────────────────────────

/// The footer for a refusal that says what to run instead.
const STOPPED: &str = "evnx stopped before making changes (exit 2). Run one of the commands above.";

/// The quick-start run: `spec init` with a contract already there said what to
/// pass, then "No verdict … not a clean result", which reads as a crash.
#[test]
fn spec_init_with_a_contract_says_evnx_stopped() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\n")
        .toml("[vars.A]\nrequired = true\n");
    let out = run(p.path(), &["spec", "init"]);
    out.assert_code(2);
    out.assert_contains("--merge");
    out.assert_contains(STOPPED);
    out.assert_not_contains(NO_VERDICT);
}

/// `scan hook install` where pre-commit manages the hook prints the config to
/// add and the command to run.
#[test]
fn hook_install_beside_a_manager_says_evnx_stopped() {
    let p = Project::new().git();
    let hook = p.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/usr/bin/env bash\n# File generated by pre-commit\n",
    )
    .unwrap();
    let out = run(p.path(), &["scan", "hook", "install"]);
    out.assert_code(2);
    out.assert_contains("pre-commit install");
    out.assert_contains(STOPPED);
    out.assert_not_contains(NO_VERDICT);
}

/// `migrate --to github --dry-run` with names GitHub reserves names the flags
/// that fix it.
#[cfg(feature = "migrate")]
#[test]
fn migrate_refusal_that_names_the_flags_says_evnx_stopped() {
    let p = Project::new().env("GITHUB_APP_ID=1\n");
    let out = run(
        p.path(),
        &[
            "migrate",
            "--from",
            "env-file",
            "--to",
            "github",
            "--repo",
            "o/r",
            "--dry-run",
        ],
    );
    out.assert_code(2);
    out.assert_contains("--add-prefix");
    out.assert_contains(STOPPED);
    out.assert_not_contains(NO_VERDICT);
}

/// `parse --values` on a terminal names `--show-values`.
#[cfg(unix)]
#[test]
fn parse_values_on_a_terminal_says_evnx_stopped() {
    let p = Project::new().env("A=1\n");
    let (out, code) = support::pty::Pty::spawn(p.path(), &["parse", "--values"]).finish();
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--show-values"), "{out}");
    assert!(out.contains(STOPPED), "{out}");
    assert!(!out.contains(NO_VERDICT), "{out}");
}
