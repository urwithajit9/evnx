//! `evnx migrate --to github-actions` reports failed uploads in its exit code
//! (I-106, U4 pilot).
//!
//! An upload GitHub refused was listed under "✗ 1 failed" and the run exited
//! **0**, so a pipeline step that migrated half its secrets passed. A run that
//! finishes with failures is now exit 1 (the answer is "no"); a run that cannot
//! start — no token, no repository, GitHub unreachable — stays 2.
//!
//! I-109 (S-M3): the upload asked "Proceed with migration?" and had no `--yes`,
//! so it could not run in CI. `--yes` now skips the question; with no terminal
//! and no `--yes` it stops before contacting GitHub, exit 2, naming `--yes`. At a
//! terminal it still asks, answered on Enter; Esc cancels and uploads nothing.
//!
//! I-266 (R-migrate): which secrets already exist was read from one page of
//! GitHub's list — 30 names — and compared case-sensitively, so `--skip-existing`
//! and `--yes` replaced secrets they promise to keep. The mock pages like GitHub
//! and folds nothing, so the tests see what a real repository would show.
#![cfg(all(unix, feature = "migrate"))]

mod support;

use support::pty::{Pty, ENTER};
use support::{run_with, Project};

/// 32 bytes, base64: a key GitHub's sealed-box encryption accepts.
const PUBLIC_KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";

/// A mock GitHub repository `o/r` that accepts both uploads, or refuses one.
fn github(refuse: Option<&str>) -> mockito::ServerGuard {
    let mut server = github_with(&[]);
    for name in ["GOOD_ONE", "BAD_ONE"] {
        let status = if refuse == Some(name) { 500 } else { 201 };
        server
            .mock("PUT", format!("/repos/o/r/actions/secrets/{name}").as_str())
            .with_status(status)
            .create();
    }
    server
}

