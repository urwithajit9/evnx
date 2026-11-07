//! RM-55 (X3): a cloud failure keeps its type to `main` and exits with its own
//! code, 10–15, instead of the generic 2.
//!
//! 10 not signed in / session or token refused · 11 forbidden · 12 plan limit ·
//! 13 server unreachable or answered by something that is not evnx ·
//! 14 conflict or locked · 15 server error or rate limited.
#![cfg(feature = "cloud")]

mod support;

use support::cloud::signed_in;
use support::Project;

/// `evnx vault list` against a server whose `GET /api/v1/vaults` answers
/// `status` with `body`.
fn list_with(status: usize, headers: &[(&str, &str)], body: &str) -> support::Output {
    let mut s = mockito::Server::new();
    let mut m = s.mock("GET", "/api/v1/vaults").with_status(status);
    for (k, v) in headers {
        m = m.with_header(*k, v);
    }
    m.with_body(body).create();
    // A 401 tries one refresh; answer it with the same refusal.
    s.mock("POST", "/api/v1/auth/refresh")
        .with_status(401)
        .with_body(r#"{"error":"Invalid refresh token","code":"UNAUTHORIZED"}"#)
        .create();
    signed_in(&Project::new(), &s.url(), &["vault", "list"], None)
}

#[test]
fn a_401_exits_10() {
    let out = list_with(
        401,
        &[],
        r#"{"error":"Unauthorized","code":"UNAUTHORIZED"}"#,
    );
    out.assert_code(10);
    assert!(out.stderr.contains("evnx auth login"), "{}", out.stderr);
}

#[test]
fn not_signed_in_exits_10() {
    let p = Project::new();
    let cfg = p.path().join("empty-cfg");
    let out = support::run_with(
        p.path(),
        &["vault", "list"],
        &[
            ("EVNX_SERVER", "http://127.0.0.1:9"),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        None,
    );
    out.assert_code(10);
    assert!(out.stderr.contains("not signed in"), "{}", out.stderr);
}

#[test]
fn a_403_with_a_role_exits_11_and_names_the_role() {
    let out = list_with(
        403,
        &[],
        r#"{"error":"Forbidden","code":"FORBIDDEN","reason":"ROLE_TOO_LOW","required_role":"admin"}"#,
    );
    out.assert_code(11);
    assert!(out.stderr.contains("admin"), "{}", out.stderr);
    assert!(out.stderr.contains("ask the vault owner"), "{}", out.stderr);
}

#[test]
fn a_403_from_an_older_server_exits_11_with_its_own_sentence() {
    let out = list_with(403, &[], r#"{"error":"Forbidden","code":"FORBIDDEN"}"#);
    out.assert_code(11);
    assert!(!out.stderr.contains("role"), "{}", out.stderr);
}

#[test]
fn a_plan_limit_exits_12() {
    for status in [403, 409] {
        let out = list_with(
            status,
            &[],
            r#"{"error":"your plan allows 3 vaults.","code":"PLAN_LIMIT"}"#,
        );
        out.assert_code(12);
        assert!(out.stderr.contains("allows 3 vaults"), "{}", out.stderr);
    }
}

#[test]
fn an_unreachable_server_exits_13() {
    // A port that was free a moment ago and has nothing listening on it now.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let out = signed_in(&Project::new(), &url, &["vault", "list"], None);
    out.assert_code(13);
    assert!(out.stderr.contains("could not reach"), "{}", out.stderr);
}

#[test]
fn a_proxy_page_exits_13() {
    let out = list_with(
        502,
        &[("content-type", "text/html")],
        "<html>Bad Gateway</html>",
    );
    out.assert_code(13);
    assert!(out.stderr.contains("not from evnx"), "{}", out.stderr);
}

#[test]
fn a_409_exits_14() {
    let out = list_with(409, &[], r#"{"error":"Conflict","code":"CONFLICT"}"#);
    out.assert_code(14);
}

#[test]
fn a_500_exits_15_and_a_429_says_when_to_retry() {
    let out = list_with(500, &[], r#"{"error":"boom","code":"INTERNAL_ERROR"}"#);
    out.assert_code(15);

    let out = list_with(
        429,
        &[("retry-after", "30")],
        r#"{"error":"Too many requests","code":"RATE_LIMITED"}"#,
    );
    out.assert_code(15);
    assert!(out.stderr.contains("30 s"), "{}", out.stderr);
}

#[test]
fn the_message_is_printed_once() {
    let out = list_with(
        401,
        &[],
        r#"{"error":"Unauthorized","code":"UNAUTHORIZED"}"#,
    );
    assert_eq!(
        out.stderr.matches("session has expired").count(),
        1,
        "{}",
        out.stderr
    );
}
