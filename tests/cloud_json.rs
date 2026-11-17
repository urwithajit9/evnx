//! WP-9b: `--format json` on the cloud list commands, and `status --check`.
//!
//! Each command is run against a mock server and its stdout must be exactly
//! one JSON document with the `run` header and the array its plural noun names.
#![cfg(feature = "cloud")]

mod support;

use serde_json::Value;
use support::cloud::signed_in;
use support::{run_with, Output, Project};

const ORG_ID: &str = "11111111-1111-1111-1111-111111111111";
const VAULT_ID: &str = "22222222-2222-2222-2222-222222222222";

fn doc(out: &Output) -> Value {
    out.assert_code(0);
    let v: Value = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("not one JSON document ({e}):\n{}", out.stdout));
    assert_eq!(v["run"]["tool"], "evnx", "{v}");
    assert_eq!(v["run"]["version"], env!("CARGO_PKG_VERSION"), "{v}");
    assert!(v["run"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "cloud"));
    assert!(v["run"].get("timestamp").is_none());
    v
}

fn org_server() -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/orgs")
        .with_status(200)
        .with_body(format!(
            r#"{{"organizations":[{{"id":"{ORG_ID}","name":"Acme","slug":"acme","plan":"team",
            "your_role":"owner","you_hold_a_seat":true,"seats":{{"used":1,"purchased":5}}}}]}}"#
        ))
        .create();
    s
}

fn vault_json() -> String {
    format!(
        r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
        "role":"owner","version_count":2,"updated_at":"2026-10-16T00:00:00Z"}}]}}"#
    )
}

#[test]
fn org_list_json() {
    let s = org_server();
    let out = signed_in(&Project::new(), &s.url(), &["org", "list", "--json"], None);
    let v = doc(&out);
    assert_eq!(v["organizations"][0]["slug"], "acme");
    assert_eq!(v["organizations"][0]["seats"]["purchased"], 5);
}

#[test]
fn org_members_json() {
    let mut s = org_server();
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[{"user_id":"u-me","email":"me@example.com","role":"owner",
            "holds_a_seat":true,"is_you":true}]}"#,
        )
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "members", "--format", "json"],
        None,
    );
    let v = doc(&out);
    assert_eq!(v["members"][0]["email"], "me@example.com");
    assert_eq!(v["members"][0]["is_you"], true);
}

#[test]
fn org_invites_json_has_an_empty_array_not_prose() {
    let mut s = org_server();
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/invites").as_str())
        .with_status(200)
        .with_body(r#"{"invites":[]}"#)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "invites", "--json"],
        None,
    );
    assert_eq!(doc(&out)["invites"].as_array().unwrap().len(), 0);
}

#[test]
fn org_billing_json() {
    let mut s = org_server();
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/billing").as_str())
        .with_status(200)
        .with_body(
            r#"{"plan":"team","seats":{"used":1,"purchased":5},"over_seated":false,
            "your_role":"owner","billing_configured":true,
            "subscription":{"exists":true,"status":"active",
            "current_period_ends_at":"2026-12-01T00:00:00Z","scheduled_change":null}}"#,
        )
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "billing", "--json"],
        None,
    );
    let v = doc(&out);
    assert_eq!(v["billing"]["plan"], "team");
    assert_eq!(v["billing"]["subscription"]["status"], "active");
}

#[test]
fn vault_list_and_members_json() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(vault_json())
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[{"user_id":"u-me","email":"me@example.com","role":"owner",
            "granted_at":"2026-10-01T00:00:00Z","has_mlkem_key":true,"is_you":true}]}"#,
        )
        .create();
    let p = Project::new();
    let v = doc(&signed_in(&p, &s.url(), &["vault", "list", "--json"], None));
    assert_eq!(v["vaults"][0]["name"], "app");
    assert_eq!(v["vaults"][0]["version_count"], 2);
    let v = doc(&signed_in(
        &p,
        &s.url(),
        &["vault", "members", "app/production", "--json"],
        None,
    ));
    assert_eq!(v["members"][0]["role"], "owner");
}

fn versions_server() -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
    )
    .with_status(200)
    .with_body(
        r#"{"versions":[
            {"version_num":1,"key_count":2,"key_names":["A","B"],"blob_hash":"h1",
             "blob_size_bytes":120,"pushed_by":"u-me","pushed_at":"2026-10-01T00:00:00Z"},
            {"version_num":2,"key_count":3,"key_names":["A","B","C"],"blob_hash":"h2",
             "blob_size_bytes":150,"pushed_by":"u-me","pushed_at":"2026-10-02T00:00:00Z"}]}"#,
    )
    .create();
    s
}

