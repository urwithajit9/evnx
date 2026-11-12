//! The CI key bundle (`EVNX_KEY_BUNDLE`): an API token unlocks from the bundle
//! and never asks `/auth/me` for key material.
//!
//! Keys here are real: an account whose keypair is sealed under one password,
//! as the server stores it. Nothing secret-like is written as a literal; the
//! token and password are built from parts at run time.
#![cfg(feature = "cloud")]

mod support;

use evnx::cloud::keybundle::KeyBundle;
use evnx_crypto::{
    b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
    generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key, MasterKey, VaultKey,
};
use support::cloud::signed_in;
use support::{run_with, Project};

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

fn token() -> String {
    format!("evnx_tok_{}", "0".repeat(32))
}

fn right() -> String {
    ["ci", "bundle", "right", "phrase"].join("-")
}

fn typo() -> String {
    ["ci", "bundle", "right", "phrasf"].join("-")
}

struct Account {
    me: String,
    bundle: String,
    master: MasterKey,
}

fn account(user_id: &str, password: &str) -> Account {
    let salt = generate_salt();
    let master = derive_master_key(password.as_bytes(), &salt).unwrap();
    let sealed = encrypt_private_key(&generate_keypair(), &master)
        .unwrap()
        .to_base64();
    let salt_b64 = salt_to_base64(&salt);
    let me = format!(
        r#"{{"user_id":"{user_id}","email":"me@example.com","email_verified":true,
        "argon2_salt":"{salt_b64}","encrypted_private_key":"{sealed}",
        "totp_enabled":false,"has_mlkem_key":true}}"#
    );
    let bundle = KeyBundle::new(user_id, &salt_b64, &sealed).encode();
    Account { me, bundle, master }
}

/// The blob and own-key mocks for version 1 of `VAULT_ID`, holding `A=1`.
fn vault_mocks(s: &mut mockito::ServerGuard, master: &MasterKey) {
    let vk = VaultKey::generate();
    let b = encrypt_vault(b"A=1\n", &vk, &vault_aad(VAULT_ID, 1)).unwrap();
    let mut bytes = b.nonce.to_vec();
    bytes.extend_from_slice(&b.ciphertext);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(bytes)
    .create();
    let wrapped = wrap_vault_key_with_master_key(&vk, master).unwrap();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"encrypted_vault_key":"{}"}}"#,
            b64_encode(&wrapped)
        ))
        .create();
}

/// `evnx cloud pull` as CI runs it: a token, the master password on stdin, and
/// whatever extra environment the case sets.
fn ci_pull(server: &str, password: &str, extra: &[(&str, &str)]) -> support::Output {
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let tok = token();
    let mut env = vec![
        ("EVNX_SERVER", server),
        ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ("EVNX_TOKEN", tok.as_str()),
    ];
    env.extend_from_slice(extra);
    run_with(
        p.path(),
        &[
            "cloud",
            "pull",
            "--vault",
            VAULT_ID,
            "--version",
            "1",
            "--file",
            "-",
            "--password-stdin",
        ],
        &env,
        Some(format!("{password}\n").as_bytes()),
    )
}

#[test]
fn a_token_with_a_bundle_pulls_without_asking_auth_me() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    vault_mocks(&mut s, &a.master);
    let out = ci_pull(&s.url(), &right(), &[("EVNX_KEY_BUNDLE", &a.bundle)]);
    out.assert_code(0);
    assert_eq!(out.stdout, "A=1\n");
    me.assert();
}

#[test]
fn the_bundle_can_come_from_the_first_line_of_a_file() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    vault_mocks(&mut s, &a.master);
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("kb.txt");
    // CRLF, and a second line that must be ignored.
    std::fs::write(&f, format!("{}\r\nsecond line\r\n", a.bundle)).unwrap();
    let out = ci_pull(
        &s.url(),
        &right(),
        &[("EVNX_KEY_BUNDLE_FILE", f.to_str().unwrap())],
    );
    out.assert_code(0);
    assert_eq!(out.stdout.replace("\r\n", "\n"), "A=1\n");
    me.assert();
}

#[test]
fn no_bundle_and_a_server_that_refuses_tokens_says_how_to_make_one() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s
        .mock("GET", "/api/v1/auth/me")
        .with_status(403)
        .with_body(r#"{"error":"Forbidden","code":"FORBIDDEN","reason":"TOKEN_NOT_ALLOWED"}"#)
        .expect(1)
        .create();
    vault_mocks(&mut s, &a.master);
    let out = ci_pull(&s.url(), &right(), &[]);
    out.assert_code(11);
    assert!(
        out.stderr.contains(
            "this server no longer gives key material to API tokens: create a key bundle \
             with `evnx auth key-bundle` on a signed-in machine and store it as EVNX_KEY_BUNDLE"
        ),
        "{}",
        out.stderr
    );
    me.assert();
}

#[test]
fn no_bundle_and_an_older_server_still_works() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s
        .mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .expect(1)
        .create();
    vault_mocks(&mut s, &a.master);
    let out = ci_pull(&s.url(), &right(), &[]);
    out.assert_code(0);
    assert_eq!(out.stdout, "A=1\n");
    me.assert();
}

#[test]
fn a_malformed_bundle_is_a_clear_error_and_not_a_fallback() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    vault_mocks(&mut s, &a.master);
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    for bad in ["not-a-bundle", "evnxkb1_%%%", "evnxkb1_e30"] {
        let out = ci_pull(&s.url(), &right(), &[("EVNX_KEY_BUNDLE", bad)]);
        out.assert_code(2);
        assert!(out.stderr.contains("EVNX_KEY_BUNDLE"), "{}", out.stderr);
        assert!(out.stderr.contains("key bundle"), "{}", out.stderr);
        assert!(out.stdout.is_empty(), "{}", out.stdout);
    }
    me.assert();
}

