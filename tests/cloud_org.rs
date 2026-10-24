//! `evnx org` fixes (U7 Part D1: CG-01, DEC-OR1, DEC-OR2, DEC-CL8, DEC-AU6/7).
//!
//! - I-214: `org delete` sent `force: true` whenever seats were assigned, so the
//!   `--force` the help presents as a safety gate gated nothing.
//! - I-215, I-216: slug, name and invite email went to the server unchecked.
//! - I-218: `org` read raw stdin, so `echo y |` confirmed a removal.
//! - I-219: invite expiry printed as `2026-10-20T09:00:00.123456Z`.
#![cfg(feature = "cloud")]

mod support;

use support::cloud::signed_in;
use support::Project;

const ORG_ID: &str = "11111111-1111-1111-1111-111111111111";

fn org_server(seats_used: i64) -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/orgs")
        .with_status(200)
        .with_body(format!(
            r#"{{"organizations":[{{"id":"{ORG_ID}","name":"Acme","slug":"acme","plan":"team",
            "your_role":"owner","you_hold_a_seat":true,"seats":{{"used":{seats_used},"purchased":5}}}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[
            {"user_id":"u-me","email":"me@example.com","role":"owner","holds_a_seat":true,"is_you":true},
            {"user_id":"u-bob","email":"bob@example.com","role":"member","holds_a_seat":false,"is_you":false}]}"#,
        )
        .create();
    s
}

fn no_delete(s: &mut mockito::ServerGuard) -> mockito::Mock {
    s.mock("DELETE", mockito::Matcher::Any).expect(0).create()
}

#[test]
fn t_i214_delete_with_seats_needs_force_and_sends_none() {
    let mut s = org_server(2);
    let del = no_delete(&mut s);
    let out = signed_in(&Project::new(), &s.url(), &["org", "delete", "-y"], None);
    out.assert_code(2);
    assert!(out.stderr.contains("--force"), "{}", out.stderr);
    del.assert();
}

#[test]
fn t_i214_force_is_sent_only_with_the_flag() {
    for (seats, extra, force) in [(2, &["--force"][..], true), (0, &[][..], false)] {
        let mut s = org_server(seats);
        let del = s
            .mock("DELETE", format!("/api/v1/orgs/{ORG_ID}").as_str())
            .match_body(mockito::Matcher::Json(
                serde_json::json!({ "force": force }),
            ))
            .with_status(204)
            .expect(1)
            .create();
        let mut args = vec!["org", "delete", "-y"];
        args.extend_from_slice(extra);
        let out = signed_in(&Project::new(), &s.url(), &args, None);
        out.assert_code(0);
        del.assert();
    }
}

#[test]
fn t_i215_t_i216_bad_slug_name_and_email_are_refused_before_any_request() {
    let mut s = mockito::Server::new();
    let any_post = s.mock("POST", mockito::Matcher::Any).expect(0).create();
    for (args, needle) in [
        (
            &["org", "create", "Acme", "--slug", "Bad Slug!"][..],
            "slug",
        ),
        (&["org", "create", "Acme", "--slug", "-acme"][..], "slug"),
        (&["org", "create", "   ", "--slug", "acme"][..], "name"),
        (
            &["org", "invite", "not-an-email", "--org", "acme"][..],
            "email",
        ),
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, None);
        out.assert_code(2);
        assert!(out.stderr.contains(needle), "{args:?}:\n{}", out.stderr);
    }
    any_post.assert();
}

/// I-218: `echo y |` is not a terminal, so it is not consent: stop, name --yes.
#[test]
fn t_i218_piped_yes_does_not_confirm_a_removal() {
    let mut s = org_server(0);
    let del = no_delete(&mut s);
    for args in [
        &["org", "remove", "bob@example.com"][..],
        &["org", "delete"][..],
    ] {
        let out = signed_in(&Project::new(), &s.url(), args, Some(b"y\n"));
        out.assert_code(2);
        assert!(out.stderr.contains("--yes"), "{args:?}:\n{}", out.stderr);
    }
    del.assert();
}

#[test]
fn t_i219_invite_expiry_is_readable() {
    let mut s = org_server(0);
    s.mock("GET", format!("/api/v1/orgs/{ORG_ID}/invites").as_str())
        .with_status(200)
        .with_body(
            r#"{"invites":[{"id":"inv-1","email":"new@example.com","role":"member",
            "expires_at":"2026-10-20T09:00:00.123456Z"}]}"#,
        )
        .create();
    let out = signed_in(&Project::new(), &s.url(), &["org", "invites"], None);
    out.assert_code(0);
    assert!(out.stdout.contains("2026-10-20 09:00"), "{}", out.stdout);
    assert!(!out.stdout.contains("09:00:00.123456Z"), "{}", out.stdout);
}

/// DEC-VL10 (A-303): an organisation role is read the way a vault role is —
/// spaces trimmed, any case — and a role that is not one gets the same shape of
/// message. The reason `owner` cannot be granted is kept.
#[test]
fn t_dec_vl10_org_roles_are_read_like_vault_roles() {
    let mut s = org_server(0);
    let invite = s
        .mock("POST", format!("/api/v1/orgs/{ORG_ID}/invites").as_str())
        .match_body(mockito::Matcher::Json(
            serde_json::json!({ "email": "new@example.com", "role": "admin" }),
        ))
        .with_status(201)
        .with_body(r#"{"email":"new@example.com","role":"admin","token":"t","expires_in_days":7}"#)
        .expect(1)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "invite", "new@example.com", "--role", " Admin "],
        None,
    );
    out.assert_code(0);
    invite.assert();

    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "role", "bob@example.com", "--set", "developer"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr
            .contains("role must be one of: member, admin. Got `developer`."),
        "{}",
        out.stderr
    );

    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["org", "invite", "new@example.com", "--role", "owner"],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("`owner` cannot be granted.")
            && out.stderr.contains("exactly one owner"),
        "{}",
        out.stderr
    );
}
