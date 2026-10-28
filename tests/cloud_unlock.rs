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
    generate_salt, salt_to_base64, vault_aad, wrap_vault_key_for_user,
    wrap_vault_key_with_master_key, MasterKey, UserKeypair, UserPublicKeys, VaultKey,
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
    keypair: UserKeypair,
}

fn account(password: &str) -> Account {
    let salt = generate_salt();
    let master = derive_master_key(password.as_bytes(), &salt).unwrap();
    let keypair = generate_keypair();
    let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
    let me = format!(
        r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
        "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
        salt_to_base64(&salt)
    );
    Account { me, salt, keypair }
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
    // I-320: the hint pushed nowhere — `vault create` binds no directory.
    assert!(
        out.stdout
            .contains("evnx cloud push --vault app/development"),
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

/// I-311 (DEC-VL5): an admin holds a copy shared to them — the hybrid wrap —
/// and `share` refused it ("re-sharing is not supported") after the password,
/// though the server lets admins share. It is opened with the keypair, as
/// `vault rekey` already did, and re-wrapped for the recipient.
#[test]
fn t_i311_an_admin_shares_a_vault_shared_with_them() {
    let admin = account(RIGHT);
    let vk = VaultKey::generate();
    let mine = wrap_vault_key_for_user(
        &vk,
        &UserPublicKeys::from_base64(
            &admin.keypair.x25519_public_base64(),
            &admin.keypair.mlkem_public_base64(),
        )
        .unwrap(),
    )
    .unwrap();
    let recipient = generate_keypair();

    let mut s = mockito::Server::new();
    me_mock(&mut s, &admin);
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"admin","version_count":1,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/members").as_str())
        .with_status(200)
        .with_body(
            r#"{"members":[
            {"user_id":"u-boss","email":"boss@example.com","role":"owner","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":false},
            {"user_id":"u-me","email":"me@example.com","role":"admin","granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":true}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/users/new%40example.com/public-key")
        .with_status(200)
        .with_body(format!(
            r#"{{"x25519_public_key":"{}","mlkem_public_key":"{}"}}"#,
            recipient.x25519_public_base64(),
            recipient.mlkem_public_base64()
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"encrypted_vault_key":"{}","eph_pub_key":"{}","mlkem_ciphertext":"{}"}}"#,
            mine.encrypted_vault_key_base64(),
            mine.eph_pub_key_base64(),
            mine.mlkem_ciphertext_base64()
        ))
        .create();
    let add = s
        .mock(
            "POST",
            format!("/api/v1/vaults/{VAULT_ID}/members").as_str(),
        )
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "user_email": "new@example.com",
            "role": "developer"
        })))
        .with_status(201)
        .expect(1)
        .create();

    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "share",
            "app",
            "--with",
            "new@example.com",
            "--password-stdin",
        ],
        Some(format!("{RIGHT}\n").as_bytes()),
    );
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("shared app/development with new@example.com as developer"),
        "{}",
        out.stdout
    );
    add.assert();
}

/// I-218: `cloud pull` over a different `.env` asks first; with no terminal it
/// failed "IO error: not a terminal". Now it names `--force`, and the file is
/// left as it was.
#[test]
fn t_i218_pull_over_a_different_file_names_force_without_a_terminal() {
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
    let p = Project::new();
    std::fs::write(p.path().join(".env"), "A=local\n").unwrap();
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "cloud",
            "pull",
            "--vault",
            VAULT_ID,
            "--version",
            "1",
            "--password-stdin",
        ],
        Some(format!("{RIGHT}\n").as_bytes()),
    );
    out.assert_code(2);
    assert!(out.stderr.contains("--force"), "{}", out.stderr);
    // RM-85 (F1): what a pull does, the right way round.
    assert!(
        out.stdout.contains(&format!(
            "differs from version 1 of {VAULT_ID}; pulling replaces it"
        )),
        "{}",
        out.stdout
    );
    assert!(
        !out.all().contains("to a production vault from"),
        "{}",
        out.all()
    );
    assert!(
        !out.stderr.contains("not a terminal: not a terminal"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(p.path().join(".env")).unwrap(),
        "A=local\n"
    );
}