#[test]
fn an_old_bundle_after_a_password_change_says_to_regenerate() {
    // The bundle was made under the old password; the new one does not open it.
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    vault_mocks(&mut s, &a.master);
    let out = ci_pull(&s.url(), &typo(), &[("EVNX_KEY_BUNDLE", &a.bundle)]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("regenerate the key bundle"),
        "{}",
        out.stderr
    );
    me.assert();
}

#[test]
fn auth_status_under_a_token_shows_token_info_when_auth_me_is_forbidden() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(403)
        .with_body(r#"{"error":"Forbidden","code":"FORBIDDEN"}"#)
        .create();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let tok = token();
    let url = s.url();
    let out = run_with(
        p.path(),
        &["auth", "status"],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_TOKEN", tok.as_str()),
        ],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("API token"), "{}", out.stdout);
    assert!(
        out.stdout.contains("need a signed-in session"),
        "{}",
        out.stdout
    );
    assert!(!out.all().contains("refused"), "{}", out.all());
}

/// The last whitespace-separated word of `text` that is a bundle.
fn bundle_in(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("evnxkb1_") && w.len() > 20)
        .map(str::to_string)
}

#[test]
fn key_bundle_prints_a_bundle_that_a_token_can_pull_with() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .create();
    let out = signed_in(&Project::new(), &s.url(), &["auth", "key-bundle"], None);
    out.assert_code(0);
    let printed = bundle_in(&out.stdout).expect("a bundle on stdout");
    assert_eq!(printed, a.bundle);
    assert!(out.stdout.contains("EVNX_KEY_BUNDLE"), "{}", out.stdout);
    assert!(out.stdout.contains("no plaintext secret"), "{}", out.stdout);

    let mut c = mockito::Server::new();
    let me = c.mock("GET", "/api/v1/auth/me").expect(0).create();
    vault_mocks(&mut c, &a.master);
    let pulled = ci_pull(&c.url(), &right(), &[("EVNX_KEY_BUNDLE", &printed)]);
    pulled.assert_code(0);
    assert_eq!(pulled.stdout, "A=1\n");
    me.assert();
}

#[test]
fn key_bundle_quiet_prints_only_the_bundle() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["-q", "auth", "key-bundle"],
        None,
    );
    out.assert_code(0);
    assert_eq!(out.stdout.trim(), a.bundle);
}

#[test]
fn key_bundle_is_refused_for_an_api_token_before_any_request() {
    let mut s = mockito::Server::new();
    let any = s.mock("GET", mockito::Matcher::Any).expect(0).create();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let tok = token();
    let url = s.url();
    let out = run_with(
        p.path(),
        &["auth", "key-bundle"],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_TOKEN", tok.as_str()),
        ],
        None,
    );
    assert_ne!(out.code, Some(0));
    assert!(out.stderr.contains("API token cannot"), "{}", out.stderr);
    any.assert();
}

fn create_token_mock(s: &mut mockito::ServerGuard) {
    s.mock("POST", "/api/v1/auth/tokens")
        .with_status(201)
        .with_body(format!(
            r#"{{"id":"t-1","raw_token":"{}","expires_at":null}}"#,
            token()
        ))
        .create();
}

#[test]
fn token_create_prints_a_bundle_that_a_token_can_pull_with() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    create_token_mock(&mut s);
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["auth", "token", "create", "ci"],
        None,
    );
    out.assert_code(0);
    assert_eq!(bundle_in(&out.stdout).as_deref(), Some(a.bundle.as_str()));
    assert!(out.stdout.contains("EVNX_KEY_BUNDLE"), "{}", out.stdout);
    assert!(
        out.stdout.contains("EVNX_MASTER_PASSWORD"),
        "{}",
        out.stdout
    );
}

#[test]
fn token_create_can_skip_the_bundle() {
    let mut s = mockito::Server::new();
    create_token_mock(&mut s);
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["auth", "token", "create", "ci", "--no-key-bundle"],
        None,
    );
    out.assert_code(0);
    assert!(bundle_in(&out.stdout).is_none(), "{}", out.stdout);
    assert!(out.stdout.contains(&token()), "{}", out.stdout);
    me.assert();
}

#[test]
fn token_create_still_succeeds_when_the_bundle_cannot_be_made() {
    let mut s = mockito::Server::new();
    create_token_mock(&mut s);
    s.mock("GET", "/api/v1/auth/me").with_status(500).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["auth", "token", "create", "ci"],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains(&token()), "{}", out.stdout);
    assert!(
        out.stderr.contains("evnx auth key-bundle"),
        "{}",
        out.stderr
    );
}

#[test]
fn history_under_a_token_does_not_ask_auth_me_who_you_are() {
    let a = account("u-ci", &right());
    let mut s = mockito::Server::new();
    let me = s.mock("GET", "/api/v1/auth/me").expect(0).create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
    )
    .with_status(200)
    .with_body(
        r#"{"versions":[{"version_num":1,"key_count":1,"key_names":["A"],
            "blob_hash":"h","blob_size_bytes":10,"pushed_by":"u-ci",
            "pushed_at":"2026-11-01T10:00:00Z"}]}"#,
    )
    .create();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let tok = token();
    let url = s.url();
    let out = run_with(
        p.path(),
        &["cloud", "history", "--vault", VAULT_ID],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_TOKEN", tok.as_str()),
            ("EVNX_KEY_BUNDLE", a.bundle.as_str()),
        ],
        None,
    );
    out.assert_code(0);
    assert!(out.stdout.contains("you"), "{}", out.stdout);
    me.assert();
}
