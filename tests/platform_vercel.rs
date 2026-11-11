//! `evnx platform diff|pull --to vercel` against a mock Vercel server.
//!
//! The mock speaks what the live test found (design §2a): the list never has a
//! value, one key can have several entries with different targets, a Config value
//! comes from `GET /v1/projects/{id}/env/{envId}`, and a Secret has no `value`
//! there.
//!
//! ⚠️ Every test asserts that no value (local, remote, ciphertext or the token)
//! appears in stdout or stderr: `quiet()` is the check, and a test that skips it
//! says why.
#![cfg(feature = "net")]

mod support;

use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{json, Value};
use support::{run_with, Output, Project};

const TOKEN: &str = "vcp_test_token_9f8e7d6c5b4a";

// Remote values (Config only: a Secret has none to leak).
const R_API: &str = "remote-api-value-7c1d";
const R_LOG: &str = "remote-log-debug-77";
const R_NEW: &str = "remote-new-value-d4e5";
const R_PROD_API: &str = "remote-PROD-api-value-0f0f";
// Local values.
const L_API_OLD: &str = "old-local-api-value-aa11";
const L_DB: &str = "keep-me-local-db-value-bb22";
const L_ONLY: &str = "local-only-value-cc33";
const L_NEW: &str = "local-new-value-1d4c";
// What the list carries in `value` (ciphertext for Config).
const CIPHER: &str = "ciphertext-from-the-list-ZXCV";

const ALL_VALUES: &[&str] = &[
    TOKEN, R_API, R_LOG, R_NEW, R_PROD_API, L_API_OLD, L_DB, L_ONLY, L_NEW, CIPHER,
];

fn env_entry(id: &str, key: &str, ty: &str, target: &[&str]) -> Value {
    let secret = ty == "sensitive";
    json!({
        "id": id, "key": key, "type": ty, "target": target,
        "value": if secret { "" } else { CIPHER },
        "decrypted": false,
        "createdAt": 1_700_000_000_000u64,
        "updatedAt": 1_700_000_000_000u64 + id.len() as u64,
    })
}

fn standard_envs() -> Vec<Value> {
    vec![
        env_entry("e1", "API_URL", "encrypted", &["development", "preview"]),
        env_entry("e1b", "API_URL", "encrypted", &["production"]),
        env_entry("e2", "DB_URL", "sensitive", &["development"]),
        env_entry("e3", "LOG_LEVEL", "plain", &["development"]),
        env_entry("e4", "NEW_REMOTE", "encrypted", &["development"]),
        env_entry("e5", "REDIS_URL", "sensitive", &["development"]),
    ]
}

/// A mock Vercel with the standard project. Keeps the mocks whose call counts
/// matter.
struct Vercel {
    server: ServerGuard,
    /// Value reads that must not happen (a Secret, another environment's entry).
    never: Vec<Mock>,
    /// Value reads whose count is part of the test.
    once: Vec<Mock>,
}

fn list_query(team: &str) -> Matcher {
    Matcher::Exact(format!("teamId={team}"))
}

impl Vercel {
    /// Pull reads every Config value in the environment, e4 included.
    fn start() -> Vercel {
        Vercel::with_list(
            json!({ "envs": standard_envs(), "pagination": { "count": 6 } }),
            1,
        )
    }

    /// Diff reads only what it compares: e4 is on Vercel only, so it is not read.
    fn start_for_diff() -> Vercel {
        Vercel::with_list(
            json!({ "envs": standard_envs(), "pagination": { "count": 6 } }),
            0,
        )
    }

    fn with_list(body: Value, e4_reads: usize) -> Vercel {
        let mut server = Server::new();
        server
            .mock("GET", "/v10/projects/prj_1/env")
            .match_query(list_query("team_1"))
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_status(200)
            .with_body(body.to_string())
            .create();
        let mut v = Vercel {
            server,
            never: vec![],
            once: vec![],
        };
        for (id, value, hits) in [("e1", R_API, 1), ("e3", R_LOG, 1), ("e4", R_NEW, e4_reads)] {
            let m = v.value_mock(id, 200, Some(value), hits);
            v.once.push(m);
        }
        // A Secret is never read, and neither is another environment's entry.
        for id in ["e2", "e5", "e1b"] {
            let m = v.value_mock(id, 200, None, 0);
            v.never.push(m);
        }
        v
    }

    fn value_mock(&mut self, id: &str, status: usize, value: Option<&str>, hits: usize) -> Mock {
        let body = match value {
            Some(v) => json!({ "id": id, "value": v, "decrypted": true }),
            None => json!({ "id": id, "decrypted": false }),
        };
        self.server
            .mock("GET", format!("/v1/projects/prj_1/env/{id}").as_str())
            .match_query(list_query("team_1"))
            .with_status(status)
            .with_body(body.to_string())
            .expect(hits)
            .create()
    }

    fn url(&self) -> String {
        self.server.url()
    }

    fn assert_reads(&self) {
        for m in self.once.iter().chain(&self.never) {
            m.assert();
        }
    }
}

fn run(p: &Project, url: &str, args: &[&str]) -> Output {
    run_with(
        p.path(),
        args,
        &[
            ("VERCEL_TOKEN", TOKEN),
            ("EVNX_VERCEL_API_URL", url),
            ("VERCEL_PROJECT_ID", ""),
            ("VERCEL_ORG_ID", ""),
        ],
        None,
    )
}

const COMMON: &[&str] = &["--to", "vercel", "--project", "prj_1", "--team", "team_1"];

fn diff_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["platform", "diff"];
    a.extend_from_slice(COMMON);
    a.extend_from_slice(extra);
    a
}

fn pull_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["platform", "pull"];
    a.extend_from_slice(COMMON);
    a.extend_from_slice(extra);
    a
}

/// No value anywhere a person or a log could read it.
#[track_caller]
fn quiet(out: &Output) {
    for v in ALL_VALUES {
        out.assert_not_contains(v);
    }
}

fn diff_project() -> Project {
    Project::new().file(
        ".env.development",
        &format!("API_URL={L_API_OLD}\nDB_URL={L_DB}\nLOG_LEVEL={R_LOG}\nBRAND_NEW={L_NEW}\n"),
    )
}

// ── diff ─────────────────────────────────────────────────────────────────────

#[test]
fn diff_picks_the_entry_for_the_environment_and_prints_the_plan() {
    let v = Vercel::start_for_diff();
    let p = diff_project();
    let out = run(&p, &v.url(), &diff_args(&[]));
    out.assert_code(1);
    quiet(&out);
    v.assert_reads();
    support::snapshot::check_golden(
        "platform/diff-development.txt",
        &support::snapshot::normalise(&out.stdout, &[p.path()]),
    );
    assert_eq!(out.stderr, "", "no warnings expected");
}

#[test]
fn diff_json_has_keys_kinds_and_actions_only() {
    let v = Vercel::start_for_diff();
    let p = diff_project();
    let out = run(&p, &v.url(), &diff_args(&["--format", "json"]));
    out.assert_code(1);
    quiet(&out);
    let j: Value = serde_json::from_str(&out.stdout).expect("stdout is JSON and nothing else");
    assert_eq!(j["differs"], true);
    assert_eq!(j["summary"]["add"], 1);
    assert_eq!(j["summary"]["update"], 1);
    assert_eq!(j["summary"]["unchanged"], 1);
    assert_eq!(j["summary"]["present"], 1);
    assert_eq!(j["summary"]["remote_only"], 2);
    support::snapshot::check_golden(
        "platform/diff-development.json",
        &support::snapshot::normalise(&out.stdout, &[p.path()]),
    );
}

/// One Secret on both sides, nothing else: the F1 case.
fn secret_only_remote() -> ServerGuard {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(
            json!({ "envs": [env_entry("e2", "DB_URL", "sensitive", &["development"])] })
                .to_string(),
        )
        .create();
    s
}

#[test]
fn f1_a_secret_on_both_sides_is_not_a_difference() {
    let s = secret_only_remote();
    let p = Project::new().file(".env.development", &format!("DB_URL={L_DB}\n"));
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("= DB_URL");
    out.assert_contains("secret  present  (write-only, not compared)");
    out.assert_contains("1 present (not compared) · values never shown");
    out.assert_not_contains("~ DB_URL");
}

#[test]
fn f1_strict_restores_the_difference_and_exit_1() {
    let s = secret_only_remote();
    let p = Project::new().file(".env.development", &format!("DB_URL={L_DB}\n"));
    let out = run(&p, &s.url(), &diff_args(&["--strict"]));
    out.assert_code(1);
    quiet(&out);
    out.assert_contains("~ DB_URL");
    out.assert_contains("value cannot be compared: write-only");
}

#[test]
fn f1_json_tells_present_from_unchanged_and_strict_from_not() {
    let s = secret_only_remote();
    let p = Project::new().file(".env.development", &format!("DB_URL={L_DB}\n"));
    let out = run(&p, &s.url(), &diff_args(&["--format", "json"]));
    out.assert_code(0);
    quiet(&out);
    let j: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(j["differs"], false);
    assert_eq!(j["strict"], false);
    assert_eq!(j["summary"]["present"], 1);
    assert_eq!(j["summary"]["update"], 0);
    assert_eq!(j["entries"][0]["action"], "unchanged");
    assert_eq!(j["entries"][0]["compare"], "write_only");

    let out = run(&p, &s.url(), &diff_args(&["--format", "json", "--strict"]));
    out.assert_code(1);
    let j: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(j["differs"], true);
    assert_eq!(j["strict"], true);
    assert_eq!(j["summary"]["present"], 0);
    assert_eq!(j["entries"][0]["action"], "update");
    assert_eq!(j["entries"][0]["compare"], "write_only");
}

#[test]
fn diff_with_nothing_different_exits_0() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(
            json!({ "envs": [env_entry("e3", "LOG_LEVEL", "plain", &["development"])] })
                .to_string(),
        )
        .create();
    s.mock("GET", "/v1/projects/prj_1/env/e3")
        .match_query(list_query("team_1"))
        .with_body(json!({ "id": "e3", "value": R_LOG }).to_string())
        .create();
    let p = Project::new().file(".env.development", &format!("LOG_LEVEL={R_LOG}\n"));
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("1 unchanged · values never shown");
}

