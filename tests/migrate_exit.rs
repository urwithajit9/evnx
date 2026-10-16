//! `evnx migrate --to github-actions` reports failed uploads in its exit code
//! (I-106, U4 pilot).
//!
//! An upload GitHub refused was listed under "✗ 1 failed" and the run exited
//! **0**, so a pipeline step that migrated half its secrets passed. A run that
//! finishes with failures is now exit 1 (the answer is "no"); a run that cannot
//! start — no token, no repository, GitHub unreachable — stays 2.
//!
//! I-109 (S-M3): the upload asked "Proceed with migration?" and had no `--yes`,
//! so it could not run in CI. `--yes` now skips the question; with no terminal
//! and no `--yes` it stops before contacting GitHub, exit 2, naming `--yes`. At a
//! terminal it still asks, answered on Enter; Esc cancels and uploads nothing.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use support::pty::{Pty, ENTER};
use support::{run_with, Project};

/// 32 bytes, base64: a key GitHub's sealed-box encryption accepts.
const PUBLIC_KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";

/// A mock GitHub repository `o/r` that accepts both uploads, or refuses one.
fn github(refuse: Option<&str>) -> mockito::ServerGuard {
    let mut server = github_with(&[]);
    for name in ["GOOD_ONE", "BAD_ONE"] {
        let status = if refuse == Some(name) { 500 } else { 201 };
        server
            .mock("PUT", format!("/repos/o/r/actions/secrets/{name}").as_str())
            .with_status(status)
            .create();
    }
    server
}

/// A mock GitHub repository `o/r` that already holds `existing`, with no
/// upload routes: each test adds its own with [`put`], which counts calls.
fn github_with(existing: &[&str]) -> mockito::ServerGuard {
    let mut server = mockito::Server::new();
    let names: Vec<String> = existing
        .iter()
        .map(|n| format!(r#"{{"name":"{n}"}}"#))
        .collect();
    server
        .mock("GET", "/repos/o/r/actions/secrets")
        .with_status(200)
        .with_body(format!(
            r#"{{"total_count":{},"secrets":[{}]}}"#,
            existing.len(),
            names.join(",")
        ))
        .create();
    server
        .mock("GET", "/repos/o/r/actions/secrets/public-key")
        .with_status(200)
        .with_body(format!(r#"{{"key_id":"kid","key":"{PUBLIC_KEY}"}}"#))
        .create();
    server
}

const ARGS: &[&str] = &[
    "migrate",
    "--from",
    "env-file",
    "--to",
    "github-actions",
    "--repo",
    "o/r",
    "--github-token",
    "t",
];

fn project() -> Project {
    Project::new().env("GOOD_ONE=1\nBAD_ONE=2\n")
}

/// At a terminal, answering the confirmation with `answer`.
fn migrate_answering(server: &mockito::ServerGuard, answer: &[&str]) -> (String, i32) {
    let p = project();
    let url = server.url();
    let mut s = Pty::spawn_with(p.path(), ARGS, &[("GITHUB_API_URL", url.as_str())]);
    s.wait_for("Proceed with migration?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(answer);
    s.finish()
}

fn migrate(server: &mockito::ServerGuard) -> (String, i32) {
    // ⚠️ "y" then Enter: the question is answered on Enter (N-09), like every
    // other yes/no prompt in evnx.
    migrate_answering(server, &["y", ENTER])
}

/// Without a terminal, with `extra` flags.
fn migrate_piped(server: &mockito::ServerGuard, extra: &[&str]) -> support::Output {
    let p = project();
    let url = server.url();
    let mut args = ARGS.to_vec();
    args.extend_from_slice(extra);
    run_with(p.path(), &args, &[("GITHUB_API_URL", url.as_str())], None)
}

fn put(server: &mut mockito::ServerGuard, name: &str, hits: usize) -> mockito::Mock {
    server
        .mock("PUT", format!("/repos/o/r/actions/secrets/{name}").as_str())
        .with_status(201)
        .expect(hits)
        .create()
}

#[test]
fn t_i106_a_failed_upload_exits_1_and_is_named() {
    let server = github(Some("BAD_ONE"));
    let (out, code) = migrate(&server);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("1 failed"), "{out}");
    assert!(out.contains("BAD_ONE"), "{out}");
}

#[test]
fn t_i106_every_upload_succeeding_exits_0() {
    let server = github(None);
    let (out, code) = migrate(&server);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn t_i109_no_terminal_and_no_yes_stops_before_github() {
    let mut server = mockito::Server::new();
    let any = server.mock("GET", mockito::Matcher::Any).expect(0).create();
    let out = migrate_piped(&server, &[]);
    out.assert_code(2);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);
    assert!(out.stderr.contains("No verdict"), "{}", out.stderr);
    any.assert();
}

#[test]
fn t_i109_yes_uploads_without_a_terminal() {
    let mut server = github_with(&[]);
    let good = put(&mut server, "GOOD_ONE", 1);
    let bad = put(&mut server, "BAD_ONE", 1);
    let out = migrate_piped(&server, &["--yes"]);
    out.assert_code(0);
    good.assert();
    bad.assert();
}

/// `--yes` answers "Proceed?", not "overwrite a secret that is already
/// there?". That needs its own flag, so nothing is replaced by habit.
#[test]
fn t_i109_yes_does_not_overwrite_existing_secrets() {
    let mut server = github_with(&["GOOD_ONE"]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 0);
    let out = migrate_piped(&server, &["--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("GOOD_ONE"), "{}", out.stderr);
    assert!(out.stderr.contains("--overwrite"), "{}", out.stderr);
    assert!(out.stderr.contains("--skip-existing"), "{}", out.stderr);
    good.assert();
    bad.assert();
}

/// T-MIG-28: `--skip-existing` leaves an existing secret alone and says so.
#[test]
fn t_mig_28_skip_existing_uploads_only_new_secrets() {
    let mut server = github_with(&["GOOD_ONE"]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 1);
    let out = migrate_piped(&server, &["--yes", "--skip-existing"]);
    out.assert_code(0);
    assert!(out.stdout.contains("1 skipped"), "{}", out.stdout);
    good.assert();
    bad.assert();
}

#[test]
fn t_i109_esc_at_the_confirmation_uploads_nothing() {
    let mut server = github_with(&[]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 0);
    // ⚠️ Not `press(&[ESC, ENTER])`: queued together they are read as one
    // unknown escape sequence and the Enter is lost, so the question never
    // returns — it hung on a slow CI runner (#156, again here in U8d's CI).
    // `cancel_yes_no` waits for the redraw between the two.
    let p = project();
    let url = server.url();
    let mut s = Pty::spawn_with(p.path(), ARGS, &[("GITHUB_API_URL", url.as_str())]);
    s.wait_for("Proceed with migration?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.cancel_yes_no();
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing was uploaded"), "{out}");
    good.assert();
    bad.assert();
}