#[test]
fn cloud_history_json_is_newest_first_and_limited() {
    let s = versions_server();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "cloud", "history", "--vault", VAULT_ID, "--limit", "1", "--json",
        ],
        None,
    );
    let v = doc(&out);
    let rows = v["versions"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["version_num"], 2);
    assert_eq!(rows[0]["blob_size_bytes"], 150);
}

#[test]
fn cloud_diff_json_names_only() {
    let s = versions_server();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["cloud", "diff", "--vault", VAULT_ID, "--json"],
        None,
    );
    let v = doc(&out);
    assert_eq!(v["from"], 1);
    assert_eq!(v["to"], 2);
    assert_eq!(v["added"], serde_json::json!(["C"]));
    assert_eq!(v["removed"], serde_json::json!([]));
    assert_eq!(v["changed"], serde_json::json!([]));
}

#[test]
fn cloud_audit_json() {
    let mut s = mockito::Server::new();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/audit").as_str())
        .with_status(200)
        .with_body(
            r#"{"events":[{"event_type":"pull","actor_email":"me@example.com","metadata":null,
            "created_at":"2026-10-01T00:00:00Z"}],"limit":100,"retention_days":null,
            "hidden_by_retention":0}"#,
        )
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["cloud", "audit", "--vault", VAULT_ID, "--json"],
        None,
    );
    let v = doc(&out);
    assert_eq!(v["events"][0]["event_type"], "pull");
    assert_eq!(v["hidden_by_retention"], 0);
}

#[test]
fn auth_sessions_and_devices_json() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/sessions")
        .with_status(200)
        .with_body(
            r#"{"sessions":[{"session_id":"s-1","created_at":"2026-10-01T00:00:00Z",
            "last_used_at":"2026-10-02T00:00:00Z","expires_at":"2026-11-01T00:00:00Z",
            "current":true}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/devices")
        .with_status(200)
        .with_body(
            r#"{"devices":[{"id":"d-1","first_seen":"2026-10-01T00:00:00Z",
            "last_seen":"2026-10-02T00:00:00Z","sign_in_count":3,"is_current":true,
            "disavowed":false,"unknown_origin":false}]}"#,
        )
        .create();
    let p = Project::new();
    let v = doc(&signed_in(
        &p,
        &s.url(),
        &["auth", "sessions", "list", "--json"],
        None,
    ));
    assert_eq!(v["sessions"][0]["current"], true);
    let v = doc(&signed_in(
        &p,
        &s.url(),
        &["auth", "devices", "list", "--json"],
        None,
    ));
    assert_eq!(v["devices"][0]["sign_in_count"], 3);
}

// ── status --check ───────────────────────────────────────────────────────────

fn status(p: &Project, server: &str, signed: bool, args: &[&str]) -> Output {
    if signed {
        return signed_in(p, server, args, None);
    }
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    run_with(
        p.path(),
        args,
        &[
            ("EVNX_SERVER", server),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        None,
    )
}

#[test]
fn status_check_exits_0_when_signed_in_and_10_when_not() {
    let s = mockito::Server::new();
    for cmd in [["auth", "status"], ["cloud", "status"]] {
        let mut args = cmd.to_vec();
        args.push("--check");
        status(&Project::new(), &s.url(), true, &args).assert_code(0);
        status(&Project::new(), &s.url(), false, &args).assert_code(10);
    }
}

#[test]
fn status_check_quiet_prints_nothing() {
    let s = mockito::Server::new();
    for cmd in [["auth", "status"], ["cloud", "status"]] {
        let mut args = cmd.to_vec();
        args.extend(["--check", "-q"]);
        let out = status(&Project::new(), &s.url(), false, &args);
        out.assert_code(10);
        assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));
        let out = status(&Project::new(), &s.url(), true, &args);
        out.assert_code(0);
        assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));
    }
}

#[test]
fn status_check_json_shape() {
    let s = mockito::Server::new();
    for cmd in [["auth", "status"], ["cloud", "status"]] {
        let mut args = cmd.to_vec();
        args.extend(["--check", "--json"]);
        let out = status(&Project::new(), &s.url(), true, &args);
        out.assert_code(0);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["signed_in"], true);
        assert_eq!(v["user"], "me@example.com");
        assert_eq!(v["server"], s.url());
        assert_eq!(v["run"]["tool"], "evnx");

        let out = status(&Project::new(), &s.url(), false, &args);
        out.assert_code(10);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["signed_in"], false);
        assert!(v.get("user").is_none(), "{v}");
        assert_eq!(out.stderr, "");
    }
}

#[test]
fn status_without_check_keeps_exit_0_when_signed_out() {
    let s = mockito::Server::new();
    status(&Project::new(), &s.url(), false, &["auth", "status"]).assert_code(0);
    status(&Project::new(), &s.url(), false, &["cloud", "status"]).assert_code(0);
}