#[test]
fn diff_uses_the_production_entry_for_production() {
    let mut v = Vercel::start_for_diff();
    // For production the Config read is the other API_URL entry (e1b).
    let prod = v.value_mock("e1b", 200, Some(R_PROD_API), 1);
    let p = Project::new().file(".env.production", &format!("API_URL={R_PROD_API}\n"));
    let out = run(&p, &v.url(), &diff_args(&["--env-name", "production"]));
    // Same value on both sides: API_URL is unchanged; the other variables are
    // not in production, so none is listed.
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("= API_URL");
    out.assert_contains("local  .env.production");
    prod.assert();
}

#[test]
fn diff_says_which_file_it_used() {
    let v = Vercel::start();
    // No .env.development: falls back to .env, and says so in the header.
    let p = Project::new().file(".env", "LOG_LEVEL=x\n");
    let out = run(&p, &v.url(), &diff_args(&[]));
    quiet(&out);
    out.assert_contains("local  .env\n");
}

#[test]
fn f3_no_local_file_means_an_empty_local_side() {
    let v = Vercel::start_for_diff();
    let p = Project::new();
    let out = run(&p, &v.url(), &diff_args(&["--env-name", "development"]));
    // Five development variables, all remote only: a difference, but not an error.
    out.assert_code(1);
    quiet(&out);
    out.assert_contains("neither .env.development nor .env exists");
    out.assert_contains("every Vercel variable shows as remote only");
    out.assert_contains("local  .env\n");
    assert_eq!(
        out.stdout.matches("remote only").count(),
        6,
        "{}",
        out.stdout
    );
    out.assert_contains("5 remote only");
    assert!(!out.stdout.contains("+ "), "{}", out.stdout);

    let out = run(&p, &v.url(), &diff_args(&["--format", "json"]));
    out.assert_code(1);
    let j: Value = serde_json::from_str(&out.stdout).expect("stdout is JSON and nothing else");
    assert_eq!(j["local_file_found"], false);
    assert_eq!(j["summary"]["remote_only"], 5);
    quiet(&out);
}

#[test]
fn f3_nothing_on_vercel_and_no_local_file_is_clean_and_a_missing_explicit_file_is_an_error() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(json!({ "envs": [] }).to_string())
        .create();
    let p = Project::new();
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(0);
    out.assert_contains("neither");
    // --file names a path on purpose: a missing one is a typo.
    let out = run(&p, &s.url(), &diff_args(&["--file", "nope.env"]));
    out.assert_code(2);
    out.assert_contains("nope.env not found");
}

#[test]
fn a_config_variable_vercel_will_not_reveal_is_write_only() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(
            json!({ "envs": [
                env_entry("e9", "ODD", "encrypted", &["development"]),
                // `visibility: secret` with a legacy-looking type.
                { "id": "e8", "key": "VIS", "type": "encrypted", "visibility": "secret",
                  "target": ["development"], "value": "" },
            ] })
            .to_string(),
        )
        .create();
    s.mock("GET", "/v1/projects/prj_1/env/e9")
        .match_query(list_query("team_1"))
        .with_body(json!({ "id": "e9", "decrypted": false }).to_string())
        .create();
    let never = s
        .mock("GET", "/v1/projects/prj_1/env/e8")
        .match_query(list_query("team_1"))
        .expect(0)
        .create();
    let p = Project::new().file(".env.development", "ODD=a\nVIS=b\n");
    // F1: both are Secrets on both sides, so present and not a difference...
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(0);
    out.assert_contains("= ODD");
    out.assert_contains("= VIS");
    assert_eq!(
        out.stdout
            .matches("present  (write-only, not compared)")
            .count(),
        2,
        "{}",
        out.stdout
    );
    // ...unless --strict asks for the old reading.
    let out = run(&p, &s.url(), &diff_args(&["--strict"]));
    out.assert_code(1);
    out.assert_contains("~ ODD");
    out.assert_contains("~ VIS");
    assert_eq!(
        out.stdout
            .matches("value cannot be compared: write-only")
            .count(),
        2,
        "{}",
        out.stdout
    );
    never.assert();
}

#[test]
fn diff_follows_pagination_next() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(Matcher::Exact("teamId=team_1".into()))
        .with_body(
            json!({
                "envs": [env_entry("e3", "LOG_LEVEL", "plain", &["development"])],
                "pagination": { "count": 1, "next": 1_700_000_000_555u64 }
            })
            .to_string(),
        )
        .create();
    let page2 = s
        .mock("GET", "/v10/projects/prj_1/env")
        .match_query(Matcher::Exact("teamId=team_1&until=1700000000555".into()))
        .with_body(
            json!({
                "envs": [env_entry("e5", "REDIS_URL", "sensitive", &["development"])],
                "pagination": { "count": 1, "next": null }
            })
            .to_string(),
        )
        .expect(1)
        .create();
    s.mock("GET", "/v1/projects/prj_1/env/e3")
        .match_query(list_query("team_1"))
        .with_body(json!({ "id": "e3", "value": R_LOG }).to_string())
        .create();
    let p = Project::new().file(".env.development", &format!("LOG_LEVEL={R_LOG}\n"));
    let out = run(&p, &s.url(), &diff_args(&[]));
    // LOG_LEVEL from page one, REDIS_URL (remote only) from page two.
    out.assert_code(1);
    out.assert_contains("= LOG_LEVEL");
    out.assert_contains("! REDIS_URL");
    quiet(&out);
    page2.assert();
}

