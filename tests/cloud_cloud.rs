//! `evnx cloud` fixes (U7 Part D3: CG-03, CG-05, CG-08).
//!
//! - I-221, DEC-CL2: `cloud link <uuid>` wrote `vault = "/"` — an id skipped the
//!   lookup and left name and environment empty.
//! - I-224, DEC-CL8: `cloud unlink` left a 0-byte `.evnx.toml`.
//! - I-232, DEC-CL8: `cloud status` said "not signed in" with `EVNX_TOKEN` set,
//!   while every other cloud command used the token.
//! - I-230, DEC-CL6: `cloud export` wrote plaintext into `./evnx-export` inside a
//!   git work tree with no check.
#![cfg(feature = "cloud")]

mod support;

use support::cloud::signed_in;
use support::{Project, run_with};

const VAULT_ID: &str = "22222222-2222-2222-2222-222222222222";

#[test]
fn t_i221_link_by_id_writes_the_vault_name() {
    let p = Project::new();
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"owner","version_count":1,"updated_at":"2026-10-16T00:00:00Z"}}]}}"#
        ))
        .create();
    let out = signed_in(&p, &s.url(), &["cloud", "link", VAULT_ID], None);
    out.assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains(r#"vault = "app/production""#), "{toml}");
    assert!(!toml.contains(r#"vault = "/""#), "{toml}");
}

#[test]
fn t_i224_unlink_removes_a_file_it_emptied() {
    let p = Project::new().file(".evnx.toml", "[cloud]\nvault = \"app/production\"\n");
    let out = run_with(p.path(), &["cloud", "unlink"], &[], None);
    out.assert_code(0);
    assert!(
        !p.path().join(".evnx.toml").exists(),
        "an empty .evnx.toml was left"
    );

    // Anything else in the file is the user's: the file stays, without [cloud].
    let p = Project::new().file(
        ".evnx.toml",
        "[sync]\nnaming_policy = \"warn\"\n\n[cloud]\nvault = \"app/production\"\n",
    );
    run_with(p.path(), &["cloud", "unlink"], &[], None).assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(
        toml.contains("naming_policy") && !toml.contains("vault"),
        "{toml}"
    );
}

#[test]
fn t_i232_status_reports_an_api_token() {
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let out = run_with(
        p.path(),
        &["cloud", "status"],
        &[
            ("EVNX_SERVER", "http://127.0.0.1:9"),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_TOKEN", "evnx_tok_00000000"),
        ],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("API token"), "{}", out.stdout);
    assert!(!out.stdout.contains("not signed in"), "{}", out.stdout);
    // F11: the build is named in the header block.
    assert!(
        out.stdout
            .contains(concat!("client    evnx ", env!("CARGO_PKG_VERSION"))),
        "{}",
        out.stdout
    );
}

#[test]
fn t_i230_export_refuses_a_git_work_tree_before_any_request() {
    let p = Project::new();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(p.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status();
    if !git.is_ok_and(|s| s.success()) {
        eprintln!("git not available; skipping");
        return;
    }
    let mut s = mockito::Server::new();
    let any: Vec<_> = ["GET", "POST"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    let out = signed_in(
        &p,
        &s.url(),
        &["cloud", "export", "--vault", VAULT_ID],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("-o") && out.stderr.contains("--force"),
        "{}",
        out.stderr
    );
    assert!(!p.path().join("evnx-export").exists());
    for m in any {
        m.assert();
    }
}

/// RM-85 (F8): `link` run in a subfolder writes the project's `.evnx.toml`,
/// which covers every folder below it. It said "bound this directory".
#[test]
fn rm85_link_names_the_folder_it_bound() {
    let p = Project::new()
        .file(".evnx.toml", "[sync]\nnaming_policy = \"warn\"\n")
        .file("deploy/.keep", "")
        .git();
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"owner","version_count":1,"updated_at":"2026-10-16T00:00:00Z"}}]}}"#
        ))
        .create();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let out = run_with(
        &p.path().join("deploy"),
        &["cloud", "link", "app/production"],
        &[
            ("EVNX_SERVER", s.url().as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        None,
    );
    out.assert_code(0);
    let root = p.path().file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        out.stdout
            .contains("and every folder below it to app/production"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains(&root), "{}", out.stdout);
    assert!(!out.stdout.contains("this directory"), "{}", out.stdout);
    assert!(p.read(".evnx.toml").contains(r#"vault = "app/production""#));
}

/// RM-86: `link --env` stores the file relative to the `.evnx.toml`'s
/// folder, and refuses one outside it.
#[test]
fn rm86_link_file_is_stored_from_the_toml_folder() {
    let p = Project::new()
        .file(".evnx.toml", "[sync]\nnaming_policy = \"warn\"\n")
        .file("deploy/.keep", "")
        .git();
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"owner","version_count":1,"updated_at":"2026-10-16T00:00:00Z"}}]}}"#
        ))
        .create();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let env = [
        ("EVNX_SERVER", s.url()),
        ("EVNX_CONFIG_DIR", cfg.to_str().unwrap().to_string()),
    ];
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let deploy = p.path().join("deploy");

    let out = run_with(
        &deploy,
        &["cloud", "link", "app/production", "--env", ".env"],
        &env,
        None,
    );
    out.assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(toml.contains(r#"file = "deploy/.env""#), "{toml}");
    assert!(toml.contains("naming_policy"), "{toml}");

    let outside = run_with(
        &deploy,
        &[
            "cloud",
            "link",
            "app/production",
            "--env",
            "../../elsewhere.env",
        ],
        &env,
        None,
    );
    outside.assert_code(2);
    assert!(outside.stderr.contains("is outside"), "{}", outside.stderr);
}

// ─── `cloud delete-version --latest` ─────────────────────────────────────────

fn versions_body(nums: &[i32]) -> String {
    let rows: Vec<String> = nums
        .iter()
        .rev()
        .map(|n| {
            format!(
                r#"{{"version_num":{n},"key_count":{n},"key_names":["A"],"blob_hash":"h{n}",
                "blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-11-1{n}T09:00:00Z"}}"#
            )
        })
        .collect();
    format!(r#"{{"versions":[{}]}}"#, rows.join(","))
}