/// RM-34: `cloud pull --force` over a different `.env` writes through the
/// shared writer: the replaced file is kept as `.env.bak` and is named; an
/// existing file keeps its mode, with a warning when others can read it; a
/// pull of identical content keeps no copy.
#[test]
fn rm34_pull_keeps_the_replaced_file_and_its_mode() {
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
    let p = Project::new()
        .file(".gitignore", ".env*\n")
        .env("A=local\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o644);
    let stdin = format!("{RIGHT}\n");
    let pull = || {
        signed_in(
            &p,
            &s.url(),
            &[
                "cloud",
                "pull",
                "--vault",
                VAULT_ID,
                "--version",
                "1",
                "--force",
                "--password-stdin",
            ],
            Some(stdin.as_bytes()),
        )
    };

    let out = pull();
    out.assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n");
    assert_eq!(p.read(".env.bak"), "A=local\n");
    // `.\.env` on Windows.
    let sep = std::path::MAIN_SEPARATOR;
    assert!(
        out.all()
            .contains(&format!("kept the previous .{sep}.env as .{sep}.env.bak")),
        "{}",
        out.all()
    );
    #[cfg(unix)]
    {
        assert_eq!(
            evnx::core::permissions::mode(&p.path().join(".env")),
            Some(0o644),
            "an existing file keeps its mode"
        );
        assert!(out.stderr.contains("chmod"), "{}", out.stderr);
    }

    // Already up to date: nothing more is kept.
    let out = pull();
    out.assert_code(0);
    assert!(!p.path().join(".env.bak.2").exists());
}

/// I-225 (DEC-CL3): with real keys, `-q` silences `vault create` and
/// `cloud push`, and `cloud pull --file -` prints the file and nothing else.
#[test]
fn t_i225_quiet_create_push_and_pull() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock("POST", "/api/v1/vaults")
        .with_status(201)
        .with_body(format!(
            r#"{{"vault_id":"{VAULT_ID}","name":"app","environment":"development"}}"#
        ))
        .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/latest").as_str(),
    )
    .with_status(404)
    .with_body(r#"{"error":"Not found","code":"NOT_FOUND"}"#)
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(RIGHT, &a)))
        .create();
    s.mock(
        "POST",
        format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
    )
    .with_status(201)
    .with_body(r#"{"version_num":1,"pushed_at":"2026-10-24T09:00:00Z"}"#)
    .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, b"A=1\n"))
    .create();

    let p = Project::new();
    std::fs::write(p.path().join(".env"), "A=1\n").unwrap();
    let stdin = format!("{RIGHT}\n");
    for args in [
        &["-q", "vault", "create", "app", "--password-stdin"][..],
        &[
            "-q",
            "cloud",
            "push",
            "--vault",
            VAULT_ID,
            "--password-stdin",
        ][..],
    ] {
        let out = signed_in(&p, &s.url(), args, Some(stdin.as_bytes()));
        out.assert_code(0);
        assert_eq!(out.stdout, "", "{args:?}");
        assert_eq!(out.stderr, "", "{args:?}");
    }
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "-q",
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
        Some(stdin.as_bytes()),
    );
    out.assert_code(0);
    assert_eq!(out.stdout, "A=1\n");
    assert_eq!(out.stderr, "");
}

/// R-template (owner, 2026-10-17): `cloud pull` writes plaintext secrets into
/// the project and did nothing about git. It now takes the shared step
/// `template`, `convert` and `restore` use: in a repository, a file git would
/// pick up is warned about without a terminal, or added with `--gitignore`.
/// `--file -` writes no file and checks nothing.
#[test]
fn t_dec_t11_pull_protects_the_file_it_writes() {
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
    let p = Project::new().git();
    let stdin = format!("{RIGHT}\n");
    let pull = |extra: &[&str]| {
        let mut args = vec![
            "cloud",
            "pull",
            "--vault",
            VAULT_ID,
            "--version",
            "1",
            "--password-stdin",
        ];
        args.extend_from_slice(extra);
        signed_in(&p, &s.url(), &args, Some(stdin.as_bytes()))
    };

    let out = pull(&[]);
    out.assert_code(0);
    assert!(
        out.stderr
            .contains("'.env' may contain secrets and is not in .gitignore"),
        "{}",
        out.stderr
    );

    let out = pull(&["--file", "prod.env", "--gitignore"]);
    out.assert_code(0);
    let ignore = std::fs::read_to_string(p.path().join(".gitignore")).unwrap_or_default();
    assert!(ignore.lines().any(|l| l == "prod.env"), "{ignore}");

    let out = pull(&["--file", "-"]);
    out.assert_code(0);
    assert!(!out.stderr.contains(".gitignore"), "{}", out.stderr);
}