#[test]
fn hidden_production_variables_are_warned_about() {
    let v = Vercel::with_list(
        json!({
            "envs": standard_envs(),
            "pagination": { "count": 6 },
            "hiddenProductionEnvCount": 3
        }),
        0,
    );
    let p = diff_project();
    let out = run(&p, &v.url(), &diff_args(&[]));
    quiet(&out);
    assert!(
        out.stderr
            .contains("3 production variables are hidden from this token"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_secret_under_a_public_prefix_and_a_credential_in_config_are_type_differences() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(
            json!({ "envs": [
                env_entry("a", "NEXT_PUBLIC_SITE", "sensitive", &["development"]),
                env_entry("b", "AUTH_SECRET", "encrypted", &["development"]),
            ] })
            .to_string(),
        )
        .create();
    s.mock("GET", "/v1/projects/prj_1/env/b")
        .match_query(list_query("team_1"))
        .with_body(json!({ "id": "b", "value": "abc" }).to_string())
        .create();
    let p = Project::new().file(".env.development", "NEXT_PUBLIC_SITE=x\nAUTH_SECRET=abc\n");
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(1);
    assert_eq!(
        out.stdout.matches("type differs").count(),
        3,
        "{}",
        out.stdout
    );
    out.assert_contains("2 type differs");
}

// ── failures and exit codes ──────────────────────────────────────────────────

fn list_status(status: usize, headers: &[(&str, &str)], body: &str, hits: usize) -> (Output, Mock) {
    let mut s = Server::new();
    let mut m = s
        .mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_status(status);
    for (k, v) in headers {
        m = m.with_header(*k, v);
    }
    let m = m.with_body(body).expect(hits).create();
    let p = Project::new().file(".env.development", "A=1\n");
    let out = run(&p, &s.url(), &diff_args(&[]));
    // The server must outlive the call; leak it into the mock handle's scope.
    std::mem::forget(s);
    (out, m)
}

fn unix_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

#[test]
fn a_401_exits_10() {
    let (out, _) = list_status(
        401,
        &[],
        r#"{"error":{"code":"forbidden","message":"x"}}"#,
        1,
    );
    out.assert_code(10);
    out.assert_contains("refused the token");
    out.assert_contains("VERCEL_TOKEN");
    quiet(&out);
}

#[test]
fn a_403_exits_11_with_the_servers_sentence() {
    let (out, _) = list_status(
        403,
        &[],
        r#"{"error":{"code":"forbidden","message":"Not authorized to access this project"}}"#,
        1,
    );
    out.assert_code(11);
    out.assert_contains("Not authorized to access this project");
    quiet(&out);
}

#[test]
fn a_403_with_invalid_token_exits_10_and_says_to_create_one() {
    let (out, _) = list_status(
        403,
        &[],
        r#"{"error":{"code":"forbidden","message":"Not authorized","invalidToken":true}}"#,
        1,
    );
    out.assert_code(10);
    out.assert_contains("the token is invalid or expired; create one at vercel.com/account/tokens");
    out.assert_not_contains("may not read that");
    out.assert_not_contains("(--team");
    quiet(&out);
    // `invalidToken: false` is an ordinary 403.
    let (out, _) = list_status(
        403,
        &[],
        r#"{"error":{"code":"forbidden","message":"No","invalidToken":false}}"#,
        1,
    );
    out.assert_code(11);
}

#[test]
fn a_404_is_an_error_naming_the_flags() {
    let (out, _) = list_status(
        404,
        &[],
        r#"{"error":{"code":"not_found","message":"x"}}"#,
        1,
    );
    out.assert_code(2);
    out.assert_contains("--project");
}

#[test]
fn a_500_exits_15_and_is_not_retried() {
    let (out, m) = list_status(500, &[], "boom", 1);
    out.assert_code(15);
    out.assert_contains("server error");
    m.assert();
}

#[test]
fn a_429_waits_for_the_reset_then_succeeds() {
    let mut s = Server::new();
    let limited = s
        .mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_status(429)
        .with_header("x-ratelimit-remaining", "0")
        .with_header("x-ratelimit-reset", &unix_now())
        .with_body(r#"{"error":{"code":"rate_limited","message":"slow down"}}"#)
        .expect(1)
        .create();
    let ok = s
        .mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(json!({ "envs": [] }).to_string())
        .expect(1)
        .create();
    let p = Project::new().file(".env.development", "A=1\n");
    let out = run(&p, &s.url(), &diff_args(&[]));
    out.assert_code(1); // A is local only
    out.assert_contains("1 add");
    limited.assert();
    ok.assert();
}

#[test]
fn a_429_that_never_clears_gives_up_after_three_tries_with_exit_15() {
    let (out, m) = list_status(
        429,
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &unix_now()),
        ],
        r#"{"error":{"code":"rate_limited","message":"slow down"}}"#,
        3,
    );
    out.assert_code(15);
    out.assert_contains("3 tries");
    m.assert();
}

#[test]
fn an_unreachable_server_exits_13() {
    let p = Project::new().file(".env.development", "A=1\n");
    // Port 9 (discard) refuses connections on loopback.
    let out = run(&p, "http://127.0.0.1:9", &diff_args(&[]));
    out.assert_code(13);
    out.assert_contains("could not reach");
    quiet(&out);
}

#[test]
fn plain_http_to_a_real_host_is_refused_before_the_token_is_sent() {
    let p = Project::new().file(".env.development", "A=1\n");
    let out = run(&p, "http://example.com", &diff_args(&[]));
    out.assert_code(2);
    out.assert_contains("refusing plain http://");
}

// ── token and project ────────────────────────────────────────────────────────

#[test]
fn no_token_names_both_ways_and_there_is_no_token_flag() {
    let p = diff_project();
    let out = run_with(
        p.path(),
        &diff_args(&[]),
        &[
            ("VERCEL_TOKEN", ""),
            ("VERCEL_PROJECT_ID", ""),
            ("VERCEL_ORG_ID", ""),
        ],
        None,
    );
    out.assert_code(2);
    out.assert_contains("VERCEL_TOKEN");
    out.assert_contains("--token-file");
    // And the flag really is absent.
    let mut a = diff_args(&[]);
    a.extend(["--token", "abc"]);
    let out = run(&p, "http://127.0.0.1:9", &a);
    out.assert_code(2);
    out.assert_contains("unexpected argument");
}

#[test]
fn a_token_file_is_used_and_beats_the_variable() {
    let mut s = Server::new();
    let m = s
        .mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .match_header("authorization", "Bearer token-from-the-file")
        .with_body(json!({ "envs": [] }).to_string())
        .expect(1)
        .create();
    let p = Project::new()
        .file(".env.development", "A=1\n")
        .file("tok", "token-from-the-file\n");
    let tok = p.join("tok");
    let out = run(
        &p,
        &s.url(),
        &diff_args(&["--token-file", tok.to_str().unwrap()]),
    );
    out.assert_code(1);
    out.assert_not_contains("token-from-the-file");
    out.assert_not_contains(TOKEN);
    m.assert();
}

#[test]
fn the_project_comes_from_flags_then_the_environment_then_the_link_file() {
    let mut s = Server::new();
    let mut hit = |project: &str, team: &str| {
        s.mock("GET", format!("/v10/projects/{project}/env").as_str())
            .match_query(list_query(team))
            .with_body(json!({ "envs": [] }).to_string())
            .expect(1)
            .create()
    };
    let flag = hit("prj_flag", "team_flag");
    let envv = hit("prj_env", "team_env");
    let file = hit("prj_file", "team_file");
    let p = Project::new().file(".env.development", "A=1\n").file(
        ".vercel/project.json",
        r#"{"projectId":"prj_file","orgId":"team_file"}"#,
    );
    let url = s.url();
    let base = |extra: &[(&str, &str)], args: &[&str]| {
        let mut env = vec![
            ("VERCEL_TOKEN", TOKEN),
            ("EVNX_VERCEL_API_URL", url.as_str()),
        ];
        env.extend_from_slice(extra);
        run_with(p.path(), args, &env, None)
    };
    let envs = [
        ("VERCEL_PROJECT_ID", "prj_env"),
        ("VERCEL_ORG_ID", "team_env"),
    ];
    // Flags win over the environment.
    base(
        &envs,
        &[
            "platform",
            "diff",
            "--to",
            "vercel",
            "--project",
            "prj_flag",
            "--team",
            "team_flag",
        ],
    )
    .assert_code(1);
    // The environment wins over the file.
    base(&envs, &["platform", "diff", "--to", "vercel"]).assert_code(1);
    // The file is the last resort.
    base(
        &[("VERCEL_PROJECT_ID", ""), ("VERCEL_ORG_ID", "")],
        &["platform", "diff", "--to", "vercel"],
    )
    .assert_code(1);
    for m in [flag, envv, file] {
        m.assert();
    }
}

#[test]
fn no_project_anywhere_names_all_three_ways() {
    let p = Project::new().file(".env.development", "A=1\n");
    let out = run(
        &p,
        "http://127.0.0.1:9",
        &["platform", "diff", "--to", "vercel"],
    );
    out.assert_code(2);
    for needle in ["--project", "VERCEL_PROJECT_ID", ".vercel/project.json"] {
        out.assert_contains(needle);
    }
}

#[test]
fn only_vercel_is_accepted_for_to() {
    let p = Project::new();
    let out = run(
        &p,
        "http://127.0.0.1:9",
        &["platform", "diff", "--to", "netlify"],
    );
    out.assert_code(2);
    out.assert_contains("vercel");
}

// ── pull ─────────────────────────────────────────────────────────────────────

const PULL_BEFORE: &str = "\
# Pulled from Vercel
API_URL=old-local-api-value-aa11   # dev server
export LOG_LEVEL=\"remote-log-debug-77\"
DB_URL=keep-me-local-db-value-bb22
LOCAL_ONLY=local-only-value-cc33
";

fn pull_project() -> Project {
    Project::new().file(".env.development", PULL_BEFORE)
}

#[test]
fn pull_writes_config_keeps_secrets_and_local_only_and_keeps_the_formatting() {
    let v = Vercel::start();
    let p = pull_project();
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet(&out);
    v.assert_reads();

    assert_eq!(
        p.read(".env.development"),
        format!(
            "# Pulled from Vercel\n\
             API_URL={R_API}   # dev server\n\
             export LOG_LEVEL=\"{R_LOG}\"\n\
             DB_URL={L_DB}\n\
             LOCAL_ONLY={L_ONLY}\n\
             NEW_REMOTE={R_NEW}\n\
             REDIS_URL=\n"
        )
    );
    // Secrets: the one with a local value kept it; the missing one is listed.
    out.assert_contains("Written empty");
    out.assert_contains("REDIS_URL");
    assert!(
        !out.stdout.contains(
            "Written empty (write-only on Vercel, so evnx cannot fetch the value): DB_URL"
        ),
        "{}",
        out.stdout
    );
    // The previous file is kept.
    assert_eq!(p.read(".env.development.bak"), PULL_BEFORE);
    support::snapshot::check_golden(
        "platform/pull-development.txt",
        &support::snapshot::normalise(&out.stdout, &[p.path()]),
    );
}

#[test]
fn pull_records_ids_and_timestamps_but_no_values() {
    let v = Vercel::start();
    let p = pull_project();
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    let text = p.read(".evnx/platform/vercel.json");
    for value in ALL_VALUES {
        assert!(!text.contains(value), "{value} in the state file:\n{text}");
    }
    let s = record(&p, "development");
    assert_eq!(s["project"], "prj_1");
    assert_eq!(s["team"], "team_1");
    assert_eq!(s["environment"], "development");
    let entries = s["entries"].as_array().unwrap();
    // Development's five variables: not the production-only API_URL entry.
    let ids: Vec<&str> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["e1", "e2", "e3", "e4", "e5"]);
    let api = &entries[0];
    assert_eq!(api["key"], "API_URL");
    assert_eq!(api["target"], json!(["development", "preview"]));
    assert_eq!(api["type"], "encrypted");
    assert!(api["updatedAt"].is_u64());
    assert_eq!(entries[1]["type"], "sensitive");
    for e in entries {
        let keys: Vec<&String> = e.as_object().unwrap().keys().collect();
        assert_eq!(
            keys.len(),
            5,
            "only key, id, target, type, updatedAt: {keys:?}"
        );
    }
}

#[test]
fn pull_hints_at_gitignore_when_the_state_is_not_ignored() {
    let v = Vercel::start();
    let p = pull_project().git();
    let out = run(&p, &v.url(), &pull_args(&["--yes"]));
    out.assert_code(0);
    assert!(
        out.stderr.contains(".evnx/platform/"),
        "no gitignore hint:\n{}",
        out.stderr
    );
    let p = pull_project()
        .git()
        .file(".gitignore", ".evnx/platform/\n.env*\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes"]));
    out.assert_code(0);
    out.assert_not_contains("per-machine state");
}

#[test]
fn quiet_pull_with_yes_is_silent_and_quiet_diff_still_prints_the_plan() {
    let v = Vercel::start();
    let p = pull_project();
    // A file others can read is a warning `-q` keeps (and evnx keeps the mode).
    #[cfg(unix)]
    let p = p.mode(".env.development", 0o600);
    let mut args = vec!["-q"];
    args.extend(pull_args(&["--yes", "--no-gitignore"]));
    let out = run(&p, &v.url(), &args);
    out.assert_code(0);
    assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));
    assert!(p.read(".env.development").contains(R_NEW));

    let v = Vercel::start_for_diff();
    let p = diff_project();
    let mut args = vec!["-q"];
    args.extend(diff_args(&[]));
    let out = run(&p, &v.url(), &args);
    out.assert_code(1);
    out.assert_contains("values never shown");
    quiet(&out);
}