/// A server with `app/production` holding `nums`, and one mock per version
/// deletion that is expected to happen.
fn latest_server(nums: &[i32]) -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"admin","version_count":{},"updated_at":"2026-11-18T00:00:00Z"}}]}}"#,
            nums.len()
        ))
        .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
    )
    .with_status(200)
    .with_body(versions_body(nums))
    .create();
    s
}

fn marks(p: &Project) -> serde_json::Value {
    let text = std::fs::read_to_string(p.path().join("cfg/high-water.json")).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
}

/// `--latest` resolves the number from the list, deletes that number, names the
/// version that takes over, and lowers this machine's high-water mark so the
/// machine that did it is not the one that warns.
#[test]
fn del_latest_with_yes_deletes_the_resolved_number_and_lowers_the_mark() {
    let mut s = latest_server(&[1, 2, 3]);
    let del = s
        .mock(
            "DELETE",
            format!("/api/v1/vaults/{VAULT_ID}/versions/3").as_str(),
        )
        .with_status(200)
        .with_body(r#"{"deleted":3,"latest":2,"was_latest":true}"#)
        .expect(1)
        .create();
    let other = s
        .mock(
            "DELETE",
            mockito::Matcher::Regex(r"/versions/[12]$".to_string()),
        )
        .expect(0)
        .create();
    let p = Project::new();
    std::fs::create_dir_all(p.path().join("cfg")).unwrap();
    std::fs::write(
        p.path().join("cfg/high-water.json"),
        format!(r#"{{"{VAULT_ID}": 3}}"#),
    )
    .unwrap();
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "cloud",
            "delete-version",
            "--latest",
            "--vault",
            "app/production",
            "--yes",
        ],
        None,
    );
    out.assert_code(0);
    del.assert();
    other.assert();
    assert!(
        out.stdout
            .contains("Version 2 (2 key(s), pushed 2026-11-12) becomes the latest"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("deleted version 3"), "{}", out.stdout);
    assert_eq!(marks(&p)[VAULT_ID], 2, "the mark must follow the delete");
}

/// The vault's only version is not deletable this way: the message names the
/// whole-vault delete and nothing is sent.
#[test]
fn del_latest_with_a_single_version_is_refused_and_sends_nothing() {
    let mut s = latest_server(&[1]);
    let any = s.mock("DELETE", mockito::Matcher::Any).expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "cloud",
            "delete-version",
            "--latest",
            "--vault",
            "app/production",
            "--yes",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("only version") && out.stderr.contains("evnx vault delete"),
        "{}",
        out.stderr
    );
    any.assert();
}

/// A bare number for the latest version is still refused, and the message says
/// which flag does it.
#[test]
fn del_latest_by_number_points_to_the_flag() {
    let mut s = latest_server(&[1, 2]);
    let any = s.mock("DELETE", mockito::Matcher::Any).expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "cloud",
            "delete-version",
            "2",
            "--vault",
            "app/production",
            "--yes",
        ],
        None,
    );
    out.assert_code(2);
    assert!(out.stderr.contains("--latest"), "{}", out.stderr);
    any.assert();
}

/// Without a terminal and without `--yes`: exit 4, the command with `--yes` is
/// named, and nothing is sent.
#[test]
fn del_latest_without_a_terminal_exits_4() {
    let mut s = latest_server(&[1, 2]);
    let any = s.mock("DELETE", mockito::Matcher::Any).expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "cloud",
            "delete-version",
            "--latest",
            "--vault",
            "app/production",
        ],
        None,
    );
    out.assert_code(4);
    assert!(
        out.stderr
            .contains("evnx cloud delete-version --latest --vault app/production --yes"),
        "{}",
        out.stderr
    );
    any.assert();
}

