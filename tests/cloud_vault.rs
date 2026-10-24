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
        out.stderr
            .contains("Pass the vault's id in place of its name"),
        "{}",
        out.stderr
    );
    // `vault members` takes the vault as an argument, not `--vault`.
    assert!(!out.stderr.contains("--vault <uuid>"), "{}", out.stderr);
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

// ─── I-311, I-312, I-317, I-318, I-315: the rest of the vault fixes ──────────

/// I-311 (DEC-VL5): sharing needs admin or owner, and a developer or viewer was
/// told "re-sharing is not supported" after typing the password. Now the role
/// is named, from the member list, before the key lookup and the password.
#[test]
fn t_i311_a_developer_is_told_sharing_needs_admin() {
    let members = format!(
        "{},{}",
        member("boss@example.com", "owner", false),
        member("me@example.com", "developer", true)
    );
    let (s, never) = share_server("developer", &members);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "share", "app", "--with", "new@example.com"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("sharing app/development needs the admin or owner role; you are a developer"),
        "{}",
        out.stderr
    );
    for m in never {
        m.assert();
    }
}

/// I-311: an admin may grant only a role below their own, so `--role admin`
/// from an admin is refused before the password (the server's 403 otherwise).
#[test]
fn t_i311_an_admin_cannot_make_an_admin() {
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
            "new@example.com",
            "--role",
            "admin",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("only the owner can make someone an admin"),
        "{}",
        out.stderr
    );
    for m in never {
        m.assert();
    }
}

/// A server for `vault leave` / `vault revoke` with the given members, where
/// only `delete` (the expected DELETE, if any) may change anything.
fn members_server(your_role: &str, members: &str) -> (mockito::ServerGuard, Vec<mockito::Mock>) {
    let (mut s, mut never) = share_server(your_role, members);
    never.push(s.mock("GET", "/api/v1/auth/me").expect(0).create());
    (s, never)
}

/// I-312 (DEC-VL6): a member could not leave from the CLI, though the server
/// allows it. `vault leave` deletes your own membership — one request.
#[test]
fn t_i312_vault_leave_deletes_your_membership() {
    let members = format!(
        "{},{}",
        member("boss@example.com", "owner", false),
        member("me@example.com", "developer", true)
    );
    let (mut s, never) = members_server("developer", &members);
    let delete = s
        .mock(
            "DELETE",
            format!("/api/v1/vaults/{VAULT_ID}/members/u-developer").as_str(),
        )
        .with_status(204)
        .expect(1)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "leave", "app", "-y"],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("left app/development"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("evnx vault rekey"), "{}", out.stdout);
    delete.assert();
    for m in never {
        m.assert();
    }
}

/// I-312: the owner cannot leave — nobody could then delete the vault.
#[test]
fn t_i312_the_owner_cannot_leave() {
    let members = format!(
        "{},{}",
        member("me@example.com", "owner", true),
        member("bob@example.com", "developer", false)
    );
    let (mut s, never) = members_server("owner", &members);
    let delete = s.mock("DELETE", mockito::Matcher::Any).expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "leave", "app", "-y"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("you own app/development, and the owner cannot leave"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("evnx vault delete app"),
        "{}",
        out.stderr
    );
    delete.assert();
    for m in never {
        m.assert();
    }
}

/// I-312: with no terminal and no `-y`, leaving says how to confirm.
#[test]
fn t_i312_leaving_without_a_terminal_names_yes() {
    let members = format!(
        "{},{}",
        member("boss@example.com", "owner", false),
        member("me@example.com", "viewer", true)
    );
    let (mut s, never) = members_server("viewer", &members);
    let delete = s.mock("DELETE", mockito::Matcher::Any).expect(0).create();
    let out = signed_in(&Project::new(), &s.url(), &["vault", "leave", "app"], None);
    out.assert_code(2);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);
    delete.assert();
    for m in never {
        m.assert();
    }
}

/// I-318: revoking yourself said "ask an admin to remove you" — also to the
/// owner, whom nobody can remove. Now: `vault leave`, or why the owner cannot.
#[test]
fn t_i318_revoking_yourself_points_at_vault_leave() {
    for (role, needle) in [
        ("developer", "evnx vault leave app"),
        (
            "owner",
            "you own app/development, and the owner cannot be removed",
        ),
    ] {
        let members = format!(
            "{},{}",
            member("me@example.com", role, true),
            member(
                "x@example.com",
                if role == "owner" { "admin" } else { "owner" },
                false
            )
        );
        let (s, never) = members_server(role, &members);
        let out = signed_in(
            &Project::new(),
            &s.url(),
            &["vault", "revoke", "app", "--user", "me@example.com", "-y"],
            None,
        );
        out.assert_code(2);
        assert!(out.stderr.contains(needle), "{role}:\n{}", out.stderr);
        assert!(!out.stderr.contains("ask an admin"), "{}", out.stderr);
        for m in never {
            m.assert();
        }
    }
}

