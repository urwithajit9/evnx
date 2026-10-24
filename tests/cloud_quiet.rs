//! `-q` for the cloud commands (I-225, DEC-CL3, A-44).
//!
//! `-q` reached only the config banner and `cloud status`/`unlink`: `vault`,
//! `org`, `auth token`, `sessions` and the rest printed every status line. Now a
//! command that only reports what it did prints nothing on success, and one that
//! answers a question still answers it. (The commands that need real keys —
//! `vault create`, `cloud push`, `pull` — are in `cloud_unlock.rs`.)
#![cfg(feature = "cloud")]

mod support;

use support::cloud::signed_in;
use support::Project;

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

fn server() -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"owner","version_count":0,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[
            {"user_id":"u-me","email":"me@example.com","role":"owner","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":true},
            {"user_id":"u-bob","email":"bob@example.com","role":"developer","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":false}]}"#,
        )
        .create();
    s
}

/// Status-only commands print nothing on success under `-q`.
#[test]
fn t_i225_quiet_status_commands_print_nothing_on_success() {
    let mut s = server();
    s.mock("DELETE", format!("/api/v1/vaults/{VAULT_ID}").as_str())
        .with_status(204)
        .create();
    s.mock("POST", "/api/v1/orgs")
        .with_status(201)
        .with_body(r#"{"id":"o-1","slug":"acme"}"#)
        .create();
    s.mock("POST", "/api/v1/auth/logout")
        .with_status(204)
        .create();
    for args in [
        &["-q", "vault", "delete", "app", "-y"][..],
        // The role bob already holds: "nothing changed", a status line.
        &["-q", "vault", "share", "app", "--with", "bob@example.com"][..],
        &["-q", "org", "create", "Acme", "--slug", "acme"][..],
        &["-q", "auth", "logout"][..],
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(0);
        assert_eq!(out.stdout, "", "{args:?} printed on stdout");
        assert_eq!(out.stderr, "", "{args:?} printed on stderr");
    }
}

/// A question is still answered under `-q` (DEC-G3): the vault table, the
/// member list.
#[test]
fn t_i225_quiet_keeps_the_answer() {
    let s = server();
    let out = signed_in(&Project::new(), &s.url(), &["-q", "vault", "list"], None);
    out.assert_code(0);
    assert!(out.stdout.contains("app"), "{}", out.stdout);
    assert!(out.stdout.contains("NAME"), "{}", out.stdout);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["-q", "vault", "members", "app"],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("bob@example.com"), "{}", out.stdout);
}

/// `auth token create -q` prints the token and nothing else, so a script can
/// take it from stdout.
#[test]
fn t_i225_quiet_token_create_prints_only_the_token() {
    let mut s = mockito::Server::new();
    s.mock("POST", "/api/v1/auth/tokens")
        .with_status(201)
        .with_body(r#"{"id":"t-1","raw_token":"evnx_tok_example","expires_at":null}"#)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["-q", "auth", "token", "create", "ci"],
        None,
    );
    out.assert_code(0);
    assert_eq!(out.stdout.trim(), "evnx_tok_example", "{}", out.stdout);
}
