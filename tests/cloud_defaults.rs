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

// ─── vault share and delete ──────────────────────────────────────────────────

fn vault_server(role: &str, members: &str) -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"{role}","version_count":3,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(format!(r#"{{"members":[{members}]}}"#))
        .create();
    s
}

fn member(email: &str, role: &str, you: bool) -> String {
    format!(
        r#"{{"user_id":"u-{email}","email":"{email}","role":"{role}",
        "granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":{you}}}"#
    )
}

fn members_of(bob_role: &str) -> String {
    format!(
        "{},{}",
        member("me@example.com", "owner", true),
        member("bob@example.com", bob_role, false)
    )
}

#[test]
fn wp12_share_defaults_to_viewer() {
    // A member who already is a viewer: "nothing changed" is only said when the
    // requested role, here the default, equals theirs. No key lookup, no POST.
    let mut s = vault_server("owner", &members_of("viewer"));
    let none = s.mock("POST", Matcher::Any).expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "share",
            "app/production",
            "--with",
            "bob@example.com",
        ],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("already a viewer") && out.stdout.contains("nothing changed"),
        "{}",
        out.stdout
    );
    none.assert();
}

#[test]
fn wp12_share_default_is_named_in_help() {
    let out = support::run(Project::new().path(), &["vault", "share", "--help"]);
    assert!(out.stdout.contains("[default: viewer]"), "{}", out.stdout);
}

#[test]
fn wp12_share_explicit_developer_still_works() {
    let s = vault_server("owner", &members_of("developer"));
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "share",
            "app/production",
            "--with",
            "bob@example.com",
            "--role",
            "developer",
        ],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("already a developer"), "{}", out.stdout);
}

fn delete_mock(s: &mut mockito::ServerGuard, hits: usize) -> mockito::Mock {
    s.mock("DELETE", format!("/api/v1/vaults/{VAULT_ID}").as_str())
        .with_status(204)
        .expect(hits)
        .create()
}

#[test]
fn wp12_delete_yes_with_bare_name_refused_exit_2() {
    let mut s = vault_server("owner", &members_of("viewer"));
    let del = delete_mock(&mut s, 0);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "delete", "app", "--yes"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("full name/environment"),
        "{}",
        out.stderr
    );
    del.assert();
}

#[test]
fn wp12_delete_yes_with_full_name_deletes() {
    let mut s = vault_server("owner", &members_of("viewer"));
    let del = delete_mock(&mut s, 1);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "delete", "app/production", "--yes"],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("deleted app/production"),
        "{}",
        out.stdout
    );
    del.assert();
}

#[test]
fn wp12_delete_yes_with_an_id_deletes() {
    let mut s = vault_server("owner", &members_of("viewer"));
    let del = delete_mock(&mut s, 1);
    signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "delete", VAULT_ID, "-y"],
        None,
    )
    .assert_code(0);
    del.assert();
}

#[test]
fn wp12_delete_no_terminal_no_yes_exit_4() {
    let mut s = vault_server("owner", &members_of("viewer"));
    let del = delete_mock(&mut s, 0);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "delete", "app/production"],
        None,
    );
    out.assert_code(4);
    assert!(
        out.stderr
            .contains("evnx vault delete app/production --yes"),
        "{}",
        out.stderr
    );
    del.assert();
}

#[cfg(unix)]
mod delete_prompt {
    use super::*;
    use support::pty::Pty;

    fn run_delete(typed: &str) -> (String, i32, bool) {
        let mut s = vault_server("owner", &members_of("viewer"));
        let hits = usize::from(typed == "app/production");
        let del = delete_mock(&mut s, hits);
        let p = Project::new();
        let cfg = p.path().join("cfg");
        support::cloud::login(&cfg, &s.url());
        let url = s.url();
        let mut run = Pty::spawn_with(
            p.path(),
            &["vault", "delete", "app"],
            &[
                ("EVNX_SERVER", url.as_str()),
                ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ],
        );
        run.wait_for("Type app/production to confirm");
        run.type_line(typed);
        let (rest, code) = run.finish();
        del.assert();
        let deleted = rest.contains("deleted app/production");
        (rest, code, deleted)
    }

    #[test]
    fn wp12_delete_prompt_requires_typed_full_name() {
        let (rest, code, deleted) = run_delete("app/production");
        assert_eq!(code, 0, "{rest}");
        assert!(deleted, "{rest}");
    }

    #[test]
    fn wp12_delete_wrong_typed_name_cancels() {
        for typed in ["app", "y", "app/staging"] {
            let (rest, code, deleted) = run_delete(typed);
            assert_eq!(code, 0, "{typed}: {rest}");
            assert!(!deleted, "{typed}: {rest}");
            assert!(rest.contains("Cancelled; nothing was deleted"), "{rest}");
        }
    }
}
