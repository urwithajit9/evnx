//! Every cloud confirmation, asked with no terminal, says how to confirm (I-218).
//!
//! 0.10.3 moved `org` onto `prompt::confirm`; `vault delete`, `revoke` and
//! `rekey`, `cloud delete-version`, `auth sessions revoke`, `revoke-others` and
//! `auth token revoke` still called dialoguer directly, so a script without
//! `--yes` failed with "IO error: not a terminal: not a terminal". Nothing is
//! sent in any of these cases: the question comes before the change.
#![cfg(feature = "cloud")]

mod support;

use support::cloud::signed_in;
use support::Project;

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

fn vault_server() -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"owner","version_count":2,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
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
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/versions").as_str())
        .with_status(200)
        .with_body(
            r#"{"versions":[
            {"version_num":2,"key_count":1,"key_names":["A"],"blob_hash":"h2","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-21T09:00:00Z"},
            {"version_num":1,"key_count":1,"key_names":["A"],"blob_hash":"h1","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-20T09:00:00Z"}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/sessions")
        .with_status(200)
        .with_body(
            r#"{"sessions":[
            {"session_id":"aaaaaaaa-1111-2222-3333-444444444444","created_at":"2026-10-20T09:00:00Z","last_used_at":"2026-10-20T09:00:00Z","expires_at":"2026-11-20T09:00:00Z","current":true},
            {"session_id":"bbbbbbbb-1111-2222-3333-444444444444","created_at":"2026-10-20T09:00:00Z","last_used_at":"2026-10-20T09:00:00Z","expires_at":"2026-11-20T09:00:00Z","current":false}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/tokens")
        .with_status(200)
        .with_body(
            r#"{"tokens":[{"id":"t-1","name":"ci","scope":"read","vault_id":null,
            "expires_at":null,"created_at":"2026-10-20T09:00:00Z","last_used_at":null}]}"#,
        )
        .create();
    s
}

#[test]
fn t_i218_every_cloud_confirmation_names_yes_without_a_terminal() {
    for (args, example) in [
        (
            &["vault", "delete", "app"][..],
            "evnx vault delete app --yes",
        ),
        (
            &["vault", "revoke", "app", "--user", "bob@example.com"][..],
            "evnx vault revoke app --user bob@example.com --yes",
        ),
        (&["vault", "rekey", "app"][..], "evnx vault rekey app --yes"),
        (
            &["cloud", "delete-version", "1", "--vault", "app"][..],
            "evnx cloud delete-version 1 --vault app --yes",
        ),
        (
            &["auth", "sessions", "revoke", "bbbbbbbb"][..],
            "evnx auth sessions revoke bbbbbbbb --yes",
        ),
        (
            &["auth", "sessions", "revoke-others"][..],
            "evnx auth sessions revoke-others --yes",
        ),
        (
            &["auth", "token", "revoke", "ci"][..],
            "evnx auth token revoke ci --yes",
        ),
    ] {
        let mut s = vault_server();
        let changes: Vec<_> = ["POST", "DELETE", "PATCH", "PUT"]
            .iter()
            .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
            .collect();
        let password = s.mock("GET", "/api/v1/auth/me").expect(0).create();
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(2);
        assert!(
            !out.stderr.contains("not a terminal: not a terminal"),
            "{args:?}:\n{}",
            out.stderr
        );
        assert!(out.stderr.contains("--yes"), "{args:?}:\n{}", out.stderr);
        assert!(out.stderr.contains(example), "{args:?}:\n{}", out.stderr);
        for m in changes {
            m.assert();
        }
        password.assert();
    }
}