/// I-317: an admin revoking the owner re-encrypted every version and only then
/// heard "the vault owner cannot be removed", leaving staged blobs behind.
/// Refused first: no password, no key, no blob.
#[test]
fn t_i317_revoking_the_owner_is_refused_before_any_work() {
    let members = format!(
        "{},{}",
        member("boss@example.com", "owner", false),
        member("me@example.com", "admin", true)
    );
    let (s, never) = members_server("admin", &members);
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "revoke", "app", "--user", "boss@example.com", "-y"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("boss@example.com owns app/development, and the owner cannot be removed"),
        "{}",
        out.stderr
    );
    for m in never {
        m.assert();
    }
}

/// I-315 (DEC-VL8): the CLI accepted upper case, which the server refuses with
/// a message about "alphanumeric". The rule is checked here, before the
/// password, as the server states it.
#[test]
fn t_i315_an_upper_case_name_is_refused_before_the_password() {
    let mut s = mockito::Server::new();
    let any: Vec<_> = ["GET", "POST"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "create", "MyApp"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("a vault name may contain only lower-case letters, digits and hyphens"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("\"myapp\""), "{}", out.stderr);
    for m in any {
        m.assert();
    }
}

// ─── I-303, I-319, I-320: the small fixes ─────────────────────────────────────

/// I-303 (DEC-CL11): `--version 0` reached the server and came back as a raw
/// API path. Versions start at 1, so 0 and below are refused by the parser.
#[test]
fn t_i303_a_version_below_one_is_refused() {
    let mut s = mockito::Server::new();
    let any = s.mock("GET", mockito::Matcher::Any).expect(0).create();
    for args in [
        &["cloud", "pull", "--vault", VAULT_ID, "--version", "0"][..],
        &[
            "cloud",
            "run",
            "--vault",
            VAULT_ID,
            "--version",
            "0",
            "--",
            "true",
        ][..],
        &["cloud", "delete-version", "0", "--vault", VAULT_ID, "-y"][..],
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(2);
        assert!(
            out.stderr.contains("invalid value '0'"),
            "{args:?}:\n{}",
            out.stderr
        );
    }
    any.assert();
}

/// I-303: a version that does not exist is named, with where to look — not
/// "not found on …: /api/v1/vaults/<uuid>/versions/7/blob".
#[test]
fn t_i303_a_missing_version_is_named_without_the_api_path() {
    let mut s = mockito::Server::new();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/7/blob").as_str(),
    )
    .with_status(404)
    .with_body(r#"{"error":"Not found","code":"NOT_FOUND"}"#)
    .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "cloud",
            "pull",
            "--vault",
            VAULT_ID,
            "--version",
            "7",
            "--file",
            "-",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains(&format!("version 7 of {VAULT_ID} does not exist")),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("evnx cloud history"), "{}", out.stderr);
    assert!(!out.stderr.contains("/api/v1"), "{}", out.stderr);
}

/// I-319: `PUSHED BY` showed `you` for yourself and an 8-character id for
/// everyone else; the members list has their email.
#[test]
fn t_i319_history_names_who_pushed() {
    let mut s = mockito::Server::new();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/versions").as_str())
        .with_status(200)
        .with_body(
            r#"{"versions":[
            {"version_num":2,"key_count":1,"key_names":["A"],"blob_hash":"h2","blob_size_bytes":40,"pushed_by":"u-bob","pushed_at":"2026-10-21T09:00:00Z"},
            {"version_num":1,"key_count":1,"key_names":["A"],"blob_hash":"h1","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-20T09:00:00Z"}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(r#"{"user_id":"u-me","email":"me@example.com"}"#)
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"members":[{},{}]}}"#,
            r#"{"user_id":"u-me","email":"me@example.com","role":"owner","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":true}"#,
            r#"{"user_id":"u-bob","email":"bob@example.com","role":"developer","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":false}"#
        ))
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["cloud", "history", "--vault", VAULT_ID],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("bob@example.com"), "{}", out.stdout);
    assert!(out.stdout.contains("you"), "{}", out.stdout);
    assert!(!out.stdout.contains("u-bob"), "{}", out.stdout);
}
