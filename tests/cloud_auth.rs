//! `evnx auth` fixes (U7 Part C: AU-01, AU-02).
//!
//! Every test runs against a local mock server with an empty config directory.
//! Where the claim is "refused before any request", the mock requires zero
//! requests of any method.
//!
//! - I-211, DEC-AU3: with `EVNX_TOKEN` set, `rotate-master-password`,
//!   `download-data` and `delete-account` went to the server; the other account
//!   commands refused locally. T-SHARE-51: one table for all of them.
//! - I-204, DEC-AU1: `sessions revoke ""` matched every session by prefix and,
//!   with one session, signed the machine out. A target under 4 characters is
//!   refused before anything is listed.
//! - I-212, DEC-AU10: `status` did not say which credential it used, and
//!   `logout` with only `EVNX_TOKEN` said "nothing to do" while the token stays
//!   valid until revoked.
#![cfg(feature = "cloud")]

mod support;

use support::{run_with, Project};

const TOKEN: &str = "evnx_tok_00000000000000000000000000000000";

/// A mock server that fails the test if it receives any request.
fn silent_server() -> (mockito::ServerGuard, Vec<mockito::Mock>) {
    let mut server = mockito::Server::new();
    let mocks = ["GET", "POST", "PUT", "PATCH", "DELETE"]
        .iter()
        .map(|m| server.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    (server, mocks)
}

fn evnx(p: &Project, server: &str, token: Option<&str>, args: &[&str]) -> support::Output {
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let mut env = vec![
        ("EVNX_SERVER", server.to_string()),
        ("EVNX_CONFIG_DIR", cfg.to_str().unwrap().to_string()),
    ];
    if let Some(t) = token {
        env.push(("EVNX_TOKEN", t.to_string()));
    }
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_with(p.path(), args, &env, None)
}

/// T-SHARE-51: an API token is refused, before any request, by every command
/// that manages the account.
#[test]
fn t_i211_account_management_refuses_an_api_token_before_any_request() {
    let p = Project::new();
    let (server, mocks) = silent_server();
    for args in [
        &["auth", "rotate-master-password"][..],
        &["auth", "download-data"][..],
        &["auth", "delete-account", "--confirm-stdin"][..],
        &["auth", "sessions", "list"][..],
        &["auth", "token", "list"][..],
        &["auth", "devices", "list"][..],
        &["auth", "totp", "enable"][..],
        &["org", "list"][..],
    ] {
        let out = evnx(&p, &server.url(), Some(TOKEN), args);
        out.assert_code(2);
        assert!(
            out.stderr.contains("an API token cannot") && out.stderr.contains("deliberate"),
            "{args:?}:\n{}",
            out.stderr
        );
    }
    for m in mocks {
        m.assert();
    }
}

#[test]
fn t_i204_a_short_or_empty_session_target_is_refused_before_any_request() {
    let p = Project::new();
    let (server, mocks) = silent_server();
    for target in ["", "   ", "abc"] {
        let out = evnx(
            &p,
            &server.url(),
            None,
            &["auth", "sessions", "revoke", target, "-y"],
        );
        out.assert_code(2);
        assert!(
            out.stderr.contains("at least 4 characters"),
            "{target:?}:\n{}",
            out.stderr
        );
    }
    for m in mocks {
        m.assert();
    }
}

/// DEC-AU10: `status` names the credential it used.
#[test]
fn t_i212_status_says_it_used_an_api_token() {
    let p = Project::new();
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(
            r#"{"user_id":"u-1","email":"ci@example.com","email_verified":true,"totp_enabled":false}"#,
        )
        .create();
    let out = evnx(&p, &server.url(), Some(TOKEN), &["auth", "status"]);
    out.assert_code(0);
    assert!(out.stdout.contains("API token"), "{}", out.stdout);
    assert!(out.stdout.contains("EVNX_TOKEN"), "{}", out.stdout);
}

/// DEC-AU10: `logout` cannot end an API token, and says how it ends.
#[test]
fn t_i212_logout_with_only_a_token_says_how_to_revoke_it() {
    let p = Project::new();
    let (server, mocks) = silent_server();
    let out = evnx(&p, &server.url(), Some(TOKEN), &["auth", "logout"]);
    out.assert_code(0);
    assert!(
        out.all().contains("evnx auth token revoke"),
        "{}",
        out.all()
    );
    assert!(!out.all().contains("nothing to do"), "{}", out.all());
    for m in mocks {
        m.assert();
    }
}

/// I-209, DEC-AU6: revoking the session this machine uses also removes the
/// stored login. It printed "You are now signed out on this machine" while
/// `credentials.json` still held the dead session.
#[test]
fn t_i209_revoking_the_current_session_signs_this_machine_out() {
    use evnx::cloud::creds::{SecretString, Session, Store};
    let p = Project::new();
    let mut server = mockito::Server::new();
    let url = server.url();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let mut store = Store::default();
    store.set_session(
        &url,
        Session {
            email: "me@example.com".into(),
            access_token: SecretString::new("access"),
            access_expires_at: 4_000_000_000,
            refresh_token: SecretString::new("refresh"),
        },
    );
    store.save_to(&cfg.join("credentials.json")).unwrap();

    server
        .mock("GET", "/api/v1/auth/sessions")
        .with_status(200)
        .with_body(
            r#"{"sessions":[{"session_id":"abcd1111-0000-0000-0000-000000000000",
            "created_at":"2026-10-01T00:00:00Z","last_used_at":"2026-10-16T00:00:00Z",
            "expires_at":"2026-11-01T00:00:00Z","current":true}]}"#,
        )
        .create();
    let del = server
        .mock(
            "DELETE",
            "/api/v1/auth/sessions/abcd1111-0000-0000-0000-000000000000",
        )
        .with_status(204)
        .expect(1)
        .create();

    let out = evnx(
        &p,
        &url,
        None,
        &["auth", "sessions", "revoke", "abcd1111", "-y"],
    );
    out.assert_code(0);
    del.assert();
    let after = Store::load_from(&cfg.join("credentials.json")).unwrap();
    assert!(
        after.session(&url).is_none(),
        "the dead session is still stored:\n{}",
        out.all()
    );
}
