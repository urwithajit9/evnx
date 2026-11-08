//! `--vault` given more than once on `cloud run` and `cloud pull` (RM-59, X4,
//! GitHub #128): merged left to right, an overlap refused unless
//! `--allow-override`, `--version` refused with several.
#![cfg(all(unix, feature = "cloud"))]

mod support;

use evnx_crypto::{
    b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
    generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key, VaultKey,
};
use support::cloud::signed_in;
use support::Project;

const PW: &str = "correct horse battery staple";
const SHARED_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";
const PROD_ID: &str = "7c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f";
/// Labels as the list gives them: `name/environment`.
const SHARED: &str = "shared/development";
const PROD: &str = "app/production";

/// A server with two vaults, each at version 1 holding the given text.
fn server(shared: &str, prod: &str) -> mockito::ServerGuard {
    let salt = generate_salt();
    let master = derive_master_key(PW.as_bytes(), &salt).unwrap();
    let keypair = generate_keypair();
    let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
    let me = format!(
        r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
        "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
        salt_to_base64(&salt)
    );
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(me)
        .create();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[
            {{"id":"{SHARED_ID}","name":"shared","environment":"development","role":"owner",
              "version_count":1,"updated_at":"2026-01-01T00:00:00Z"}},
            {{"id":"{PROD_ID}","name":"app","environment":"production","role":"owner",
              "version_count":1,"updated_at":"2026-01-01T00:00:00Z"}}]}}"#
        ))
        .create();
    for (id, text) in [(SHARED_ID, shared), (PROD_ID, prod)] {
        let vk = VaultKey::generate();
        let b = encrypt_vault(text.as_bytes(), &vk, &vault_aad(id, 1)).unwrap();
        let mut bytes = b.nonce.to_vec();
        bytes.extend_from_slice(&b.ciphertext);
        let wrapped = wrap_vault_key_with_master_key(&vk, &master).unwrap();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{id}/versions/latest").as_str(),
        )
        .with_status(200)
        .with_body(r#"{"version_num":1}"#)
        .create();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{id}/versions/1/blob").as_str(),
        )
        .with_status(200)
        .with_body(bytes)
        .create();
        s.mock("GET", format!("/api/v1/vaults/{id}/my-key").as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"encrypted_vault_key":"{}"}}"#,
                b64_encode(&wrapped)
            ))
            .create();
    }
    s
}

fn go(s: &mockito::ServerGuard, p: &Project, args: &[&str]) -> support::Output {
    signed_in(p, &s.url(), args, Some(format!("{PW}\n").as_bytes()))
}

const PULL2: &[&str] = &[
    "cloud",
    "pull",
    "--vault",
    "shared",
    "--vault",
    "app/production",
    "--file",
    "-",
    "--password-stdin",
];

fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    let mut v = base.to_vec();
    v.extend_from_slice(extra);
    v
}

#[test]
fn disjoint_vaults_merge_first_vault_keys_first() {
    let s = server("B_FIRST=1\nA_SECOND=2\n", "C_THIRD=3\n");
    let out = go(&s, &Project::new(), PULL2);
    out.assert_code(0);
    // First vault's keys in its own order, then the second's new keys.
    assert_eq!(out.stdout, "B_FIRST=1\nA_SECOND=2\nC_THIRD=3\n");
}

#[test]
fn run_injects_the_merge() {
    let s = server("A=1\n", "C=3\n");
    let out = go(
        &s,
        &Project::new(),
        &[
            "cloud",
            "run",
            "--vault",
            "shared",
            "--vault",
            "app/production",
            "--password-stdin",
            "--",
            "sh",
            "-c",
            "printf '%s,%s' \"$A\" \"$C\"",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "1,3");
}

#[test]
fn an_overlap_is_refused_naming_key_and_vaults_and_writes_nothing() {
    let s = server(
        "DATABASE_URL=sekret-one-xyz\n",
        "DATABASE_URL=sekret-two-xyz\nX=1\n",
    );
    let p = Project::new();
    for args in [
        with(&PULL2[..6], &["--password-stdin"]),
        vec![
            "cloud",
            "run",
            "--vault",
            "shared",
            "--vault",
            "app/production",
            "--password-stdin",
            "--",
            "touch",
            "ran",
        ],
    ] {
        let out = go(&s, &p, &args);
        assert_eq!(out.code, Some(2), "{args:?}");
        assert!(
            out.stderr
                .contains(&format!("DATABASE_URL is in both {SHARED} and {PROD}")),
            "{}",
            out.stderr
        );
        assert!(out.stderr.contains("--allow-override"), "{}", out.stderr);
        assert!(!out.all().contains("sekret"), "{}", out.all());
        assert!(!p.path().join(".env").exists());
        assert!(!p.path().join("ran").exists());
    }
}

#[test]
fn allow_override_lets_the_later_vault_win_and_names_the_overlap() {
    let s = server("K=old\nA=1\n", "K=new\n");
    let out = go(&s, &Project::new(), &with(PULL2, &["--allow-override"]));
    out.assert_code(0);
    assert_eq!(out.stdout, "K=new\nA=1\n");
    assert!(
        out.stderr.contains("K is in") && out.stderr.contains(&format!("{PROD} wins")),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("old") && !out.stderr.contains("=new"));
}

#[test]
fn a_version_with_several_vaults_is_refused_before_any_request() {
    let mut s = mockito::Server::new();
    let any = s.mock("GET", mockito::Matcher::Any).expect(0).create();
    let p = Project::new();
    for args in [
        with(PULL2, &["--version", "1"]),
        vec![
            "cloud",
            "run",
            "--vault",
            "a",
            "--vault",
            "b",
            "--version",
            "1",
            "--",
            "true",
        ],
    ] {
        let out = signed_in(&p, &s.url(), &args, None);
        assert_eq!(out.code, Some(2), "{args:?}");
        assert!(
            out.stderr
                .contains("--version selects a version of one vault"),
            "{}",
            out.stderr
        );
    }
    any.assert();
}

#[test]
fn one_vault_is_unchanged() {
    let s = server("A=1\n", "C=3\n");
    let out = go(
        &s,
        &Project::new(),
        &[
            "cloud",
            "pull",
            "--vault",
            "shared",
            "--file",
            "-",
            "--password-stdin",
        ],
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "A=1\n");
    assert!(
        out.stderr
            .contains("pulled version 1 of shared/development to stdout"),
        "{}",
        out.stderr
    );
}

#[test]
fn the_flags_are_documented() {
    for cmd in ["pull", "run"] {
        let out = support::run(Project::new().path(), &["cloud", cmd, "--help"]);
        out.assert_code(0);
        assert!(out.stdout.contains("--allow-override"), "{}", out.stdout);
        assert!(out.stdout.contains("Repeatable"), "{}", out.stdout);
    }
}
