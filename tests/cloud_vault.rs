//! `evnx vault` and `cloud delete-version` fixes (U7 Part D2: CG-02, DEC-VL1).
//!
//! - I-220: an API token could delete a vault, delete versions, share, change
//!   roles, revoke and re-key from the CLI; the server now refuses those for
//!   tokens (evnx-server SV-1) and the CLI refuses them first, sending nothing.
//!   Push, pull, list and members stay open to tokens — that is what CI needs.
//! - I-223: `--role Admin` was refused; roles are now case-insensitive, as
//!   environments already were.
#![cfg(feature = "cloud")]

mod support;

use support::{run_with, Project};

const TOKEN: &str = "evnx_tok_00000000000000000000000000000000";

fn with_token(server: &str, args: &[&str]) -> support::Output {
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    run_with(
        p.path(),
        args,
        &[
            ("EVNX_SERVER", server),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_TOKEN", TOKEN),
        ],
        None,
    )
}

#[test]
fn t_i220_a_token_cannot_change_who_holds_a_vault() {
    let mut s = mockito::Server::new();
    let any: Vec<_> = ["GET", "POST", "PUT", "PATCH", "DELETE"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    for args in [
        &["vault", "delete", "v", "-y"][..],
        &["vault", "share", "v", "--with", "bob@example.com"][..],
        &[
            "vault",
            "role",
            "v",
            "--user",
            "bob@example.com",
            "--role",
            "viewer",
        ][..],
        &["vault", "revoke", "v", "--user", "bob@example.com", "-y"][..],
        &["vault", "rekey", "v", "-y"][..],
        &["cloud", "delete-version", "1", "--vault", "v", "-y"][..],
    ] {
        let out = with_token(&s.url(), args);
        out.assert_code(2);
        assert!(
            out.stderr.contains("an API token cannot") && out.stderr.contains("deliberate"),
            "{args:?}:\n{}",
            out.stderr
        );
    }
    for m in any {
        m.assert();
    }
}

#[test]
fn t_i223_roles_are_case_insensitive() {
    for role in ["Admin", "VIEWER", " developer "] {
        for args in [
            &[
                "vault",
                "role",
                "v",
                "--user",
                "bob@example.com",
                "--role",
                role,
            ][..],
            &[
                "vault",
                "share",
                "v",
                "--with",
                "bob@example.com",
                "--role",
                role,
            ][..],
        ] {
            let out = with_token("http://127.0.0.1:9", args);
            assert!(
                !out.stderr.contains("role must be one of"),
                "{args:?} refused by case:\n{}",
                out.stderr
            );
        }
    }
    let out = with_token(
        "http://127.0.0.1:9",
        &[
            "vault",
            "role",
            "v",
            "--user",
            "bob@example.com",
            "--role",
            "owner",
        ],
    );
    assert!(out.stderr.contains("role must be one of"), "{}", out.stderr);
}

/// DEC-VL1: what CI does stays open to a token — it reaches the server.
#[test]
fn t_i220_a_token_still_lists_vaults() {
    let mut s = mockito::Server::new();
    let list = s
        .mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(r#"{"vaults":[]}"#)
        .expect(1)
        .create();
    let out = with_token(&s.url(), &["vault", "list"]);
    out.assert_code(0);
    list.assert();
}
