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

use support::cloud::signed_in;
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
    // `owner` is still refused, now with the reason (DEC-VL10).
    assert!(
        out.stderr.contains("`owner` cannot be granted"),
        "{}",
        out.stderr
    );
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

/// DEC-VL10 (A-303): one role parser for `vault` and `org`. `owner` is refused
/// with the reason by every command that grants a role — `vault share` used to
/// give only "role must be one of …" — and before any request.
#[test]
fn t_dec_vl10_a_vault_owner_role_is_refused_with_the_reason() {
    let mut s = mockito::Server::new();
    let any: Vec<_> = ["GET", "POST", "PATCH"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    for args in [
        &[
            "vault",
            "share",
            "v",
            "--with",
            "bob@example.com",
            "--role",
            "Owner",
        ][..],
        &[
            "vault",
            "role",
            "v",
            "--user",
            "bob@example.com",
            "--role",
            "owner",
        ][..],
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(2);
        assert!(
            out.stderr
                .contains("`owner` cannot be granted. A vault has one owner"),
            "{args:?}:\n{}",
            out.stderr
        );
    }
    for m in any {
        m.assert();
    }
}

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

/// I-309 (A-301, DEC-CL12): `vault members <id>` with a vault-scoped token
/// listed every vault first, which such a token is refused by design, so it
/// failed before asking for the one thing it may see. An id needs no lookup.
#[test]
fn t_i309_a_vault_scoped_token_lists_members_by_id() {
    let mut s = mockito::Server::new();
    let list = s
        .mock("GET", "/api/v1/vaults")
        .with_status(403)
        .with_body(r#"{"error":"Insufficient permissions","code":"FORBIDDEN"}"#)
        .expect(0)
        .create();
    let members = s
        .mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[{"user_id":"u-1","email":"ci-owner@example.com","role":"owner",
            "granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":false}]}"#,
        )
        .expect(1)
        .create();
    let out = with_token(&s.url(), &["vault", "members", VAULT_ID]);
    out.assert_code(0);
    assert!(
        out.stdout.contains(&format!("{VAULT_ID} — 1 member(s)")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("ci-owner@example.com"),
        "{}",
        out.stdout
    );
    list.assert();
    members.assert();
}

/// I-309: by name, a vault-scoped token cannot list, so it is told to pass the
/// id — as `cloud pull` already says — not shown a bare "Insufficient
/// permissions".
#[test]
fn t_i309_by_name_a_token_is_told_to_pass_the_id() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(403)
        .with_body(r#"{"error":"Insufficient permissions","code":"FORBIDDEN"}"#)
        .create();
    let out = with_token(&s.url(), &["vault", "members", "app"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("Pass the vault id instead"),
        "{}",
        out.stderr
    );
}

/// A server with one vault, `app/development`, whose members are `members`
/// (JSON), and nothing else: a key lookup or a POST is a test failure.
fn share_server(your_role: &str, members: &str) -> (mockito::ServerGuard, Vec<mockito::Mock>) {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"{your_role}","version_count":1,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(format!(r#"{{"members":[{members}]}}"#))
        .create();
    let never = vec![
        s.mock("GET", mockito::Matcher::Regex("^/api/v1/users/".into()))
            .expect(0)
            .create(),
        s.mock("GET", mockito::Matcher::Regex("/my-key$".into()))
            .expect(0)
            .create(),
        s.mock("POST", mockito::Matcher::Any).expect(0).create(),
    ];
    (s, never)
}

fn member(email: &str, role: &str, you: bool) -> String {
    format!(
        r#"{{"user_id":"u-{role}","email":"{email}","role":"{role}",
        "granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":{you}}}"#
    )
}

/// I-313 (DEC-VL3): sharing with your own address rewrote your own row — an
/// owner became a developer and could no longer delete the vault. Refused
/// before the key lookup and before the password; nothing is sent.
#[test]
fn t_i313_sharing_with_yourself_is_refused_before_the_password() {
    for role in ["owner", "admin"] {
        let members = format!(
            "{},{}",
            member("me@example.com", role, true),
            member("bob@example.com", "developer", false)
        );
        let (s, never) = share_server(role, &members);
        let out = signed_in(
            &Project::new(),
            &s.url(),
            &["vault", "share", "app", "--with", " ME@example.com "],
            None,
        );
        out.assert_code(2);
        assert!(
            out.stderr.contains(&format!(
                "That is you: you already hold app/development as {role}. \
                 Sharing cannot change your own role."
            )),
            "{role}:\n{}",
            out.stderr
        );
        for m in never {
            m.assert();
        }
    }
}

/// I-313: an admin "sharing" with the owner is how the owner's row was
/// rewritten and the owner then removed. The same sentence as the server's 409.
#[test]
fn t_i313_an_admin_sharing_with_the_owner_is_refused() {
    let members = format!(
        "{},{}",
        member("boss@example.com", "owner", false),
        member("me@example.com", "admin", true)
    );
    let (s, never) = share_server("admin", &members);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "share",
            "app",
            "--with",
            "boss@example.com",
            "--role",
            "viewer",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains(
            "boss@example.com owns app/development. Sharing cannot change the owner's role or key."
        ),
        "{}",
        out.stderr
    );
    for m in never {
        m.assert();
    }
}

/// I-314 (DEC-VL4): sharing with a member silently changed their role and
/// printed "shared". A different role is a role change, so `vault role` is
/// named, with the arguments that would do it.
#[test]
fn t_i314_sharing_with_a_member_points_at_vault_role() {
    let members = format!(
        "{},{}",
        member("me@example.com", "owner", true),
        member("bob@example.com", "developer", false)
    );
    let (s, never) = share_server("owner", &members);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "share",
            "app",
            "--with",
            "bob@example.com",
            "--role",
            "viewer",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("bob@example.com is already a developer on app/development."),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("evnx vault role app --user bob@example.com --role viewer"),
        "{}",
        out.stderr
    );
    for m in never {
        m.assert();
    }
}

/// I-314, owner's choice: asking for the role they already hold changes
/// nothing and is not an error, as `vault role` already treats the same role.
#[test]
fn t_i314_sharing_again_at_the_same_role_changes_nothing() {
    let members = format!(
        "{},{}",
        member("me@example.com", "owner", true),
        member("bob@example.com", "developer", false)
    );
    let (s, never) = share_server("owner", &members);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "share", "app", "--with", "Bob@Example.com"],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains(
            "bob@example.com is already a developer on app/development — nothing changed."
        ),
        "{}",
        out.stdout
    );
    for m in never {
        m.assert();
    }
}

/// Guard: someone who is not a member still gets as far as the key lookup.
#[test]
fn t_i314_a_new_member_still_reaches_the_key_lookup() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"owner","version_count":1,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"members":[{}]}}"#,
            member("me@example.com", "owner", true)
        ))
        .create();
    let keys = s
        .mock("GET", "/api/v1/users/new%40example.com/public-key")
        .with_status(404)
        .with_body(r#"{"error":"Not found","code":"NOT_FOUND"}"#)
        .expect(1)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "share", "app", "--with", "new@example.com"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("no evnx account for new@example.com"),
        "{}",
        out.stderr
    );
    keys.assert();
}
