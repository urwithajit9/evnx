//! `evnx migrate --to github-actions` reports failed uploads in its exit code
//! (I-106, U4 pilot).
//!
//! An upload GitHub refused was listed under "✗ 1 failed" and the run exited
//! **0**, so a pipeline step that migrated half its secrets passed. A run that
//! finishes with failures is now exit 1 (the answer is "no"); a run that cannot
//! start — no token, no repository, GitHub unreachable — stays 2.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use support::pty::Pty;
use support::Project;

/// 32 bytes, base64: a key GitHub's sealed-box encryption accepts.
const PUBLIC_KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";

fn github(refuse: Option<&str>) -> mockito::ServerGuard {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/repos/o/r/actions/secrets")
        .with_status(200)
        .with_body(r#"{"total_count":0,"secrets":[]}"#)
        .create();
    server
        .mock("GET", "/repos/o/r/actions/secrets/public-key")
        .with_status(200)
        .with_body(format!(r#"{{"key_id":"kid","key":"{PUBLIC_KEY}"}}"#))
        .create();
    for name in ["GOOD_ONE", "BAD_ONE"] {
        let status = if refuse == Some(name) { 500 } else { 201 };
        server
            .mock("PUT", format!("/repos/o/r/actions/secrets/{name}").as_str())
            .with_status(status)
            .create();
    }
    server
}

fn migrate(server: &mockito::ServerGuard) -> (String, i32) {
    let p = Project::new().env("GOOD_ONE=1\nBAD_ONE=2\n");
    let url = server.url();
    let mut s = Pty::spawn_with(
        p.path(),
        &[
            "migrate",
            "--from",
            "env-file",
            "--to",
            "github-actions",
            "--repo",
            "o/r",
            "--github-token",
            "t",
        ],
        &[("GITHUB_API_URL", url.as_str())],
    );
    s.wait_for("Proceed with migration?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(&["y"]);
    s.finish()
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