/// `VERSION` and `--latest` are one choice, and one of them is required.
#[test]
fn del_latest_and_a_number_conflict_and_one_is_required() {
    let s = mockito::Server::new();
    for args in [
        &["cloud", "delete-version", "2", "--latest", "--vault", "x"][..],
        &["cloud", "delete-version", "--vault", "x"][..],
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(2);
    }
}

// ─── `cloud watch --on-change` ───────────────────────────────────────────────

/// A project whose high-water mark for the vault is `mark`, and a signed-in
/// server listing `nums`. The command in these tests writes the two watch
/// variables to `out`, and (to prove the API token is not handed over) `tok`.
fn watch_setup(mark: i32, nums: &[i32]) -> (Project, mockito::ServerGuard) {
    let s = latest_server(nums);
    let p = Project::new();
    std::fs::create_dir_all(p.path().join("cfg")).unwrap();
    std::fs::write(
        p.path().join("cfg/high-water.json"),
        format!(r#"{{"{VAULT_ID}": {mark}}}"#),
    )
    .unwrap();
    (p, s)
}

fn hook_script(p: &Project, exit: i32) -> String {
    format!(
        "printf '%s|%s|%s\\n' \"$EVNX_WATCH_VAULT\" \"$EVNX_WATCH_VERSION\" \"${{EVNX_TOKEN:-none}}\" \
         >> '{}'; exit {exit}",
        p.path().join("hook.out").display()
    )
}

/// `--once --on-change`: the vault is past what this machine has seen, so the
/// command runs once, with the vault and the new version and no `EVNX_TOKEN`,
/// and its exit status does not change watch's.
#[test]
fn n14_7_once_runs_the_command_for_a_pending_change_and_a_failure_is_not_fatal() {
    let (p, s) = watch_setup(1, &[1, 2]);
    let script = hook_script(&p, 3);
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "cloud",
            "watch",
            "--vault",
            "app/production",
            "--once",
            "--on-change",
            "--",
            "sh",
            "-c",
            &script,
        ],
        None,
    );
    out.assert_code(0);
    assert_eq!(
        p.read("hook.out"),
        "app/production|2|none\n",
        "{}\n{}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.contains("v2"), "{}", out.stdout);
    assert!(
        out.stderr.contains("exited 3") && out.stderr.contains("watching goes on"),
        "{}",
        out.stderr
    );
}

/// Nothing pending (the mark is already at the latest): no command.
#[test]
fn n14_7_once_runs_nothing_when_nothing_is_new() {
    let (p, s) = watch_setup(2, &[1, 2]);
    let script = hook_script(&p, 0);
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "cloud",
            "watch",
            "--vault",
            "app/production",
            "--once",
            "--on-change",
            "--",
            "sh",
            "-c",
            &script,
        ],
        None,
    );
    out.assert_code(0);
    assert!(!p.path().join("hook.out").exists(), "{}", out.stdout);
}

/// `--on-change` and the command belong together, and nothing runs without both.
#[test]
fn n14_7_the_flag_and_the_command_require_each_other() {
    let s = mockito::Server::new();
    for args in [
        &["cloud", "watch", "--vault", "x", "--on-change"][..],
        &["cloud", "watch", "--vault", "x", "--", "true"][..],
    ] {
        signed_in(&Project::new(), &s.url(), args, None).assert_code(2);
    }
}

/// Without `--once`: a change that appears after the first poll runs the
/// command once; a failing command leaves watch running (it is killed here).
#[test]
fn n14_7_a_new_version_runs_the_command_and_watching_goes_on() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"admin","version_count":1,"updated_at":"2026-11-18T00:00:00Z"}}]}}"#
        ))
        .create();
    let path = format!("/api/v1/vaults/{VAULT_ID}/versions");
    // The first list (at start) has one version; every later one has two.
    s.mock("GET", path.as_str())
        .with_status(200)
        .with_body(versions_body(&[1]))
        .expect(1)
        .create();
    s.mock("GET", path.as_str())
        .with_status(200)
        .with_body(versions_body(&[1, 2]))
        .create();

    let p = Project::new();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let script = hook_script(&p, 3);
    let mut child = Command::new(support::evnx_bin())
        .current_dir(p.path())
        .args([
            "cloud",
            "watch",
            "--vault",
            "app/production",
            "--interval",
            "5",
            "--on-change",
            "--",
            "sh",
            "-c",
            &script,
        ])
        .env("NO_COLOR", "1")
        .env("EVNX_SERVER", s.url())
        .env("EVNX_CONFIG_DIR", &cfg)
        .env("EVNX_TOKEN", "must-not-reach-the-command")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let marker = p.path().join("hook.out");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    // Let a second poll go by: a failed command must not have ended the watch,
    // and the same version must not run it twice.
    std::thread::sleep(Duration::from_secs(6));
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(still_running, "a failing command ended the watch");
    assert_eq!(p.read("hook.out"), "app/production|2|none\n");
}