/// A server holding one production vault, `app/production`, for the I-234
/// push tests: its list, an account, the vault's own copy, and a push target.
fn production_server(a: &Account, vk: &VaultKey) -> (mockito::ServerGuard, mockito::Mock) {
    let mut s = mockito::Server::new();
    me_mock(&mut s, a);
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"production",
            "role":"owner","version_count":0,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/latest").as_str(),
    )
    .with_status(404)
    .with_body(r#"{"error":"Not found","code":"NOT_FOUND"}"#)
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(vk, &master(RIGHT, a)))
        .create();
    let push = s
        .mock(
            "POST",
            format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
        )
        .with_status(201)
        .with_body(r#"{"version_num":1,"pushed_at":"2026-10-24T09:00:00Z"}"#)
        .create();
    (s, push)
}

/// I-234 (DEC-CL9): a forgotten `--file` pushed local development secrets
/// into production, with only a warning. On a terminal it now asks; without
/// one it is refused unless `--yes` — before the password, nothing sent. By id,
/// a signed-in session looks the environment up for this question (owner,
/// 2026-10-25); the file stays `.env`.
#[test]
fn t_i234_a_default_env_push_to_production_needs_yes_without_a_terminal() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    for target in ["app/production", VAULT_ID] {
        let (s, push) = production_server(&a, &vk);
        let p = Project::new();
        std::fs::write(p.path().join(".env"), "A=dev\n").unwrap();
        let out = signed_in(
            &p,
            &s.url(),
            &["cloud", "push", "--vault", target, "--password-stdin"],
            Some(format!("{RIGHT}\n").as_bytes()),
        );
        out.assert_code(2);
        assert!(out.stderr.contains("--yes"), "{target}:\n{}", out.stderr);
        assert!(
            out.stderr.contains("production"),
            "{target}:\n{}",
            out.stderr
        );
        push.expect(0).assert();
    }
}

/// Guards: `--yes` pushes it, and a file named on purpose is not asked about.
#[test]
fn t_i234_yes_or_a_named_file_pushes_to_production() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    for extra in [&["--yes"][..], &["--file", ".env"][..]] {
        let (s, push) = production_server(&a, &vk);
        let p = Project::new();
        std::fs::write(p.path().join(".env"), "A=dev\n").unwrap();
        let mut args = vec![
            "cloud",
            "push",
            "--vault",
            "app/production",
            "--password-stdin",
        ];
        args.extend_from_slice(extra);
        let out = signed_in(&p, &s.url(), &args, Some(format!("{RIGHT}\n").as_bytes()));
        out.assert_code(0);
        push.expect(1).assert();
    }
}

// ── RM-07: `cloud export --to` takes convert's names and collision rule ──────

const COLLIDING: &[u8] = b"A_B=1\na_b=2\nDB=x\n";

/// A server holding one version of `plaintext`, readable with `RIGHT`.
fn export_server(plaintext: &[u8]) -> mockito::ServerGuard {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
    )
    .with_status(200)
    .with_body(
        r#"{"versions":[{"version_num":1,"key_count":3,"key_names":[],
            "blob_hash":"h","blob_size_bytes":1,"pushed_by":"u-me",
            "pushed_at":"2026-10-27T00:00:00Z"}]}"#,
    )
    .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, plaintext))
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(RIGHT, &a)))
        .create();
    s
}

fn export(p: &Project, s: &mockito::ServerGuard, extra: &[&str]) -> support::Output {
    let mut args = vec!["cloud", "export", "--vault", VAULT_ID, "-o", "out"];
    args.extend_from_slice(extra);
    args.push("--password-stdin");
    signed_in(p, &s.url(), &args, Some(format!("{RIGHT}\n").as_bytes()))
}

