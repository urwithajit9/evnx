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

use support::{run, Project};

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
