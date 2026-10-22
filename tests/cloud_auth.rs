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
//! - I-321: a customer whose organisation pays for Team, and who holds no seat
//!   in it, saw `plan (free)` with no reason. Paying for seats hands none out —
//!   an organisation's owner is created without one — so `status` now names
//!   the organisation and the way to a seat.
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

// ─── I-321: a paid organisation you hold no seat in ──────────────────────────

const ME: &str =
    r#"{"user_id":"u-1","email":"me@example.com","email_verified":true,"totp_enabled":false}"#;

/// `GET /auth/usage` for a free account, or one whose plan comes from `via`.
fn usage_body(plan: &str, via: Option<(&str, &str)>) -> String {
    let via = match via {
        Some((name, slug)) => format!(r#"{{"id":"o-9","name":"{name}","slug":"{slug}"}}"#),
        None => "null".into(),
    };
    format!(
        r#"{{"plan":"{plan}","plan_via":{via},"vaults":{{"used":3,"limit":3}},
            "api_tokens":{{"used":0,"limit":2}},
            "versions_per_vault":{{"limit":5,"vaults":[]}}}}"#
    )
}

/// One `GET /orgs` entry.
fn org(
    slug: &str,
    plan: &str,
    role: &str,
    seat: bool,
    used: i64,
    purchased: Option<i64>,
) -> String {
    let purchased = purchased.map_or("null".to_string(), |p| p.to_string());
    format!(
        r#"{{"id":"id-{slug}","name":"{slug} Corp","slug":"{slug}","plan":"{plan}",
            "your_role":"{role}","you_hold_a_seat":{seat},
            "seats":{{"used":{used},"purchased":{purchased}}}}}"#
    )
}

/// `evnx auth status` signed in as me@example.com, against `/auth/me`,
/// `/auth/usage` and `/orgs` answering as given (`None`: `/orgs` fails).
fn status_with(usage: &str, orgs: Option<&[String]>, args: &[&str]) -> support::Output {
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
        .mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(ME)
        .create();
    server
        .mock("GET", "/api/v1/auth/usage")
        .with_status(200)
        .with_body(usage)
        .create();
    match orgs {
        Some(list) => server
            .mock("GET", "/api/v1/orgs")
            .with_status(200)
            .with_body(format!(r#"{{"organizations":[{}]}}"#, list.join(",")))
            .create(),
        None => server
            .mock("GET", "/api/v1/orgs")
            .with_status(500)
            .with_body(r#"{"error":"Internal server error","code":"INTERNAL_ERROR"}"#)
            .create(),
    };
    let mut full = vec!["auth", "status"];
    full.extend_from_slice(args);
    let out = evnx(&p, &url, None, &full);
    drop(server);
    out
}

/// The reported case: the owner paid, holds no seat, and a seat is free.
#[test]
fn t_i321_status_says_a_paid_org_gives_you_nothing_without_a_seat() {
    let out = status_with(
        &usage_body("free", None),
        Some(&[org("acme", "team", "owner", false, 0, Some(3))]),
        &[],
    );
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("acme is on the team plan, but you hold no seat there"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("so your own free plan applies"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("evnx org assign me@example.com --org acme"),
        "{}",
        out.stdout
    );
}

#[test]
fn t_i321_status_with_every_seat_taken_names_the_two_ways_out() {
    let out = status_with(
        &usage_body("free", None),
        Some(&[org("acme", "team", "admin", false, 1, Some(1))]),
        &[],
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("The only seat is assigned"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("evnx org release"), "{}", out.stdout);
    assert!(
        out.stdout.contains("app.evnx.dev/billing"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("evnx org assign"), "{}", out.stdout);
}

/// A member cannot assign, so the command would only be refused.
#[test]
fn t_i321_status_tells_a_member_who_to_ask() {
    let out = status_with(
        &usage_body("free", None),
        Some(&[org("acme", "team", "member", false, 0, Some(3))]),
        &[],
    );
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("Ask an owner or an admin of acme to assign you one"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("evnx org assign"), "{}", out.stdout);
}

/// One seat per person: the seat elsewhere has to go first.
#[test]
fn t_i321_status_names_the_seat_you_hold_elsewhere() {
    let out = status_with(
        &usage_body("free", Some(("beta Corp", "beta"))),
        Some(&[
            org("acme", "team", "owner", false, 0, Some(3)),
            org("beta", "free", "owner", true, 1, None),
        ]),
        &[],
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("so beta's free plan applies"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Your seat is in beta"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("evnx org release me@example.com --org beta"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("evnx org assign"), "{}", out.stdout);
}

/// Nothing to explain: holding the seat, or a free organisation, or `-q`, or
/// `/orgs` failing — and `status` still succeeds in every case.
#[test]
fn t_i321_status_says_nothing_when_there_is_nothing_to_explain() {
    let seated = status_with(
        &usage_body("team", Some(("acme Corp", "acme"))),
        Some(&[org("acme", "team", "owner", true, 1, Some(3))]),
        &[],
    );
    let free_org = status_with(
        &usage_body("free", None),
        Some(&[org("acme", "free", "owner", false, 0, None)]),
        &[],
    );
    let quiet = status_with(
        &usage_body("free", None),
        Some(&[org("acme", "team", "owner", false, 0, Some(3))]),
        &["-q"],
    );
    let failed = status_with(&usage_body("free", None), None, &[]);
    for (case, out) in [
        ("seated", seated),
        ("free org", free_org),
        ("-q", quiet),
        ("/orgs failed", failed),
    ] {
        out.assert_code(0);
        assert!(
            !out.stdout.contains("hold no seat"),
            "{case}:\n{}",
            out.stdout
        );
        assert!(out.stdout.contains("plan ("), "{case}:\n{}", out.stdout);
    }
}
