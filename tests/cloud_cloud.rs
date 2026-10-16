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
use support::{run_with, Project};

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