/// A mock GitHub repository `o/r` that already holds `existing`, with no
/// upload routes: each test adds its own with [`put`], which counts calls.
fn github_with(existing: &[&str]) -> mockito::ServerGuard {
    let mut server = mockito::Server::new();
    let names: Vec<String> = existing.iter().map(|n| n.to_string()).collect();
    server
        .mock("GET", "/repos/o/r/actions/secrets")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body_from_request(move |req| page_of(&names, req.path_and_query()).into_bytes())
        .create();
    server
        .mock("GET", "/repos/o/r/actions/secrets/public-key")
        .with_status(200)
        .with_body(format!(r#"{{"key_id":"kid","key":"{PUBLIC_KEY}"}}"#))
        .create();
    server
}

/// One page of GitHub's secret list, as GitHub pages it: `per_page` defaults
/// to 30 and is capped at 100, `page` starts at 1.
fn page_of(names: &[String], path_and_query: &str) -> String {
    let query = path_and_query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let param = |key: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.parse::<usize>().ok())
    };
    let per_page = param("per_page").unwrap_or(30).min(100);
    let page = param("page").unwrap_or(1).max(1);
    let items: Vec<String> = names
        .iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .map(|n| format!(r#"{{"name":"{n}"}}"#))
        .collect();
    format!(
        r#"{{"total_count":{},"secrets":[{}]}}"#,
        names.len(),
        items.join(",")
    )
}

const ARGS: &[&str] = &[
    "migrate",
    "--from",
    "env-file",
    "--to",
    "github-actions",
    "--repo",
    "o/r",
    "--github-token",
    "t",
];

fn project() -> Project {
    Project::new().env("GOOD_ONE=1\nBAD_ONE=2\n")
}

/// At a terminal, answering the confirmation with `answer`.
fn migrate_answering(server: &mockito::ServerGuard, answer: &[&str]) -> (String, i32) {
    let p = project();
    let url = server.url();
    let mut s = Pty::spawn_with(
        p.path(),
        ARGS,
        &[
            ("GITHUB_API_URL", url.as_str()),
            ("EVNX_GITHUB_PAUSE_MS", "0"),
        ],
    );
    s.wait_for("Proceed with migration?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(answer);
    s.finish()
}

fn migrate(server: &mockito::ServerGuard) -> (String, i32) {
    // ⚠️ "y" then Enter: the question is answered on Enter (N-09), like every
    // other yes/no prompt in evnx.
    migrate_answering(server, &["y", ENTER])
}

/// Without a terminal, with `extra` flags.
fn migrate_piped(server: &mockito::ServerGuard, extra: &[&str]) -> support::Output {
    let p = project();
    let url = server.url();
    let mut args = ARGS.to_vec();
    args.extend_from_slice(extra);
    run_with(
        p.path(),
        &args,
        &[
            ("GITHUB_API_URL", url.as_str()),
            ("EVNX_GITHUB_PAUSE_MS", "0"),
        ],
        None,
    )
}

fn put(server: &mut mockito::ServerGuard, name: &str, hits: usize) -> mockito::Mock {
    server
        .mock("PUT", format!("/repos/o/r/actions/secrets/{name}").as_str())
        .with_status(201)
        .expect(hits)
        .create()
}

#[test]
fn t_i106_a_failed_upload_exits_1_and_is_named() {
    let server = github(Some("BAD_ONE"));
    let (out, code) = migrate(&server);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("1 failed"), "{out}");
    assert!(out.contains("BAD_ONE"), "{out}");
}

#[test]
fn t_i106_every_upload_succeeding_exits_0() {
    let server = github(None);
    let (out, code) = migrate(&server);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn t_i109_no_terminal_and_no_yes_stops_before_github() {
    let mut server = mockito::Server::new();
    let any = server.mock("GET", mockito::Matcher::Any).expect(0).create();
    let out = migrate_piped(&server, &[]);
    out.assert_code(2);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);
    assert!(out.stderr.contains("No verdict"), "{}", out.stderr);
    any.assert();
}

#[test]
fn t_i109_yes_uploads_without_a_terminal() {
    let mut server = github_with(&[]);
    let good = put(&mut server, "GOOD_ONE", 1);
    let bad = put(&mut server, "BAD_ONE", 1);
    let out = migrate_piped(&server, &["--yes"]);
    out.assert_code(0);
    good.assert();
    bad.assert();
}

/// I-274 (RM-08): with no terminal the progress bar is hidden, and its
/// `println` printed nothing, so a CI log said "1 failed" and named no secret.
/// Each secret is named; under -q only the failure is.
#[test]
fn t_i274_a_ci_log_names_each_secret() {
    let server = github(Some("BAD_ONE"));
    let out = migrate_piped(&server, &["--yes"]);
    out.assert_code(1);
    assert!(out.stdout.contains("✓ GOOD_ONE"), "{}", out.stdout);
    assert!(out.stdout.contains("✗ BAD_ONE"), "{}", out.stdout);

    let quiet = migrate_piped(&server, &["--yes", "-q"]);
    quiet.assert_code(1);
    assert!(!quiet.stdout.contains("GOOD_ONE"), "{}", quiet.stdout);
    assert!(quiet.stdout.contains("✗ BAD_ONE"), "{}", quiet.stdout);
}

/// `--yes` answers "Proceed?", not "overwrite a secret that is already
/// there?". That needs its own flag, so nothing is replaced by habit.
#[test]
fn t_i109_yes_does_not_overwrite_existing_secrets() {
    let mut server = github_with(&["GOOD_ONE"]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 0);
    let out = migrate_piped(&server, &["--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("GOOD_ONE"), "{}", out.stderr);
    assert!(out.stderr.contains("--overwrite"), "{}", out.stderr);
    assert!(out.stderr.contains("--skip-existing"), "{}", out.stderr);
    good.assert();
    bad.assert();
}

/// T-MIG-28: `--skip-existing` leaves an existing secret alone and says so.
#[test]
fn t_mig_28_skip_existing_uploads_only_new_secrets() {
    let mut server = github_with(&["GOOD_ONE"]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 1);
    let out = migrate_piped(&server, &["--yes", "--skip-existing"]);
    out.assert_code(0);
    assert!(out.stdout.contains("1 skipped"), "{}", out.stdout);
    good.assert();
    bad.assert();
}

#[test]
fn t_i109_esc_at_the_confirmation_uploads_nothing() {
    let mut server = github_with(&[]);
    let good = put(&mut server, "GOOD_ONE", 0);
    let bad = put(&mut server, "BAD_ONE", 0);
    // ⚠️ Not `press(&[ESC, ENTER])`: queued together they are read as one
    // unknown escape sequence and the Enter is lost, so the question never
    // returns — it hung on a slow CI runner (#156, again here in U8d's CI).
    // `cancel_yes_no` waits for the redraw between the two.
    let p = project();
    let url = server.url();
    let mut s = Pty::spawn_with(
        p.path(),
        ARGS,
        &[
            ("GITHUB_API_URL", url.as_str()),
            ("EVNX_GITHUB_PAUSE_MS", "0"),
        ],
    );
    s.wait_for("Proceed with migration?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.cancel_yes_no();
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing was uploaded"), "{out}");
    good.assert();
    bad.assert();
}

/// Without a terminal, in a project whose `.env` is `env`.
fn migrate_env(server: &mockito::ServerGuard, env: &str, extra: &[&str]) -> support::Output {
    let p = Project::new().env(env);
    let url = server.url();
    let mut args = ARGS.to_vec();
    args.extend_from_slice(extra);
    run_with(
        p.path(),
        &args,
        &[
            ("GITHUB_API_URL", url.as_str()),
            ("EVNX_GITHUB_PAUSE_MS", "0"),
        ],
        None,
    )
}

/// I-266: 31 secrets exist and `DB_URL` is the 31st — on page 2 of GitHub's
/// default page size. `--skip-existing` must still leave it alone.
#[test]
fn t_i266_skip_existing_sees_every_existing_secret() {
    let mut existing: Vec<String> = (1..=30).map(|i| format!("S{i:02}")).collect();
    existing.push("DB_URL".into());
    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
    let mut server = github_with(&refs);
    let db = put(&mut server, "DB_URL", 0);
    let new = put(&mut server, "NEW_ONE", 1);
    let out = migrate_env(
        &server,
        "DB_URL=new\nNEW_ONE=1\n",
        &["--yes", "--skip-existing"],
    );
    out.assert_code(0);
    assert!(out.stdout.contains("1 skipped"), "{}", out.stdout);
    db.assert();
    new.assert();
}

/// I-266: `--yes` without `--overwrite` never replaces an existing secret,
/// wherever it sits in GitHub's list.
#[test]
fn t_i266_yes_does_not_overwrite_a_secret_past_the_first_page() {
    let mut existing: Vec<String> = (1..=30).map(|i| format!("S{i:02}")).collect();
    existing.push("DB_URL".into());
    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
    let mut server = github_with(&refs);
    let db = put(&mut server, "DB_URL", 0);
    let out = migrate_env(&server, "DB_URL=new\n", &["--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("DB_URL"), "{}", out.stderr);
    db.assert();
}

/// I-266: GitHub secret names ignore case (it stores them uppercase), so
/// `good_one` in the `.env` is the existing `GOOD_ONE`.
#[test]
fn t_i266_existing_names_match_whatever_their_case() {
    let mut server = github_with(&["GOOD_ONE"]);
    let lower = put(&mut server, "good_one", 0);
    let out = migrate_env(&server, "good_one=1\n", &["--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("good_one"), "{}", out.stderr);
    assert!(out.stderr.contains("--overwrite"), "{}", out.stderr);
    lower.assert();
}

/// I-267: GitHub refuses names that start with `GITHUB_`. evnx refuses them
/// before contacting GitHub, rather than counting one failed upload per name.
#[test]
fn t_i267_github_reserved_names_are_refused_before_any_request() {
    let mut server = mockito::Server::new();
    let any = server.mock("GET", mockito::Matcher::Any).expect(0).create();
    let put_any = server.mock("PUT", mockito::Matcher::Any).expect(0).create();
    let out = migrate_env(&server, "GITHUB_X=1\nOK=2\n", &["--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("GITHUB_X"), "{}", out.stderr);
    any.assert();
    put_any.assert();
}

/// I-112, DEC-MG3: every request carries evnx's versioned user agent and pins
/// the REST API version, from the one shared HTTP client.
#[test]
fn t_i112_requests_name_evnx_and_the_api_version() {
    let ua = format!("evnx/{}", env!("CARGO_PKG_VERSION"));
    let mut server = mockito::Server::new();
    let list = server
        .mock("GET", "/repos/o/r/actions/secrets")
        .match_query(mockito::Matcher::Any)
        .match_header("user-agent", ua.as_str())
        .match_header("x-github-api-version", "2022-11-28")
        .match_header("accept", "application/vnd.github+json")
        .with_body(r#"{"total_count":0,"secrets":[]}"#)
        .create();
    let key = server
        .mock("GET", "/repos/o/r/actions/secrets/public-key")
        .match_header("user-agent", ua.as_str())
        .match_header("x-github-api-version", "2022-11-28")
        .with_body(format!(r#"{{"key_id":"kid","key":"{PUBLIC_KEY}"}}"#))
        .create();
    let upload = server
        .mock("PUT", "/repos/o/r/actions/secrets/ONE")
        .match_header("user-agent", ua.as_str())
        .match_header("x-github-api-version", "2022-11-28")
        .with_status(201)
        .create();
    let out = migrate_env(&server, "ONE=1\n", &["--yes"]);
    out.assert_code(0);
    list.assert();
    key.assert();
    upload.assert();
}
