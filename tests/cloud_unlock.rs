//! The master password is proven before it is used (I-316, A-302, DEC-VL7).
//!
//! `derive_master_key_for_account` turned whatever was typed into a key and
//! never checked it. `vault create` then sealed a new vault under a typo: the
//! vault was created, used a plan slot, and the real password could never open
//! it. Every other command found out later, at the vault key, and said the
//! password was "probably wrong".
//!
//! These tests use real keys: an account whose keypair is sealed under one
//! password, as `/auth/me` returns it.
#![cfg(feature = "cloud")]

mod support;

use evnx_crypto::{
    b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
    generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key, MasterKey, VaultKey,
};
use support::cloud::signed_in;
use support::Project;

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";
const RIGHT: &str = "correct horse battery staple";
const TYPO: &str = "correct horse battery stapel";

/// The sentence every command gives for a password that is not the account's
/// (the one `rotate-master-password` already gave).
const WRONG: &str = "that master password did not open your keypair, so it is not the one this \
                     account was registered with. Nothing has been changed.";

/// An account: what `/auth/me` returns, and the salt its key was derived with.
struct Account {
    me: String,
    salt: [u8; 32],
}

fn account(password: &str) -> Account {
    let salt = generate_salt();
    let master = derive_master_key(password.as_bytes(), &salt).unwrap();
    let sealed = encrypt_private_key(&generate_keypair(), &master)
        .unwrap()
        .to_base64();
    let me = format!(
        r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
        "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
        salt_to_base64(&salt)
    );
    Account { me, salt }
}

fn master(password: &str, a: &Account) -> MasterKey {
    derive_master_key(password.as_bytes(), &a.salt).unwrap()
}

/// `my-key` for the account's own copy, wrapped under `under`.
fn own_copy(vault_key: &VaultKey, under: &MasterKey) -> String {
    let wrapped = wrap_vault_key_with_master_key(vault_key, under).unwrap();
    format!(r#"{{"encrypted_vault_key":"{}"}}"#, b64_encode(&wrapped))
}

/// Version 1 of the vault, as the blob endpoint serves it: nonce, then ciphertext.
fn blob(vault_key: &VaultKey, plaintext: &[u8]) -> Vec<u8> {
    let b = encrypt_vault(plaintext, vault_key, &vault_aad(VAULT_ID, 1)).unwrap();
    let mut bytes = b.nonce.to_vec();
    bytes.extend_from_slice(&b.ciphertext);
    bytes
}

fn me_mock(s: &mut mockito::ServerGuard, a: &Account) {
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .create();
}

fn pull(s: &mockito::ServerGuard, password: &str) -> support::Output {
    signed_in(
        &Project::new(),
        &s.url(),
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
        Some(format!("{password}\n").as_bytes()),
    )
}

/// I-316: a wrong password at `vault create` is refused before the vault
/// exists — before the request that would create it and spend a plan slot.
#[test]
fn t_i316_vault_create_refuses_a_wrong_password_before_creating() {
    let a = account(RIGHT);
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    let create = s.mock("POST", "/api/v1/vaults").expect(0).create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "create", "app", "--password-stdin"],
        Some(format!("{TYPO}\n").as_bytes()),
    );
    out.assert_code(2);
    assert!(out.stderr.contains(WRONG), "{}", out.stderr);
    create.assert();
}

/// Guard: the right password still creates the vault.
#[test]
fn t_i316_vault_create_with_the_right_password_still_creates() {
    let a = account(RIGHT);
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    let create = s
        .mock("POST", "/api/v1/vaults")
        .with_status(201)
        .with_body(format!(
            r#"{{"vault_id":"{VAULT_ID}","name":"app","environment":"development"}}"#
        ))
        .expect(1)
        .create();
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &["vault", "create", "app", "--password-stdin"],
        Some(format!("{RIGHT}\n").as_bytes()),
    );
    out.assert_code(0);
    assert!(
        out.stdout.contains("created app/development"),
        "{}",
        out.stdout
    );
    create.assert();
}

/// I-316: a wrong password at pull is named as such, before the vault key is
/// asked for — not "probably wrong" after it.
#[test]
fn t_i316_pull_with_a_wrong_password_says_so() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, b"A=1\n"))
    .create();
    let my_key = s
        .mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(RIGHT, &a)))
        .expect(0)
        .create();
    let out = pull(&s, TYPO);
    out.assert_code(2);
    assert!(out.stderr.contains(WRONG), "{}", out.stderr);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
    my_key.assert();
}

/// I-316: with the right password proven, a vault's own copy that will not
/// open was sealed under something else — a typo at `vault create` before
/// 0.10.18. Say that, and how to get out of it.
#[test]
fn t_i316_an_own_copy_sealed_under_a_typo_is_explained() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, b"A=1\n"))
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(TYPO, &a)))
        .create();
    let out = pull(&s, RIGHT);
    out.assert_code(2);
    assert!(
        out.stderr.contains("created with a mistyped password"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("evnx 0.10.17"), "{}", out.stderr);
    assert!(!out.stderr.contains("probably wrong"), "{}", out.stderr);
}

/// Guard: the right password still pulls, end to end with real keys.
#[test]
fn t_i316_pull_with_the_right_password_still_decrypts() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, b"A=1\n"))
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(RIGHT, &a)))
        .create();
    let out = pull(&s, RIGHT);
    out.assert_code(0);
    assert_eq!(out.stdout, "A=1\n");
}