/// The one `v1.*` file the export wrote.
fn exported(p: &Project) -> String {
    let dir = std::fs::read_dir(p.path().join("out"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|f| f.file_name().unwrap().to_string_lossy().starts_with("v1."))
        .unwrap();
    std::fs::read_to_string(file).unwrap()
}

/// The names come from the flags, and the file is what `convert` prints for
/// the same input and flags. 0.10.18 wrote `my-project`.
#[test]
fn rm07_export_to_uses_the_named_project_like_convert() {
    let s = export_server(b"DB=x\nTOKEN=y\n");
    let p = Project::new().env("DB=x\nTOKEN=y\n");
    export(&p, &s, &["--to", "gcp-secrets", "--project", "acme-prod"]).assert_code(0);
    let file = exported(&p);
    assert!(!file.contains("my-project"), "{file}");
    let convert = support::run(
        p.path(),
        &["convert", "--to", "gcp-secrets", "--project", "acme-prod"],
    );
    convert.assert_code(0);
    assert_eq!(file, convert.stdout);
}

/// A format that needs a name refuses without it, before any request, and
/// the hint names `cloud export`.
#[test]
fn rm07_export_to_without_its_name_is_refused_before_any_request() {
    let mut s = mockito::Server::new();
    let any: Vec<_> = ["GET", "POST"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    let p = Project::new();
    for (to, flag) in [
        ("gcp-secrets", "--project"),
        ("azure-keyvault", "--vault-name"),
        ("heroku", "--heroku-app"),
        ("kubernetes", "--name"),
    ] {
        let out = export(&p, &s, &["--to", to]);
        out.assert_code(2)
            .assert_contains(flag)
            .assert_contains(&format!("evnx cloud export --to {to} {flag}"));
    }
    // A flag meant for another format is refused the same way `convert` does.
    export(&p, &s, &["--to", "heroku", "--project", "acme-prod"])
        .assert_code(2)
        .assert_contains("--project is for --to gcp-secrets");
    for m in any {
        m.assert();
    }
}

/// Two names the format merges are refused, as `convert` refuses them;
/// `--allow-collisions` keeps the last value.
#[test]
fn rm07_export_to_refuses_a_collision() {
    let s = export_server(COLLIDING);
    let p = Project::new();
    let to = ["--to", "gcp-secrets", "--project", "acme-prod"];
    export(&p, &s, &to)
        .assert_code(2)
        .assert_contains("A_B")
        .assert_contains("a_b")
        .assert_contains("--allow-collisions");

    let p = Project::new();
    let mut allow = to.to_vec();
    allow.push("--allow-collisions");
    export(&p, &s, &allow).assert_code(0);
    let file = exported(&p);
    assert_eq!(file.matches(" a-b").count(), 1, "{file}");
}

/// RM-86 (F3): with `[cloud] file`, pull from any folder under the binding
/// writes the same file, named by its absolute path; it used to be `./.env`
/// wherever the command ran.
#[test]
fn rm86_the_bound_file_is_the_same_from_every_folder() {
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
    let p = Project::new()
        .file(
            ".evnx.toml",
            &format!("[cloud]\nvault = \"{VAULT_ID}\"\nfile = \"deploy/.env\"\n"),
        )
        .file(".gitignore", ".env*\n")
        .file("deploy/.keep", "")
        .file("tools/.keep", "");
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let out = support::run_with(
        &p.path().join("tools"),
        &["cloud", "pull", "--version", "1", "--password-stdin"],
        &[
            ("EVNX_SERVER", s.url().as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
        Some(format!("{RIGHT}\n").as_bytes()),
    );
    out.assert_code(0);
    assert_eq!(p.read("deploy/.env"), "A=1\n");
    assert!(!p.path().join("tools/.env").exists());
    assert!(out.all().contains("([cloud] file)"), "{}", out.all());
    let deploy = std::path::Path::new("deploy").join(".env");
    assert!(
        out.all().contains(&deploy.display().to_string()),
        "{}",
        out.all()
    );
}

/// RM-87 (N4): `cloud diff --local` compares the local file with a version in
/// memory: key names only, exit 0 when identical and 1 when not, and nothing
/// written.
#[test]
fn rm87_diff_local_lists_keys_and_writes_nothing() {
    let a = account(RIGHT);
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    me_mock(&mut s, &a);
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob(&vk, b"A=1\nB=2\n"))
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master(RIGHT, &a)))
        .create();
    let diff = |p: &Project| {
        signed_in(
            p,
            &s.url(),
            &[
                "cloud",
                "diff",
                "--local",
                "--vault",
                VAULT_ID,
                "--version",
                "1",
                "--password-stdin",
            ],
            Some(format!("{RIGHT}\n").as_bytes()),
        )
    };

    let same = Project::new().env("A=1\nB=2\n");
    let out = diff(&same);
    out.assert_code(0);
    assert!(out.stdout.contains("Identical"), "{}", out.stdout);

    let p = Project::new().env("A=9\nC=3\n");
    // Everything in the project but `cfg`, which the test's login creates.
    let files = |p: &Project| {
        let mut v: Vec<String> = std::fs::read_dir(p.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "cfg")
            .collect();
        v.sort();
        v
    };
    let before = files(&p);
    let out = diff(&p);
    out.assert_code(1);
    assert!(
        out.stdout.contains("+ B  only in the vault"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("- C  only in"), "{}", out.stdout);
    assert!(out.stdout.contains("~ A  value differs"), "{}", out.stdout);
    // Values never appear.
    assert!(
        !out.all().contains("A=9") && !out.all().contains("B=2"),
        "{}",
        out.all()
    );
    assert_eq!(p.read(".env"), "A=9\nC=3\n");
    assert_eq!(before, files(&p), "diff --local wrote a file");
}