#[test]
fn pull_dry_run_writes_nothing() {
    let v = Vercel::start();
    let p = pull_project();
    let out = run(&p, &v.url(), &pull_args(&["--dry-run"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("Dry run: nothing was written");
    assert_eq!(p.read(".env.development"), PULL_BEFORE);
    assert!(!p.exists(".env.development.bak"));
    assert!(!p.exists(".evnx"));
}

#[test]
fn pull_without_a_terminal_needs_yes() {
    let v = Vercel::start();
    let p = pull_project();
    let out = run(&p, &v.url(), &pull_args(&["--no-gitignore"]));
    out.assert_code(2);
    out.assert_contains("--yes");
    quiet(&out);
    assert_eq!(p.read(".env.development"), PULL_BEFORE);
    assert!(!p.exists(".evnx"));
}

#[test]
fn pull_creates_a_file_that_does_not_exist() {
    let v = Vercel::start();
    let p = Project::new();
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("file      .env (default)");
    assert_eq!(
        p.read(".env"),
        format!("API_URL={R_API}\nLOG_LEVEL={R_LOG}\nNEW_REMOTE={R_NEW}\nDB_URL=\nREDIS_URL=\n")
    );
    assert!(!p.exists(".env.bak"));
}

#[test]
fn pull_with_nothing_to_change_says_so_and_still_records_the_state() {
    let v = Vercel::start();
    let p = Project::new().file(
        ".env.development",
        &format!("API_URL={R_API}\nDB_URL=x\nLOG_LEVEL={R_LOG}\nNEW_REMOTE={R_NEW}\nREDIS_URL=y\n"),
    );
    let out = run(&p, &v.url(), &pull_args(&["--no-gitignore"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("already up to date");
    assert!(p.exists(".evnx/platform/vercel.json"));
    assert!(!p.exists(".env.development.bak"));
}

#[test]
fn pull_with_nothing_on_vercel_and_no_file_says_it_created_nothing() {
    let mut s = Server::new();
    s.mock("GET", "/v10/projects/prj_1/env")
        .match_query(list_query("team_1"))
        .with_body(json!({ "envs": [] }).to_string())
        .create();
    let p = Project::new();
    let out = run(&p, &s.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("nothing to pull; .env not created");
    out.assert_not_contains("already up to date");
    assert!(!p.exists(".env.development"));
    assert!(!p.exists(".env"));

    // The file exists and Vercel has nothing: that file is up to date.
    let p = Project::new().file(".env.development", "LOCAL=1\n");
    let out = run(&p, &s.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    out.assert_contains(".env.development is already up to date");
    out.assert_not_contains("not created");
    assert_eq!(p.read(".env.development"), "LOCAL=1\n");
}

#[test]
fn pull_refuses_a_file_it_cannot_read_without_touching_it() {
    let v = Vercel::start();
    let p = Project::new().file(".env.development", "A=1\n=broken\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes"]));
    out.assert_code(2);
    out.assert_contains("cannot read");
    assert_eq!(p.read(".env.development"), "A=1\n=broken\n");
}

#[test]
fn pull_failures_use_the_same_exit_codes_and_write_nothing() {
    for (status, code) in [(401, 10), (403, 11), (500, 15)] {
        let mut s = Server::new();
        s.mock("GET", "/v10/projects/prj_1/env")
            .match_query(list_query("team_1"))
            .with_status(status)
            .with_body("{}")
            .create();
        let p = pull_project();
        let out = run(&p, &s.url(), &pull_args(&["--yes"]));
        out.assert_code(code);
        quiet(&out);
        assert_eq!(p.read(".env.development"), PULL_BEFORE);
        assert!(!p.exists(".evnx"));
    }
}

#[cfg(unix)]
#[test]
fn declining_the_pull_exits_4_and_changes_nothing() {
    use support::pty::{Pty, ENTER};
    let v = Vercel::start();
    let p = pull_project();
    let url = v.url();
    let args = pull_args(&["--no-gitignore"]);
    let mut s = Pty::spawn_with(
        p.path(),
        &args,
        &[
            ("VERCEL_TOKEN", TOKEN),
            ("EVNX_VERCEL_API_URL", &url),
            ("VERCEL_PROJECT_ID", ""),
            ("VERCEL_ORG_ID", ""),
        ],
    );
    s.wait_for("[y/N]");
    s.wait_for_echo_off();
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert!(rest.contains("Cancelled"), "{rest}");
    for value in ALL_VALUES {
        assert!(!rest.contains(value), "{value} in:\n{rest}");
    }
    assert_eq!(p.read(".env.development"), PULL_BEFORE);
    assert!(!p.exists(".evnx"));
}

// ── Backups: one `.bak` per file, one `.gitignore` pattern ──────────────────

#[test]
fn a_second_pull_replaces_the_bak_and_removes_legacy_numbered_copies() {
    let v = Vercel::start();
    let p = pull_project()
        .git()
        .file(".gitignore", ".evnx/platform/\n")
        .file(".env.development.bak", "older\n")
        .file(".env.development.bak.2", "legacy2\n")
        .file(".env.development.bak.3", "legacy3\n")
        .file(".env.development.bak.old", "mine\n")
        .file("notes.bak.txt", "mine\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--gitignore"]));
    out.assert_code(0);
    quiet(&out);
    assert_eq!(p.read(".env.development.bak"), PULL_BEFORE);
    assert!(!p.exists(".env.development.bak.2"));
    assert!(!p.exists(".env.development.bak.3"));
    assert_eq!(p.read(".env.development.bak.old"), "mine\n");
    assert_eq!(p.read("notes.bak.txt"), "mine\n");
    assert!(
        out.stderr.contains("removed 2 old backups"),
        "{}",
        out.stderr
    );

    // The file changes again; the second pull replaces the one backup.
    let edited = PULL_BEFORE.replace("old-local-api-value-aa11", "edited-by-hand");
    std::fs::write(p.join(".env.development"), &edited).unwrap();
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--gitignore"]));
    out.assert_code(0);
    quiet(&out);
    assert_eq!(p.read(".env.development.bak"), edited);
    assert!(!p.exists(".env.development.bak.2"));
    out.assert_not_contains("old backup");

    // One pattern covers every copy, added once; nothing staged or committed.
    let ignore = p.read(".gitignore");
    assert_eq!(ignore.matches("*.bak").count(), 1, "{ignore}");
    assert!(p.is_ignored(".env.development.bak"));
    assert_eq!(p.git_cmd(&["diff", "--cached", "--name-only"]).trim(), "");
}

#[test]
fn an_existing_pattern_keeps_the_backup_silent_and_no_gitignore_adds_nothing() {
    let v = Vercel::start();
    let p = pull_project().git().file(".gitignore", ".env*\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--gitignore"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_not_contains("*.bak");
    out.assert_not_contains("not in .gitignore");
    assert!(!p.read(".gitignore").contains("*.bak"));

    let p = pull_project().git().file(".gitignore", "x\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    assert_eq!(p.read(".gitignore"), "x\n");
}

// ── F4: the state file and .gitignore ────────────────────────────────────────

#[test]
fn f4_without_a_terminal_the_state_file_gets_the_hint_and_the_flag_adds_the_entry() {
    let v = Vercel::start();
    // No terminal: the hint, and .gitignore is not touched.
    let p = pull_project().git().file(".gitignore", ".env*\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes"]));
    out.assert_code(0);
    quiet(&out);
    out.assert_contains("hint: .evnx/platform/vercel.json is per-machine state");
    assert_eq!(p.read(".gitignore"), ".env*\n");
    assert!(!p.is_ignored(".evnx/platform/vercel.json"));

    // --gitignore says yes in advance.
    let p = pull_project().git().file(".gitignore", ".env*\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--gitignore"]));
    out.assert_code(0);
    out.assert_not_contains("per-machine state");
    assert!(p.read(".gitignore").contains(".evnx/platform/"));
    assert!(p.is_ignored(".evnx/platform/vercel.json"));

    // --no-gitignore says nothing at all about it.
    let p = pull_project().git().file(".gitignore", ".env*\n");
    let out = run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    out.assert_not_contains("per-machine state");
    assert_eq!(p.read(".gitignore"), ".env*\n");
}

#[cfg(unix)]
#[test]
fn f4_on_a_terminal_without_yes_the_state_file_gets_the_gitignore_question() {
    use support::pty::{Pty, ENTER};
    for (answer, added) in [("y", true), ("n", false)] {
        let v = Vercel::start();
        let p = pull_project().git().file(".gitignore", ".env*\n");
        let url = v.url();
        let args = pull_args(&[]);
        let mut s = Pty::spawn_with(
            p.path(),
            &args,
            &[
                ("VERCEL_TOKEN", TOKEN),
                ("EVNX_VERCEL_API_URL", &url),
                ("VERCEL_PROJECT_ID", ""),
                ("VERCEL_ORG_ID", ""),
            ],
        );
        s.wait_for("[y/N]");
        s.wait_for_echo_off();
        s.press(&["y", ENTER]);
        s.wait_for("per-machine state");
        s.wait_for_echo_off();
        s.press(&[answer, ENTER]);
        let (rest, code) = s.finish();
        assert_eq!(code, 0, "{rest}");
        for value in ALL_VALUES {
            assert!(!rest.contains(value), "{value} in:\n{rest}");
        }
        assert_eq!(
            p.read(".gitignore").contains(".evnx/platform/"),
            added,
            "answer {answer}: {rest}"
        );
        assert_eq!(rest.contains("hint: .evnx"), !added, "{rest}");
    }
}

// ── push ─────────────────────────────────────────────────────────────────────
//
// Values here are built at run time from parts: nothing in this file is shaped
// like a real credential.

/// A value the classifier takes for a payment key.
fn fake_payment_key() -> String {
    format!("sk_{}_{}", "live", "x".repeat(24))
}
/// A second one, for a public-prefixed variable.
fn fake_public_key() -> String {
    format!("pk_{}_{}", "live", "y".repeat(24))
}
const P_LOG: &str = "push-new-log-level-3a3a";
const P_DB: &str = "push-db-value-9b9b";
const P_API: &str = "push-api-value-5c5c";
const P_NEW: &str = "push-brand-new-7d7d";
const P_AUTH: &str = "push-auth-value-8e8e";
/// What a server's error message might echo.
const P_ECHO: &str = "server-echoed-value-1f1f";

/// Every value a push test uses, on top of `ALL_VALUES`.
fn push_values() -> Vec<String> {
    let mut v: Vec<String> = [P_LOG, P_DB, P_API, P_NEW, P_AUTH, P_ECHO]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    v.push(fake_payment_key());
    v.push(fake_public_key());
    v
}

/// `quiet`, and none of the push values either.
#[track_caller]
fn quiet_push(out: &Output) {
    quiet(out);
    for v in push_values() {
        out.assert_not_contains(&v);
    }
}

fn push_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["platform", "push"];
    a.extend_from_slice(COMMON);
    a.extend_from_slice(extra);
    a
}

fn push_project(env_file: &str) -> Project {
    Project::new().file(".env.development", env_file)
}

/// `updatedAt` of `env_entry(id, …)`.
fn updated_at(id: &str) -> u64 {
    1_700_000_000_000u64 + id.len() as u64
}

fn state_file(entries: &[(&str, u64)]) -> String {
    state_file_for("development", entries)
}

/// The current (version 2) shape, with one record.
fn state_file_for(environment: &str, entries: &[(&str, u64)]) -> String {
    json!({
        "version": 2,
        "records": [{
            "project": "prj_1", "team": "team_1", "environment": environment,
            "entries": entries.iter().map(|(id, at)| json!({
                "key": "K", "id": id, "target": [environment], "type": "encrypted", "updatedAt": at,
            })).collect::<Vec<_>>(),
        }],
    })
    .to_string()
}

/// The shape before 0.14: the file is one record.
fn old_state_file(environment: &str, entries: &[(&str, u64)]) -> String {
    let v: Value = serde_json::from_str(&state_file_for(environment, entries)).unwrap();
    v["records"][0].to_string()
}

/// The record for `environment` in the state file.
fn record(p: &Project, environment: &str) -> Value {
    let s: Value = serde_json::from_str(&p.read(".evnx/platform/vercel.json")).unwrap();
    s["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["environment"] == environment)
        .unwrap_or_else(|| panic!("no record for {environment}: {s}"))
        .clone()
}

impl Vercel {
    /// `POST …/env?upsert=false` with exactly this body (once, by default).
    fn post(&mut self, body: Value, status: usize, answer: Value, hits: usize) -> Mock {
        self.server
            .mock("POST", "/v10/projects/prj_1/env")
            .match_query(Matcher::Exact("teamId=team_1&upsert=false".into()))
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .match_body(Matcher::Json(body))
            .with_status(status)
            .with_body(answer.to_string())
            .expect(hits)
            .create()
    }

    /// `PATCH …/env/{id}` with exactly this body.
    fn patch(&mut self, id: &str, body: Value, hits: usize) -> Mock {
        self.server
            .mock("PATCH", format!("/v9/projects/prj_1/env/{id}").as_str())
            .match_query(list_query("team_1"))
            .match_body(Matcher::Json(body))
            .with_status(200)
            // The answer echoes the entry, ciphertext and all.
            .with_body(json!({ "id": id, "value": CIPHER }).to_string())
            .expect(hits)
            .create()
    }

    /// No request that writes anything, to any path.
    fn no_writes(&mut self) -> Vec<Mock> {
        ["POST", "PATCH", "PUT", "DELETE"]
            .iter()
            .map(|m| self.server.mock(m, Matcher::Any).expect(0).create())
            .collect()
    }
}

fn created_answer(keys: &[&str]) -> Value {
    json!({
        "created": keys.iter().map(|k| json!({ "key": k, "value": CIPHER })).collect::<Vec<_>>(),
        "failed": [],
    })
}

fn new_var(key: &str, value: &str, ty: &str) -> Value {
    json!({ "key": key, "value": value, "type": ty, "target": ["development"] })
}

#[test]
fn push_creates_new_keys_in_one_bulk_post_without_upsert() {
    let mut v = Vercel::start_for_diff();
    let pk = fake_payment_key();
    let public = fake_public_key();
    let post = v.post(
        json!([
            new_var("BRAND_NEW", P_NEW, "encrypted"),
            new_var("STRIPE_SECRET_KEY", &pk, "sensitive"),
            // D3: a public prefix is Config even with a credential-shaped value.
            new_var("NEXT_PUBLIC_STRIPE_KEY", &public, "encrypted"),
        ]),
        201,
        created_answer(&["BRAND_NEW", "STRIPE_SECRET_KEY", "NEXT_PUBLIC_STRIPE_KEY"]),
        1,
    );
    let patches = v.no_writes();
    let p = push_project(&format!(
        "BRAND_NEW={P_NEW}\nSTRIPE_SECRET_KEY={pk}\nNEXT_PUBLIC_STRIPE_KEY={public}\n"
    ));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    post.assert(); // exactly one POST, with upsert=false in its query
    for m in &patches {
        m.assert();
    }
    out.assert_contains("created 3 variables");
    out.assert_contains("recorded .evnx/platform/vercel.json");

    // The state file is Vercel's listing after the push: ids and times, no values.
    let text = p.read(".evnx/platform/vercel.json");
    for value in ALL_VALUES
        .iter()
        .map(|s| s.to_string())
        .chain(push_values())
    {
        assert!(!text.contains(&value), "{value} in the state file:\n{text}");
    }
    let s = record(&p, "development");
    assert_eq!(s["entries"].as_array().unwrap().len(), 5);
    assert!(s["entries"][0]["updatedAt"].is_u64());
}

/// Remote with a Config that is not shared (e6) beside the standard ones.
fn push_remote() -> Vercel {
    let mut envs = standard_envs();
    envs.push(env_entry(
        "e6",
        "AUTH_SECRET",
        "encrypted",
        &["development"],
    ));
    let mut v = Vercel::with_list(json!({ "envs": envs, "pagination": { "count": 7 } }), 0);
    let m = v.value_mock("e6", 200, Some(P_AUTH), 1);
    v.once.push(m);
    v
}

fn golden_file() -> String {
    format!(
        "API_URL={L_API_OLD}\nDB_URL={P_DB}\nLOG_LEVEL={R_LOG}\nAUTH_SECRET={P_AUTH}\n\
         BRAND_NEW={P_NEW}\nSTRIPE_SECRET_KEY={}\nNEXT_PUBLIC_STRIPE_KEY={}\nREDIS_URL=\nSPARE=\n",
        fake_payment_key(),
        fake_public_key()
    )
}

#[test]
fn push_dry_run_prints_the_plan_and_sends_nothing() {
    let mut v = push_remote();
    let writes = v.no_writes();
    let p = push_project(&golden_file());
    let out = run(&p, &v.url(), &push_args(&["--dry-run"]));
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("Dry run: nothing was sent");
    for m in &writes {
        m.assert(); // no POST, PATCH or DELETE of any kind
    }
    assert!(!p.exists(".evnx"), "dry run writes no state file");
    // No state file yet: the one warning, and nothing else on stderr.
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
    out.assert_contains("warning: no .evnx/platform/vercel.json yet");
    support::snapshot::check_golden(
        "platform/push-development.txt",
        &support::snapshot::normalise(&out.stdout, &[p.path()]),
    );
}

#[test]
fn push_a_config_that_differs_is_patched_by_id() {
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let writes = v.no_writes();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    patch.assert();
    for m in &writes {
        m.assert(); // no create, no upsert
    }
    out.assert_contains("updated 1 variable");
}

#[test]
fn push_a_secret_on_both_sides_is_an_update_unless_only_new() {
    // As is: DB_URL (a Secret on both sides) is overwritten.
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e2", json!({ "value": P_DB }), 1);
    let p = push_project(&format!("DB_URL={P_DB}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    patch.assert();
    out.assert_contains("~ DB_URL");

    // --only-new: left alone, and nothing is sent.
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = push_project(&format!("DB_URL={P_DB}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--only-new"]));
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("= DB_URL");
    out.assert_contains("--only-new leaves existing secrets alone");
    out.assert_contains("already has everything");
    for m in &writes {
        m.assert();
    }
}

#[test]
fn push_a_shared_entry_loses_this_environment_and_gets_its_own() {
    let mut v = Vercel::start_for_diff();
    // e1 is [development, preview]: development leaves it, preview keeps it...
    let drop_target = v.patch("e1", json!({ "target": ["preview"] }), 1);
    // ...and development gets an entry of the same type (Config).
    let create = v.post(
        json!([new_var("API_URL", P_API, "encrypted")]),
        201,
        created_answer(&["API_URL"]),
        1,
    );
    // The production entry (e1b) is not touched.
    let other = v.patch("e1b", json!({ "target": [] }), 0);
    // Reading e1's value is how a difference is found.
    let p = push_project(&format!("API_URL={P_API}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    drop_target.assert();
    create.assert();
    other.assert();
    out.assert_contains("split: Preview keeps its own entry");
    out.assert_contains("1 split (shared entry left)");
    out.assert_contains("updated 1 variable");
}

#[test]
fn push_a_partial_failure_exits_1_names_the_key_and_keeps_the_rest() {
    let mut v = Vercel::start_for_diff();
    let pk = fake_payment_key();
    let create = v.post(
        json!([
            new_var("BRAND_NEW", P_NEW, "encrypted"),
            new_var("STRIPE_SECRET_KEY", &pk, "sensitive"),
        ]),
        201,
        json!({
            "created": [{ "key": "BRAND_NEW", "value": CIPHER }],
            "failed": [{ "error": {
                "code": "ENV_ALREADY_EXISTS",
                // A message may echo anything; it must never be printed.
                "message": format!("cannot store {P_ECHO}"),
                "key": "STRIPE_SECRET_KEY",
            } }],
        }),
        1,
    );
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!(
        "BRAND_NEW={P_NEW}\nSTRIPE_SECRET_KEY={pk}\nLOG_LEVEL={P_LOG}\n"
    ));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(1);
    quiet_push(&out);
    create.assert();
    patch.assert(); // the in-place change still went through
    assert!(
        out.stderr.contains("STRIPE_SECRET_KEY") && out.stderr.contains("ENV_ALREADY_EXISTS"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("BRAND_NEW"), "{}", out.stderr);
    out.assert_contains("created 1 variable");
    out.assert_contains("updated 1 variable");
    // What was applied is recorded.
    assert!(p.exists(".evnx/platform/vercel.json"));
}

#[test]
fn push_a_429_waits_then_succeeds_and_gives_up_after_three_tries() {
    let mut v = Vercel::start_for_diff();
    let pk = fake_payment_key();
    let limited = v
        .server
        .mock("POST", "/v10/projects/prj_1/env")
        .match_query(Matcher::Exact("teamId=team_1&upsert=false".into()))
        .with_status(429)
        .with_header("x-ratelimit-remaining", "0")
        .with_header("x-ratelimit-reset", &unix_now())
        .with_body(format!(
            r#"{{"error":{{"code":"rate_limited","message":"{P_ECHO}"}}}}"#
        ))
        .expect(1)
        .create();
    let ok = v.post(
        json!([new_var("STRIPE_SECRET_KEY", &pk, "sensitive")]),
        201,
        created_answer(&["STRIPE_SECRET_KEY"]),
        1,
    );
    let p = push_project(&format!("STRIPE_SECRET_KEY={pk}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    limited.assert();
    ok.assert();

    // Never clears: three tries, exit 15, nothing recorded.
    let mut v = Vercel::start_for_diff();
    let always = v
        .server
        .mock("POST", "/v10/projects/prj_1/env")
        .match_query(Matcher::Exact("teamId=team_1&upsert=false".into()))
        .with_status(429)
        .with_header("x-ratelimit-remaining", "0")
        .with_header("x-ratelimit-reset", &unix_now())
        .with_body("{}")
        .expect(3)
        .create();
    let p = push_project(&format!("STRIPE_SECRET_KEY={pk}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(15);
    out.assert_contains("3 tries");
    quiet_push(&out);
    always.assert();
    assert!(!p.exists(".evnx/platform/vercel.json"));
}

#[test]
fn push_stops_with_exit_14_before_any_write_when_vercel_changed() {
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    // The state says e3 was updated long ago; Vercel says it was updated since.
    // e2 is as recorded. The new key is not an entry yet, so is not checked.
    let p = push_project(&format!(
        "LOG_LEVEL={P_LOG}\nDB_URL={P_DB}\nBRAND_NEW={P_NEW}\n"
    ))
    .file(
        ".evnx/platform/vercel.json",
        &state_file(&[("e3", 5), ("e2", updated_at("e2"))]),
    );
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(14);
    quiet_push(&out);
    out.assert_contains("LOG_LEVEL");
    out.assert_not_contains("DB_URL");
    out.assert_contains("changed on Vercel since the last pull or push");
    out.assert_contains("Nothing was written");
    for m in &writes {
        m.assert(); // zero write requests of any kind
    }
}

/// PLAT-STATE: pulling production after development used to replace
/// development's record, so a development push saw "no state" instead of the
/// change made on Vercel.
#[test]
fn plat_state_pull_dev_then_prod_then_a_change_on_vercel_still_stops_a_dev_push() {
    let p = Project::new()
        .file(".env.development", PULL_BEFORE)
        .file(".env.production", "API_URL=old-prod-local\n");
    let flags = ["--yes", "--no-gitignore"];

    let v = Vercel::start();
    run(&p, &v.url(), &pull_args(&flags)).assert_code(0);

    let mut prod = Vercel::start_for_diff();
    let e1b = prod.value_mock("e1b", 200, Some(R_PROD_API), 1);
    let mut args = pull_args(&flags);
    args.extend(["--env-name", "production"]);
    let out = run(&p, &prod.url(), &args);
    out.assert_code(0);
    quiet(&out);
    e1b.assert();

    // Both records are in the one file.
    assert_eq!(
        record(&p, "development")["entries"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        record(&p, "production")["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let file: Value = serde_json::from_str(&p.read(".evnx/platform/vercel.json")).unwrap();
    assert_eq!(file["records"].as_array().unwrap().len(), 2);

    // LOG_LEVEL (e3) is changed on Vercel afterwards.
    let mut envs = standard_envs();
    for e in &mut envs {
        if e["id"] == "e3" {
            e["updatedAt"] = json!(1_800_000_000_000u64);
        }
    }
    let mut changed = Vercel::with_list(json!({ "envs": envs }), 0);
    let writes = changed.no_writes();
    std::fs::write(p.join(".env.development"), format!("LOG_LEVEL={P_LOG}\n")).unwrap();
    let out = run(&p, &changed.url(), &push_args(&flags));
    out.assert_code(14);
    quiet_push(&out);
    out.assert_contains("LOG_LEVEL");
    out.assert_contains("changed on Vercel since the last pull or push");
    out.assert_not_contains("warning: no .evnx");
    for m in &writes {
        m.assert();
    }
}

#[test]
fn plat_state_the_old_single_record_file_still_protects_and_is_upgraded_on_the_next_pull() {
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n")).file(
        ".evnx/platform/vercel.json",
        &old_state_file("development", &[("e3", 5)]),
    );
    run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"])).assert_code(14);
    for m in &writes {
        m.assert();
    }

    let v = Vercel::start();
    run(&p, &v.url(), &pull_args(&["--yes", "--no-gitignore"])).assert_code(0);
    let file: Value = serde_json::from_str(&p.read(".evnx/platform/vercel.json")).unwrap();
    assert_eq!(file["version"], 2);
    assert_eq!(file["records"].as_array().unwrap().len(), 1);
}

#[test]
fn push_goes_ahead_when_the_state_matches_or_does_not_know_the_entry_or_is_missing() {
    // Matches: e3 as recorded.
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n")).file(
        ".evnx/platform/vercel.json",
        &state_file(&[("e3", updated_at("e3"))]),
    );
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    out.assert_not_contains("warning");
    patch.assert();

    // An entry the state does not list is fine.
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"))
        .file(".evnx/platform/vercel.json", &state_file(&[("e999", 1)]));
    run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"])).assert_code(0);
    patch.assert();

    // No state file: a warning, and it goes ahead.
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    assert!(
        out.stderr
            .contains("warning: no .evnx/platform/vercel.json yet"),
        "{}",
        out.stderr
    );
    patch.assert();

    // A state file with only another environment's record says nothing about
    // this one (here in the old single-record shape).
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let other = old_state_file("production", &[("e3", 5)]);
    let p =
        push_project(&format!("LOG_LEVEL={P_LOG}\n")).file(".evnx/platform/vercel.json", &other);
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    out.assert_contains("has no record for development yet");
    patch.assert();
}

#[test]
fn push_without_a_terminal_needs_yes() {
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let out = run(&p, &v.url(), &push_args(&["--no-gitignore"]));
    out.assert_code(2);
    out.assert_contains("--yes");
    quiet_push(&out);
    for m in &writes {
        m.assert();
    }
    assert!(!p.exists(".evnx/platform/vercel.json"));
}

#[cfg(unix)]
#[test]
fn declining_the_push_exits_4_and_sends_nothing() {
    use support::pty::{Pty, ENTER};
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let url = v.url();
    let args = push_args(&["--no-gitignore"]);
    let mut s = Pty::spawn_with(
        p.path(),
        &args,
        &[
            ("VERCEL_TOKEN", TOKEN),
            ("EVNX_VERCEL_API_URL", &url),
            ("VERCEL_PROJECT_ID", ""),
            ("VERCEL_ORG_ID", ""),
        ],
    );
    s.wait_for("[y/N]");
    s.wait_for_echo_off();
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert!(rest.contains("Cancelled"), "{rest}");
    for value in ALL_VALUES
        .iter()
        .map(|s| s.to_string())
        .chain(push_values())
    {
        assert!(!rest.contains(&value), "{value} in:\n{rest}");
    }
    for m in &writes {
        m.assert();
    }
    assert!(!p.exists(".evnx/platform/vercel.json"));
}

#[test]
fn push_a_public_prefix_is_config_whatever_type_is_asked() {
    let mut v = Vercel::start_for_diff();
    let public = fake_public_key();
    let post = v.post(
        json!([new_var("NEXT_PUBLIC_STRIPE_KEY", &public, "encrypted")]),
        201,
        created_answer(&["NEXT_PUBLIC_STRIPE_KEY"]),
        1,
    );
    let p = push_project(&format!("NEXT_PUBLIC_STRIPE_KEY={public}\n"));
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--yes", "--no-gitignore", "--type", "secret"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("public prefix: stays config");
    post.assert();
}

#[test]
fn push_type_overrides_the_classifier_for_new_keys_only() {
    let pk = fake_payment_key();
    // --type secret: a plain-looking new key becomes a Secret; the existing
    // Config keeps its type (only its value is sent).
    let mut v = Vercel::start_for_diff();
    let post = v.post(
        json!([new_var("BRAND_NEW", P_NEW, "sensitive")]),
        201,
        created_answer(&["BRAND_NEW"]),
        1,
    );
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("BRAND_NEW={P_NEW}\nLOG_LEVEL={P_LOG}\n"));
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--yes", "--no-gitignore", "--type", "secret"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("+ BRAND_NEW");
    out.assert_contains("secret  create");
    post.assert();
    patch.assert();

    // --type config: a credential-looking new key is created readable.
    let mut v = Vercel::start_for_diff();
    let post = v.post(
        json!([new_var("STRIPE_SECRET_KEY", &pk, "encrypted")]),
        201,
        created_answer(&["STRIPE_SECRET_KEY"]),
        1,
    );
    let p = push_project(&format!("STRIPE_SECRET_KEY={pk}\n"));
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--yes", "--no-gitignore", "--type", "config"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    post.assert();
}

#[test]
fn push_never_sends_an_empty_value_so_pull_then_push_cannot_blank_a_secret() {
    let mut v = Vercel::start_for_diff();
    // REDIS_URL= is what pull writes for a Secret it cannot fetch.
    let blank_redis = v.patch("e5", json!({ "value": "" }), 0);
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("REDIS_URL=\nLOG_LEVEL={P_LOG}\nNEVER_SET=\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("- REDIS_URL");
    out.assert_contains("- NEVER_SET");
    out.assert_not_contains("! REDIS_URL");
    blank_redis.assert();
    patch.assert();
}

#[test]
fn push_json_is_one_document_with_no_values() {
    let mut v = push_remote();
    let writes = v.no_writes();
    let p = push_project(&golden_file());
    let out = run(&p, &v.url(), &push_args(&["--dry-run", "--format", "json"]));
    out.assert_code(0);
    quiet_push(&out);
    let j: Value = serde_json::from_str(&out.stdout).expect("stdout is JSON and nothing else");
    assert_eq!(j["dry_run"], true);
    assert_eq!(j["applied"], false);
    assert_eq!(j["summary"]["create"], 3);
    assert_eq!(j["summary"]["update"], 2);
    assert_eq!(j["summary"]["split"], 1);
    assert_eq!(j["summary"]["unchanged"], 1);
    assert_eq!(j["summary"]["skipped"], 2);
    let api = j["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == "API_URL")
        .unwrap();
    assert_eq!(api["split"], true);
    assert_eq!(api["keeps_shared_entry"], json!(["Preview"]));
    assert_eq!(j["summary"]["type_differs"], 1);
    assert_eq!(j["summary"]["remote_only"], 1);
    assert!(j["result"].is_null());
    for m in &writes {
        m.assert();
    }

    // Applied: still one document, and the result is in it.
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--yes", "--no-gitignore", "--format", "json"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    let j: Value = serde_json::from_str(&out.stdout).expect("stdout is JSON and nothing else");
    assert_eq!(j["applied"], true);
    assert_eq!(j["result"]["updated"], 1);
    assert_eq!(j["result"]["failed"], json!([]));
    patch.assert();
}

#[test]
fn push_a_refusal_names_the_code_and_never_the_body() {
    let mut v = Vercel::start_for_diff();
    let pk = fake_payment_key();
    let refused = v
        .server
        .mock("POST", "/v10/projects/prj_1/env")
        .match_query(Matcher::Exact("teamId=team_1&upsert=false".into()))
        .with_status(400)
        .with_body(format!(
            r#"{{"error":{{"code":"bad_request","message":"value {P_ECHO} {pk} is invalid"}}}}"#
        ))
        .expect(1)
        .create();
    let p = push_project(&format!("STRIPE_SECRET_KEY={pk}\n"));
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(2);
    out.assert_contains("bad_request");
    quiet_push(&out);
    refused.assert();
    assert!(!p.exists(".evnx/platform/vercel.json"));
}

#[test]
fn push_failures_use_the_same_exit_codes_and_the_write_403_can_be_a_bad_token() {
    let pk = fake_payment_key();
    for (status, body, code) in [
        (401, "{}", 10),
        (
            403,
            r#"{"error":{"code":"forbidden","invalidToken":true}}"#,
            10,
        ),
        (403, r#"{"error":{"code":"forbidden"}}"#, 11),
        (500, "boom", 15),
    ] {
        let mut v = Vercel::start_for_diff();
        v.server
            .mock("POST", "/v10/projects/prj_1/env")
            .match_query(Matcher::Exact("teamId=team_1&upsert=false".into()))
            .with_status(status)
            .with_body(body)
            .create();
        let p = push_project(&format!("STRIPE_SECRET_KEY={pk}\n"));
        let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
        out.assert_code(code);
        quiet_push(&out);
    }
}

#[test]
fn push_with_no_local_file_is_an_error() {
    let v = Vercel::start_for_diff();
    let p = Project::new();
    let out = run(&p, &v.url(), &push_args(&["--yes"]));
    out.assert_code(2);
    out.assert_contains("not found");
}

#[test]
fn f4_push_hints_at_gitignore_without_a_terminal() {
    let mut v = Vercel::start_for_diff();
    v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n")).git();
    let out = run(&p, &v.url(), &push_args(&["--yes"]));
    out.assert_code(0);
    quiet_push(&out);
    assert!(
        out.stderr
            .contains("hint: .evnx/platform/vercel.json is per-machine state"),
        "{}",
        out.stderr
    );
    assert!(!p.is_ignored(".evnx/platform/vercel.json"));
}

#[test]
fn quiet_push_with_yes_is_silent() {
    let mut v = Vercel::start_for_diff();
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    #[cfg(unix)]
    let p = p.mode(".env.development", 0o600);
    let mut args = vec!["-q"];
    args.extend(push_args(&["--yes", "--no-gitignore"]));
    let out = run(&p, &v.url(), &args);
    out.assert_code(0);
    quiet_push(&out);
    patch.assert();
    // Only the no-state-file warning, which -q keeps.
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr.lines().count(), 1, "{}", out.stderr);
}

// ── P3: --prune ──────────────────────────────────────────────────────────────

impl Vercel {
    /// `DELETE …/env/{id}`. The answer echoes the deleted entry, ciphertext and
    /// all (design §2a #9): the command must never print it.
    fn delete(&mut self, id: &str, status: usize, hits: usize) -> Mock {
        let body = if status < 300 {
            json!({ "id": id, "value": CIPHER })
        } else {
            json!({ "error": { "code": "forbidden_by_policy", "message": CIPHER } })
        };
        self.server
            .mock("DELETE", format!("/v9/projects/prj_1/env/{id}").as_str())
            .match_query(list_query("team_1"))
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_status(status)
            .with_body(body.to_string())
            .expect(hits)
            .create()
    }
}

/// Local file for the prune tests: LOG_LEVEL is the same on both sides, REDIS_URL
/// is empty (skipped, never pruned), and API_URL, DB_URL and NEW_REMOTE are
/// Vercel-only.
fn prune_project() -> Project {
    push_project(&format!("LOG_LEVEL={R_LOG}\nREDIS_URL=\n"))
}

#[test]
fn prune_deletes_vercel_only_variables_by_id_and_takes_a_shared_one_out_of_this_environment() {
    let mut v = Vercel::start_for_diff();
    // e2 (DB_URL, secret) and e4 (NEW_REMOTE) are only here: deleted by id.
    let del_e2 = v.delete("e2", 200, 1);
    let del_e4 = v.delete("e4", 200, 1);
    // e1 (API_URL) is shared with preview: development leaves it, nothing is deleted.
    let leave_e1 = v.patch("e1", json!({ "target": ["preview"] }), 1);
    // Never: the production-only entry, the unchanged one, and the empty local key.
    let never: Vec<Mock> = ["e1", "e1b", "e3", "e5"]
        .iter()
        .map(|id| v.delete(id, 200, 0))
        .collect();
    let p = prune_project();
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--prune", "--yes", "--no-gitignore"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    del_e2.assert();
    del_e4.assert();
    leave_e1.assert();
    for m in &never {
        m.assert();
    }
    out.assert_contains("x DB_URL");
    out.assert_contains("x NEW_REMOTE");
    out.assert_contains("~ API_URL");
    out.assert_contains("shared with Preview: only development is taken out of its targets");
    out.assert_contains("- REDIS_URL");
    out.assert_contains("skipped");
    out.assert_contains("2 delete · 1 detach (shared entry stays)");
    out.assert_contains("deleted 2 variables");
    out.assert_contains("took development out of 1 shared variable");
    out.assert_contains("recorded .evnx/platform/vercel.json");
}

#[test]
fn quiet_prune_with_yes_is_silent_on_stdout() {
    let mut v = Vercel::start_for_diff();
    let del = v.delete("e2", 200, 1);
    let del4 = v.delete("e4", 200, 1);
    let leave = v.patch("e1", json!({ "target": ["preview"] }), 1);
    let p = prune_project();
    #[cfg(unix)]
    let p = p.mode(".env.development", 0o600);
    let mut args = vec!["-q"];
    args.extend(push_args(&["--prune", "--yes", "--no-gitignore"]));
    let out = run(&p, &v.url(), &args);
    out.assert_code(0);
    quiet_push(&out);
    assert_eq!(out.stdout, "");
    del.assert();
    del4.assert();
    leave.assert();
}

#[test]
fn without_prune_the_vercel_only_variables_are_kept_and_nothing_is_deleted() {
    let mut v = Vercel::start_for_diff();
    let deletes: Vec<Mock> = ["e1", "e2", "e4", "e5"]
        .iter()
        .map(|id| v.delete(id, 200, 0))
        .collect();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    let patch = v.patch("e3", json!({ "value": P_LOG }), 1);
    let out = run(&p, &v.url(), &push_args(&["--yes", "--no-gitignore"]));
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("! DB_URL");
    out.assert_contains("kept; --prune to delete");
    patch.assert();
    for m in &deletes {
        m.assert();
    }
}

#[test]
fn prune_in_ci_needs_yes_and_says_so_before_any_request() {
    let mut s = Server::new();
    let any = s.mock("GET", Matcher::Any).expect(0).create();
    let writes: Vec<Mock> = ["POST", "PATCH", "PUT", "DELETE"]
        .iter()
        .map(|m| s.mock(m, Matcher::Any).expect(0).create())
        .collect();
    let p = prune_project();
    let out = run(&p, &s.url(), &push_args(&["--prune", "--no-gitignore"]));
    out.assert_code(2);
    quiet_push(&out);
    out.assert_contains("--prune");
    out.assert_contains("--yes");
    any.assert();
    for m in &writes {
        m.assert();
    }
    assert!(!p.exists(".evnx/platform/vercel.json"));
}

#[test]
fn prune_dry_run_lists_the_deletions_and_sends_nothing_even_without_yes() {
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = prune_project();
    let out = run(&p, &v.url(), &push_args(&["--prune", "--dry-run"]));
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("x DB_URL");
    out.assert_contains("Dry run: nothing was sent.");
    for m in &writes {
        m.assert();
    }

    let out = run(
        &p,
        &v.url(),
        &push_args(&["--prune", "--dry-run", "--format", "json"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    let j: Value = serde_json::from_str(&out.stdout).expect("stdout is JSON and nothing else");
    assert_eq!(j["summary"]["delete"], 2);
    assert_eq!(j["summary"]["detach"], 1);
    let action = |k: &str| {
        j["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == k)
            .unwrap()["action"]
            .clone()
    };
    assert_eq!(action("DB_URL"), "delete");
    assert_eq!(action("API_URL"), "detach");
    assert_eq!(action("REDIS_URL"), "skipped");
}

#[test]
fn a_refused_delete_is_exit_1_naming_the_key_and_code_and_never_the_answer() {
    let mut v = Vercel::start_for_diff();
    // 400, not 403: a write 403 may mean a bad token and ends the run.
    let refused = v.delete("e2", 400, 1);
    let ok = v.delete("e4", 200, 1);
    let leave = v.patch("e1", json!({ "target": ["preview"] }), 1);
    let p = prune_project();
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--prune", "--yes", "--no-gitignore"]),
    );
    out.assert_code(1);
    quiet_push(&out);
    refused.assert();
    ok.assert();
    leave.assert();
    out.assert_contains("deleted 1 variable");
    assert!(out.stderr.contains("DB_URL"), "{}", out.stderr);
    assert!(out.stderr.contains("forbidden_by_policy"), "{}", out.stderr);
}

// ── P3: custom environments ──────────────────────────────────────────────────

/// An entry with explicit targets, custom environments and branch.
fn entry_with(
    id: &str,
    key: &str,
    target: &[&str],
    custom: &[&str],
    branch: Option<&str>,
) -> Value {
    let mut e = env_entry(id, key, "encrypted", target);
    if target.is_empty() {
        e.as_object_mut().unwrap().remove("target");
    }
    if !custom.is_empty() {
        e["customEnvironmentIds"] = json!(custom);
    }
    if let Some(b) = branch {
        e["gitBranch"] = json!(b);
    }
    e
}

/// A project with staging and qa custom environments.
fn custom_remote() -> Vercel {
    let envs = vec![
        // staging only
        entry_with("c1", "STG_ONLY", &[], &["env_stg"], None),
        // shared: development, staging and qa
        entry_with(
            "c2",
            "SHARED_CFG",
            &["development"],
            &["env_stg", "env_qa"],
            None,
        ),
        // qa only
        entry_with("c3", "QA_ONLY", &[], &["env_qa"], None),
        // a plain development entry
        entry_with("c4", "DEV_ONLY", &["development"], &[], None),
    ];
    let mut v = Vercel::with_list(json!({ "envs": envs }), 0);
    v.server
        .mock("GET", "/v9/projects/prj_1/custom-environments")
        .match_query(list_query("team_1"))
        .match_header("authorization", format!("Bearer {TOKEN}").as_str())
        .with_status(200)
        .with_body(
            json!({ "environments": [
                { "id": "env_stg", "slug": "staging", "type": "custom" },
                { "id": "env_qa", "slug": "qa", "type": "custom" },
            ]})
            .to_string(),
        )
        .create();
    for (id, value) in [("c1", R_API), ("c2", R_LOG)] {
        let m = v.value_mock(id, 200, Some(value), 1);
        v.once.push(m);
    }
    for id in ["c3", "c4"] {
        let m = v.value_mock(id, 200, None, 0);
        v.never.push(m);
    }
    v
}

#[test]
fn a_custom_environment_is_found_by_slug_and_its_entries_by_id() {
    let v = custom_remote();
    let p = Project::new().file(
        ".env.staging",
        &format!("STG_ONLY={R_API}\nSHARED_CFG={R_LOG}\n"),
    );
    let mut args = diff_args(&["--env-name", "staging"]);
    args.push("--format");
    args.push("json");
    let out = run(&p, &v.url(), &args);
    out.assert_code(0);
    quiet(&out);
    let j: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(j["environment"], "staging");
    let keys: Vec<&str> = j["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["key"].as_str().unwrap())
        .collect();
    // Only staging's two entries: not qa's, not development's.
    assert_eq!(keys, ["STG_ONLY", "SHARED_CFG"]);
    // Their two Config values were read (the first three mocks are the standard ones).
    for m in v.once.iter().skip(3).chain(&v.never) {
        m.assert();
    }
}

#[test]
fn an_unknown_custom_environment_is_exit_2_naming_the_ones_there_are() {
    let mut v = custom_remote();
    let writes = v.no_writes();
    let p = Project::new().file(".env", "A=1\n");
    let out = run(&p, &v.url(), &diff_args(&["--env-name", "nope"]));
    out.assert_code(2);
    quiet(&out);
    out.assert_contains("no custom environment 'nope'");
    out.assert_contains("staging, qa");
    for m in &writes {
        m.assert();
    }
    // A name that cannot be a slug never reaches the network.
    let out = run(&p, &v.url(), &diff_args(&["--env-name", "../x"]));
    out.assert_code(2);
    let out = run(&p, &v.url(), &diff_args(&["--env-name", "Production"]));
    out.assert_code(2);
    out.assert_contains("lowercase");
}

#[test]
fn push_to_a_custom_environment_creates_with_ids_and_no_target_and_splits_by_ids() {
    let mut v = custom_remote();
    // STG_ONLY (c1) is staging's alone: changed in place.
    let patch_c1 = v.patch("c1", json!({ "value": P_LOG }), 1);
    // SHARED_CFG (c2) is also development's and qa's: staging leaves it by
    // customEnvironmentIds, and keeps nothing of `target`...
    let leave_c2 = v.patch("c2", json!({ "customEnvironmentIds": ["env_qa"] }), 1);
    // ...and gets its own entry: customEnvironmentIds, no `target` key at all.
    let create = v.post(
        json!([
            { "key": "SHARED_CFG", "value": P_DB, "type": "encrypted",
              "customEnvironmentIds": ["env_stg"] },
            { "key": "BRAND_NEW", "value": P_NEW, "type": "encrypted",
              "customEnvironmentIds": ["env_stg"] },
        ]),
        201,
        created_answer(&["SHARED_CFG", "BRAND_NEW"]),
        1,
    );
    // Other environments' entries are never touched.
    let untouched = [
        v.patch("c3", json!({ "value": P_LOG }), 0),
        v.patch("c4", json!({ "value": P_LOG }), 0),
    ];
    let p = Project::new().file(
        ".env.staging",
        &format!("STG_ONLY={P_LOG}\nSHARED_CFG={P_DB}\nBRAND_NEW={P_NEW}\n"),
    );
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--env-name", "staging", "--yes", "--no-gitignore"]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("staging (custom)");
    out.assert_contains("local  .env.staging");
    out.assert_contains("split: Development, qa keep their own entry");
    patch_c1.assert();
    leave_c2.assert();
    create.assert();
    for m in &untouched {
        m.assert();
    }
    // The record is keyed by the slug.
    assert!(record(&p, "staging")["entries"].is_array());
}

#[test]
fn prune_in_a_custom_environment_deletes_its_own_and_detaches_the_shared() {
    let mut v = custom_remote();
    let del_c1 = v.delete("c1", 200, 1);
    let leave_c2 = v.patch("c2", json!({ "customEnvironmentIds": ["env_qa"] }), 1);
    let never: Vec<Mock> = ["c2", "c3", "c4"]
        .iter()
        .map(|i| v.delete(i, 200, 0))
        .collect();
    // Local has neither key; falls back to `.env` (no .env.staging).
    let p = Project::new().file(".env", "KEEP_ME=\n");
    let out = run(
        &p,
        &v.url(),
        &push_args(&[
            "--env-name",
            "staging",
            "--prune",
            "--yes",
            "--no-gitignore",
        ]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("local  .env");
    del_c1.assert();
    leave_c2.assert();
    for m in &never {
        m.assert();
    }
}

// ── P3: --git-branch ─────────────────────────────────────────────────────────

fn branch_remote() -> Vercel {
    let envs = vec![
        // API_URL for the branch, and for all branches.
        entry_with("b1", "API_URL", &["preview"], &[], Some("feature")),
        entry_with("b2", "API_URL", &["preview"], &[], None),
        // All branches only: same value locally.
        entry_with("b3", "ONLY_ALL", &["preview"], &[], None),
        // Another branch's.
        entry_with("b4", "OTHER", &["preview"], &[], Some("other")),
        // All branches only: the local value differs.
        entry_with("b6", "ALL_DIFF", &["preview"], &[], None),
        // The branch's alone, and not in the local file.
        entry_with("b7", "BRANCH_GONE", &["preview"], &[], Some("feature")),
    ];
    let mut v = Vercel::with_list(json!({ "envs": envs }), 0);
    for (id, value, hits) in [("b1", R_API, 1), ("b3", R_LOG, 1), ("b6", R_NEW, 1)] {
        let m = v.value_mock(id, 200, Some(value), hits);
        v.once.push(m);
    }
    // The all-branches API_URL is shadowed, the other branch's is not ours.
    for id in ["b2", "b4"] {
        let m = v.value_mock(id, 200, None, 0);
        v.never.push(m);
    }
    v
}

#[test]
fn git_branch_uses_the_branch_entry_falls_back_to_all_branches_and_creates_with_the_branch() {
    let mut v = branch_remote();
    // The branch's own API_URL (b1) changes in place.
    let patch_b1 = v.patch("b1", json!({ "value": P_API }), 1);
    // ALL_DIFF only has an all-branches entry: the branch gets its own beside it
    // (nothing is patched, split or deleted); FRESH_VAR is new. Both carry gitBranch.
    let create = v.post(
        json!([
            { "key": "ALL_DIFF", "value": P_DB, "type": "encrypted",
              "target": ["preview"], "gitBranch": "feature" },
            { "key": "FRESH_VAR", "value": P_NEW, "type": "encrypted",
              "target": ["preview"], "gitBranch": "feature" },
        ]),
        201,
        created_answer(&["ALL_DIFF", "FRESH_VAR"]),
        1,
    );
    let untouched: Vec<Mock> = ["b2", "b3", "b4", "b6"]
        .iter()
        .flat_map(|id| {
            [
                v.patch(id, json!({ "value": P_API }), 0),
                v.delete(id, 200, 0),
            ]
        })
        .collect();
    let p = Project::new().file(
        ".env.preview",
        &format!("API_URL={P_API}\nONLY_ALL={R_LOG}\nALL_DIFF={P_DB}\nFRESH_VAR={P_NEW}\n"),
    );
    let out = run(
        &p,
        &v.url(),
        &push_args(&[
            "--env-name",
            "preview",
            "--git-branch",
            "feature",
            "--yes",
            "--no-gitignore",
        ]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("preview (branch feature)");
    // The fallback is labelled in the plan.
    out.assert_contains("same as the all-branches preview entry");
    out.assert_contains("branch feature gets its own entry, the all-branches one stays");
    assert!(
        out.stderr.contains("scoped to other git branches ignored"),
        "{}",
        out.stderr
    );
    patch_b1.assert();
    create.assert();
    for m in &untouched {
        m.assert();
    }
    // The state is kept apart from plain preview's.
    assert!(record(&p, "preview@feature")["entries"].is_array());
}

#[test]
fn git_branch_with_prune_deletes_only_the_branchs_own_entries() {
    let mut v = branch_remote();
    let del = v.delete("b7", 200, 1);
    // The all-branches and other-branch entries are never deleted.
    let never: Vec<Mock> = ["b2", "b3", "b4", "b6", "b1"]
        .iter()
        .map(|id| v.delete(id, 200, 0))
        .collect();
    let p = Project::new().file(
        ".env.preview",
        &format!("API_URL={R_API}\nONLY_ALL={R_LOG}\nALL_DIFF={R_NEW}\n"),
    );
    let out = run(
        &p,
        &v.url(),
        &push_args(&[
            "--env-name",
            "preview",
            "--git-branch",
            "feature",
            "--prune",
            "--yes",
            "--no-gitignore",
        ]),
    );
    out.assert_code(0);
    quiet_push(&out);
    out.assert_contains("x BRANCH_GONE");
    del.assert();
    for m in &never {
        m.assert();
    }
}

#[test]
fn git_branch_is_only_for_preview() {
    let mut v = Vercel::start_for_diff();
    let writes = v.no_writes();
    let p = push_project(&format!("LOG_LEVEL={P_LOG}\n"));
    for env in ["development", "production", "staging"] {
        let out = run(
            &p,
            &v.url(),
            &push_args(&["--env-name", env, "--git-branch", "feature", "--yes"]),
        );
        out.assert_code(2);
        quiet_push(&out);
        out.assert_contains("--git-branch only applies to --env-name preview");
    }
    // Also on the default environment, and for diff and pull.
    let out = run(
        &p,
        &v.url(),
        &push_args(&["--git-branch", "feature", "--yes"]),
    );
    out.assert_code(2);
    let out = run(&p, &v.url(), &diff_args(&["--git-branch", "feature"]));
    out.assert_code(2);
    let out = run(
        &p,
        &v.url(),
        &pull_args(&["--git-branch", "feature", "--yes"]),
    );
    out.assert_code(2);
    for m in &writes {
        m.assert();
    }
}

#[test]
fn without_git_branch_branch_scoped_preview_variables_are_skipped_and_the_warning_says_how() {
    let v = branch_remote();
    let p = Project::new().file(".env.preview", &format!("ONLY_ALL={R_LOG}\n"));
    let mut args = diff_args(&["--env-name", "preview"]);
    args.extend(["--format", "json"]);
    let out = run(&p, &v.url(), &args);
    quiet(&out);
    assert!(
        out.stderr
            .contains("use --git-branch <branch> with --env-name preview"),
        "{}",
        out.stderr
    );
}
