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

// ─── org seats and accept ────────────────────────────────────────────────────

const ORG_ID: &str = "11111111-1111-1111-1111-111111111111";

fn org_server(subscription: bool) -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/orgs")
        .with_status(200)
        .with_body(format!(
            r#"{{"organizations":[{{"id":"{ORG_ID}","name":"Acme","slug":"acme","plan":"team",
            "your_role":"owner","you_hold_a_seat":true,"seats":{{"used":2,"purchased":5}}}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/billing").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"plan":"team","seats":{{"used":2,"purchased":5}},"over_seated":false,
            "your_role":"owner","billing_configured":true,
            "subscription":{{"exists":{subscription},"status":null,"current_period_ends_at":null,"scheduled_change":null}}}}"#
        ))
        .create();
    s
}

fn seats_put(s: &mut mockito::ServerGuard, hits: usize) -> mockito::Mock {
    s.mock("PUT", format!("/api/v1/orgs/{ORG_ID}/seats").as_str())
        .with_status(200)
        .with_body(r#"{"seats":{"used":2,"purchased":8},"over_seated":false}"#)
        .expect(hits)
        .create()
}

#[test]
fn wp12_seats_set_without_terminal_exits_4_and_sends_nothing() {
    let mut s = org_server(false);
    let put = seats_put(&mut s, 0);
    // A piped "y" is not consent.
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "seats", "--set", "8"],
        Some(b"y\n"),
    );
    out.assert_code(4);
    assert!(
        out.stderr.contains("evnx org seats --set 8 --yes"),
        "{}",
        out.stderr
    );
    assert!(
        out.stdout.contains("seats purchased 5 -> 8"),
        "{}",
        out.stdout
    );
    put.assert();
}

#[test]
fn wp12_seats_set_yes_puts() {
    let mut s = org_server(false);
    let put = seats_put(&mut s, 1);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "seats", "--set", "8", "-y"],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("seats purchased 5 -> 8"),
        "{}",
        out.stdout
    );
    put.assert();
}

#[test]
fn wp12_seats_unlimited_asks_too() {
    let mut s = org_server(false);
    let put = seats_put(&mut s, 0);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "seats", "--unlimited"],
        None,
    );
    out.assert_code(4);
    assert!(out.stderr.contains("--unlimited --yes"), "{}", out.stderr);
    put.assert();
}

#[test]
fn wp12_seats_bare_never_writes() {
    let mut s = org_server(false);
    let put = seats_put(&mut s, 0);
    let out = signed_in(&Project::new(), &s.url(), &["org", "seats"], None);
    out.assert_code(0);
    assert!(out.stdout.contains("2 / 5"), "{}", out.stdout);
    put.assert();
}

#[test]
fn wp12_seats_subscription_path_unchanged() {
    let mut s = org_server(true);
    let put = seats_put(&mut s, 0);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "seats", "--set", "8", "--yes"],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("has a subscription"), "{}", out.stdout);
    put.assert();
}

#[cfg(unix)]
#[test]
fn wp12_seats_cancel_at_the_prompt_sends_nothing() {
    use support::pty::Pty;
    let mut s = org_server(false);
    let put = seats_put(&mut s, 0);
    let p = Project::new();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let url = s.url();
    let mut run = Pty::spawn_with(
        p.path(),
        &["org", "seats", "--set", "8"],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    run.wait_for("Change the seat count of acme?");
    run.type_line("n");
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Cancelled"), "{rest}");
    put.assert();
}

const INVITE: &str = "evnx_inv_0123456789abcdef";

fn accept_mock(s: &mut mockito::ServerGuard, hits: usize) -> mockito::Mock {
    s.mock("POST", "/api/v1/orgs/invites/accept")
        .match_body(Matcher::Json(serde_json::json!({ "token": INVITE })))
        .with_status(200)
        .with_body(r#"{"organization":"Acme","role":"member"}"#)
        .expect(hits)
        .create()
}

#[test]
fn wp12_accept_token_from_stdin() {
    let mut s = mockito::Server::new();
    let m = accept_mock(&mut s, 1);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "accept", "--token-stdin"],
        Some(format!("{INVITE}\n").as_bytes()),
    );
    out.assert_code(0);
    m.assert();
    assert!(out.stdout.contains("joined Acme"), "{}", out.stdout);
    assert!(!out.stderr.contains("shell history"), "{}", out.stderr);
}

#[test]
fn wp12_accept_positional_warns_but_works() {
    let mut s = mockito::Server::new();
    let m = accept_mock(&mut s, 1);
    let out = signed_in(&Project::new(), &s.url(), &["org", "accept", INVITE], None);
    out.assert_code(0);
    m.assert();
    assert!(out.stderr.contains("shell history"), "{}", out.stderr);
    assert!(out.stderr.contains("--token-stdin"), "{}", out.stderr);
    // The warning never repeats the token.
    assert!(!out.stderr.contains(INVITE), "{}", out.stderr);
}

#[test]
fn wp12_accept_no_input_exit_2() {
    let mut s = mockito::Server::new();
    let m = accept_mock(&mut s, 0);
    let out = signed_in(&Project::new(), &s.url(), &["org", "accept"], Some(b""));
    out.assert_code(2);
    assert!(out.stderr.contains("--token-stdin"), "{}", out.stderr);
    m.assert();
    // An empty stdin line is no input either.
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "accept", "--token-stdin"],
        Some(b"\n"),
    );
    out.assert_code(2);
    m.assert();
}

#[test]
fn wp12_accept_positional_conflicts_with_stdin_flag() {
    let mut s = mockito::Server::new();
    let m = accept_mock(&mut s, 0);
    signed_in(
        &Project::new(),
        &s.url(),
        &["org", "accept", INVITE, "--token-stdin"],
        None,
    )
    .assert_code(2);
    m.assert();
}
