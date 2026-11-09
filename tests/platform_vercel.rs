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
    assert_eq!(j["summary"]["update"], 2);
    assert_eq!(j["summary"]["unchanged"], 1);
    assert_eq!(j["summary"]["remote_only"], 2);
    support::snapshot::check_golden(
        "platform/diff-development.json",
        &support::snapshot::normalise(&out.stdout, &[p.path()]),
    );
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
    // Nothing to compare with at all is an error, not an empty diff.
    let empty = Project::new();
    let out = run(&empty, &v.url(), &diff_args(&[]));
    out.assert_code(2);
    out.assert_contains("not found");
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
    let out = run(&p, &s.url(), &diff_args(&[]));
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
    let s: Value = serde_json::from_str(&text).unwrap();
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
