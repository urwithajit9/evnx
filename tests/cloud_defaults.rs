//! WP-12: safer defaults for the cloud commands, against a mock server.
//!
//! One section per area: tokens, vault share and delete, org, cloud push.
//! A test never puts a secret value in the output it checks.
#![cfg(feature = "cloud")]

mod support;

use mockito::Matcher;
use support::cloud::signed_in;
use support::Project;

const VAULT_ID: &str = "22222222-2222-2222-2222-222222222222";

// ─── tokens ──────────────────────────────────────────────────────────────────

fn token_mock(s: &mut mockito::ServerGuard, body: Matcher) -> mockito::Mock {
    s.mock("POST", "/api/v1/auth/tokens")
        .match_body(body)
        .with_status(201)
        .with_body(r#"{"id":"t-1","raw_token":"evnx_tok_example","expires_at":null}"#)
        .create()
}

fn token_create(s: &mockito::ServerGuard, extra: &[&str]) -> support::Output {
    let mut args = vec!["auth", "token", "create", "ci", "--no-key-bundle"];
    args.extend_from_slice(extra);
    signed_in(&Project::new(), &s.url(), &args, None)
}

#[test]
fn wp12_token_default_sends_90_days() {
    let mut s = mockito::Server::new();
    let m = token_mock(
        &mut s,
        Matcher::PartialJson(serde_json::json!({"expires_in_days": 90, "vault_id": VAULT_ID})),
    );
    let out = token_create(&s, &["--vault", VAULT_ID]);
    out.assert_code(0);
    m.assert();
}

#[test]
fn wp12_explicit_days_are_sent() {
    let mut s = mockito::Server::new();
    let m = token_mock(
        &mut s,
        Matcher::PartialJson(serde_json::json!({"expires_in_days": 7})),
    );
    token_create(&s, &["--vault", VAULT_ID, "--expires-in-days", "7"]).assert_code(0);
    m.assert();
}

#[test]
fn wp12_no_expiry_omits_the_field_and_warns() {
    let mut s = mockito::Server::new();
    let bad = s
        .mock("POST", "/api/v1/auth/tokens")
        .match_body(Matcher::Regex("expires_in_days".into()))
        .expect(0)
        .create();
    let ok = s
        .mock("POST", "/api/v1/auth/tokens")
        .with_status(201)
        .with_body(r#"{"id":"t-1","raw_token":"evnx_tok_example","expires_at":null}"#)
        .create();
    let out = token_create(&s, &["--vault", VAULT_ID, "--no-expiry"]);
    out.assert_code(0);
    bad.assert();
    ok.assert();
    assert!(out.stdout.contains("never expires"), "{}", out.stdout);
}

#[test]
fn wp12_expires_flag_conflicts_with_no_expiry() {
    let mut s = mockito::Server::new();
    let none = s.mock("POST", Matcher::Any).expect(0).create();
    let out = token_create(
        &s,
        &["--vault", VAULT_ID, "--expires-in-days", "5", "--no-expiry"],
    );
    out.assert_code(2);
    none.assert();
}

#[test]
fn wp12_token_needs_vault_or_all_vaults() {
    let mut s = mockito::Server::new();
    let none = s.mock("POST", Matcher::Any).expect(0).create();
    let out = token_create(&s, &[]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("--vault") && out.stderr.contains("--all-vaults"),
        "{}",
        out.stderr
    );
    none.assert();
}

#[test]
fn wp12_vault_conflicts_with_all_vaults() {
    let mut s = mockito::Server::new();
    let none = s.mock("POST", Matcher::Any).expect(0).create();
    token_create(&s, &["--vault", VAULT_ID, "--all-vaults"]).assert_code(2);
    none.assert();
}

#[test]
fn wp12_all_vaults_sends_no_vault_id_and_warns() {
    let mut s = mockito::Server::new();
    let bad = s
        .mock("POST", "/api/v1/auth/tokens")
        .match_body(Matcher::Regex("vault_id".into()))
        .expect(0)
        .create();
    let ok = s
        .mock("POST", "/api/v1/auth/tokens")
        .match_body(Matcher::PartialJson(
            serde_json::json!({"expires_in_days": 90}),
        ))
        .with_status(201)
        .with_body(
            r#"{"id":"t-1","raw_token":"evnx_tok_example","expires_at":"2027-02-16T00:00:00Z"}"#,
        )
        .create();
    let out = token_create(&s, &["--all-vaults"]);
    out.assert_code(0);
    bad.assert();
    ok.assert();
    assert!(out.stdout.contains("every vault"), "{}", out.stdout);
    assert!(!out.stdout.contains("never expires"), "{}", out.stdout);
}

#[test]
fn wp12_token_list_shows_expiry_and_scope() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/tokens")
        .with_status(200)
        .with_body(
            r#"{"tokens":[{"id":"t-1","name":"ci","scope":"read","vault_id":null,
            "expires_at":"2027-02-16T00:00:00Z","created_at":"2026-11-18T10:00:00Z","last_used_at":null}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(r#"{"vaults":[]}"#)
        .create();
    let out = signed_in(&Project::new(), &s.url(), &["auth", "token", "list"], None);
    out.assert_code(0);
    assert!(out.stdout.contains("2027-02-16"), "{}", out.stdout);
    assert!(out.stdout.contains("read"), "{}", out.stdout);
}
