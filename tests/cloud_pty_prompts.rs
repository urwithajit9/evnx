//! A master password typed at a prompt is read, and the command goes on.
//!
//! 0.14.0 held `stdin().lock()` while `dialoguer` prompted. `Stdin`'s lock is
//! not re-entrant and the prompt takes it again to read the terminal, so every
//! interactive cloud command showed its prompt and then never read a key. The
//! unit test beside `resolve_with_stdin` covers the one function; nothing
//! typed a password into the real binary, which is what these do.
//!
//! Each test runs the binary on a pseudo-terminal against a local mock server,
//! waits for the prompt and for the terminal to stop echoing, types a master
//! password, and waits for the command to finish. A command that never reads
//! the password never finishes: the wait times out and the test fails by
//! timeout, not by hanging the suite (`Pty::wait_for` and `Pty::finish` give up
//! after 15 s).
//!
//! The mock servers are the ones the other cloud tests use: an account with
//! real keys sealed under the password (`tests/cloud_unlock.rs`) and the
//! server half of SRP (`src/cloud/auth.rs` unit tests).
//!
//! `vault share` and `add`'s prompts are not driven here: `add` has its own
//! pty tests (`tests/add_pty.rs`), and `vault share` needs a second account's
//! public keys on the mock server.
#![cfg(all(unix, feature = "cloud"))]

mod support;

use evnx_crypto::{
    MasterKey, VaultKey, b64_encode, compute_verifier, derive_master_key, derive_srp_password,
    encrypt_private_key, encrypt_vault, generate_keypair, generate_salt, salt_to_base64, vault_aad,
    wrap_vault_key_with_master_key,
};
use support::Project;
use support::pty::Pty;

const VAULT_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";
const PASSWORD: &str = "correct horse battery staple";
const EMAIL: &str = "me@example.com";

// ─── Mock servers ────────────────────────────────────────────────────────────

/// `/auth/me` for an account whose keypair is sealed under `PASSWORD`.
fn account_json() -> (String, MasterKey) {
    let salt = generate_salt();
    let master = derive_master_key(PASSWORD.as_bytes(), &salt).unwrap();
    let keypair = generate_keypair();
    let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
    let me = format!(
        r#"{{"user_id":"u-me","email":"{EMAIL}","argon2_salt":"{}",
        "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
        salt_to_base64(&salt)
    );
    (me, master)
}

fn own_copy(vault_key: &VaultKey, under: &MasterKey) -> String {
    let wrapped = wrap_vault_key_with_master_key(vault_key, under).unwrap();
    format!(r#"{{"encrypted_vault_key":"{}"}}"#, b64_encode(&wrapped))
}

/// A server holding version 1 of the vault, `A=1`, readable with `PASSWORD`.
fn pull_server() -> mockito::ServerGuard {
    let (me, master) = account_json();
    let vk = VaultKey::generate();
    let sealed = encrypt_vault(b"A=1\n", &vk, &vault_aad(VAULT_ID, 1)).unwrap();
    let mut blob = sealed.nonce.to_vec();
    blob.extend_from_slice(&sealed.ciphertext);

    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(me)
        .create();
    s.mock(
        "GET",
        format!("/api/v1/vaults/{VAULT_ID}/versions/1/blob").as_str(),
    )
    .with_status(200)
    .with_body(blob)
    .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/my-key").as_str())
        .with_status(200)
        .with_body(own_copy(&vk, &master))
        .create();
    s
}

/// A server with one empty vault that accepts a first version; the returned
/// mock is the push.
fn push_server() -> (mockito::ServerGuard, mockito::Mock) {
    let (me, master) = account_json();
    let vk = VaultKey::generate();
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(me)
        .create();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
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
        .with_body(own_copy(&vk, &master))
        .create();
    let push = s
        .mock(
            "POST",
            format!("/api/v1/vaults/{VAULT_ID}/versions").as_str(),
        )
        .with_status(201)
        .with_body(r#"{"version_num":1,"pushed_at":"2026-10-24T09:00:00Z"}"#)
        .expect(1)
        .create();
    (s, push)
}

/// The server half of SRP for `EMAIL` / `PASSWORD`, answering init and verify.
fn srp_server() -> mockito::ServerGuard {
    use sha2::Sha256;
    use srp::groups::G_2048;
    use srp::server::SrpServer;
    use std::sync::{Arc, Mutex};

    let srp_salt = generate_salt();
    let argon2_salt = generate_salt();
    let srp_password = derive_srp_password(PASSWORD.as_bytes(), &srp_salt).unwrap();
    let verifier = compute_verifier(EMAIL, srp_password, srp_salt).unwrap();
    let v = verifier.verifier.clone();

    // A fixed private ephemeral, so init and verify agree. Shared state rather
    // than a thread-local: mockito runs these closures on its own thread.
    let b = vec![7u8; 64];
    let a_pub: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let b_pub = SrpServer::<Sha256>::new(&G_2048).compute_public_ephemeral(&b, &v);

    let mut server = mockito::Server::new();

    let a_for_init = Arc::clone(&a_pub);
    let init_body = format!(
        r#"{{"session_id":"11111111-1111-1111-1111-111111111111","srp_salt":"{}","argon2_salt":"{}","server_public":"{}"}}"#,
        salt_to_base64(&srp_salt),
        salt_to_base64(&argon2_salt),
        hex::encode(&b_pub)
    );
    server
        .mock("POST", "/api/v1/auth/srp/init")
        .with_status(200)
        .with_body_from_request(move |req| {
            let body: serde_json::Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let a = hex::decode(body["client_public"].as_str().unwrap()).unwrap();
            *a_for_init.lock().unwrap() = a;
            init_body.clone().into_bytes()
        })
        .create();

    /// M2 when the client's proof checks out, `None` when it does not.
    fn m2_for(b: &[u8], v: &[u8], a: &[u8], req: &mockito::Request) -> Option<Vec<u8>> {
        let body: serde_json::Value = serde_json::from_slice(req.body().unwrap()).unwrap();
        let m1 = hex::decode(body["client_proof"].as_str().unwrap()).unwrap();
        let vf = SrpServer::<Sha256>::new(&G_2048)
            .process_reply(b, v, a)
            .expect("mock server: process_reply");
        vf.verify_client(&m1).ok()?;
        Some(vf.proof().to_vec())
    }

    let (a_status, a_body) = (Arc::clone(&a_pub), Arc::clone(&a_pub));
    let (b_status, b_body) = (b.clone(), b);
    let (v_status, v_body) = (v.clone(), v);
    server
        .mock("POST", "/api/v1/auth/srp/verify")
        .with_status_code_from_request(move |req| {
            let a = a_status.lock().unwrap().clone();
            if m2_for(&b_status, &v_status, &a, req).is_some() {
                200
            } else {
                401
            }
        })
        .with_body_from_request(move |req| {
            let a = a_body.lock().unwrap().clone();
            let Some(m2) = m2_for(&b_body, &v_body, &a, req) else {
                return br#"{"error":"Authentication failed","code":"UNAUTHORIZED"}"#.to_vec();
            };
            format!(
                r#"{{"server_proof":"{}","requires_totp":false,"access_token":"{}","refresh_token":"refresh-xyz"}}"#,
                hex::encode(&m2),
                jwt_with_exp(4_000_000_000)
            )
            .into_bytes()
        })
        .create();
    server
}

fn jwt_with_exp(exp: i64) -> String {
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!(r#"{{"exp":{exp},"scope":"user"}}"#));
    format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
}

// ─── The prompts ─────────────────────────────────────────────────────────────

/// Type `PASSWORD` at the "Master password" prompt of a running command.
///
/// Waits for the terminal to stop echoing first: typing between the prompt and
/// the echo switch is discarded by the terminal driver (`Pty::wait_for_echo_off`).
fn type_master_password(run: &mut Pty) {
    run.wait_for("Master password");
    run.wait_for_echo_off();
    run.type_line(PASSWORD);
    // Argon2 and SRP at opt-level 0 on a slow runner: the prompt is waited for
    // 15 s, what follows the password gets 60 s. A deadlock still times out.
    run.allow(std::time::Duration::from_secs(60));
}

/// `evnx auth login` reads the password typed at its prompt and signs in.
#[test]
fn auth_login_reads_the_typed_master_password() {
    let server = srp_server();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let url = server.url();

    let mut run = Pty::spawn_with(
        p.path(),
        &["auth", "login", "--email", EMAIL],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    type_master_password(&mut run);
    run.wait_for(&format!("signed in as {EMAIL}"));
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(
        cfg.join("credentials.json").exists(),
        "no session stored after the prompt was answered"
    );
}

/// `evnx cloud pull` reads the password typed at its prompt and writes `.env`.
#[test]
fn cloud_pull_reads_the_typed_master_password() {
    let server = pull_server();
    let url = server.url();
    let p = Project::new().file(".gitignore", ".env*\n");
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &url);

    let mut run = Pty::spawn_with(
        p.path(),
        &["cloud", "pull", "--vault", VAULT_ID, "--version", "1"],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    type_master_password(&mut run);
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert_eq!(p.read(".env"), "A=1\n");
}

/// `evnx cloud push` reads the password typed at its prompt and uploads once.
#[test]
fn cloud_push_reads_the_typed_master_password() {
    let (server, push) = push_server();
    let url = server.url();
    let p = Project::new()
        .file(".gitignore", ".env*\n")
        .file(".env", "A=1\n");
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &url);

    let mut run = Pty::spawn_with(
        p.path(),
        &[
            "cloud", "push", "--vault", VAULT_ID, "--env", ".env", "--yes",
        ],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    type_master_password(&mut run);
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    push.assert();
}

// ─── A "no" at a confirmation exits 4 ────────────────────────────────────────

/// A mock with just enough for each `auth` and `cloud delete-version`
/// confirmation to be reached; any change request fails the test.
fn decline_server() -> (mockito::ServerGuard, Vec<mockito::Mock>) {
    let mut s = base_server();
    let changes = ["POST", "DELETE", "PATCH", "PUT"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    (s, changes)
}

/// The reads of [`decline_server`], with no change request allowed or refused.
fn base_server() -> mockito::ServerGuard {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{{"id":"{VAULT_ID}","name":"app","environment":"development",
            "role":"owner","version_count":2,"updated_at":"2026-10-20T09:00:00Z"}}]}}"#
        ))
        .create();
    s.mock("GET", format!("/api/v1/vaults/{VAULT_ID}/versions").as_str())
        .with_status(200)
        .with_body(
            r#"{"versions":[
            {"version_num":2,"key_count":1,"key_names":["A"],"blob_hash":"h2","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-21T09:00:00Z"},
            {"version_num":1,"key_count":1,"key_names":["A"],"blob_hash":"h1","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-20T09:00:00Z"}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/sessions")
        .with_status(200)
        .with_body(
            r#"{"sessions":[
            {"session_id":"aaaaaaaa-1111-2222-3333-444444444444","created_at":"2026-10-20T09:00:00Z","last_used_at":"2026-10-20T09:00:00Z","expires_at":"2026-11-20T09:00:00Z","current":true},
            {"session_id":"bbbbbbbb-1111-2222-3333-444444444444","created_at":"2026-10-20T09:00:00Z","last_used_at":"2026-10-20T09:00:00Z","expires_at":"2026-11-20T09:00:00Z","current":false}]}"#,
        )
        .create();
    s.mock("GET", "/api/v1/auth/tokens")
        .with_status(200)
        .with_body(
            r#"{"tokens":[{"id":"t-1","name":"ci","scope":"read","vault_id":null,
            "expires_at":null,"created_at":"2026-10-20T09:00:00Z","last_used_at":null}]}"#,
        )
        .create();
    s
}

/// Answering "n" to the auth and version-delete confirmations exits 4 and sends
/// nothing. (`vault delete`, `org seats` and the `cloud push` shrink question
/// have the same check in `tests/cloud_defaults.rs`.)
#[test]
fn a_no_at_an_auth_or_version_confirmation_exits_4() {
    for (args, prompt) in [
        (&["auth", "token", "revoke", "ci"][..], "Revoke ci?"),
        (&["auth", "sessions", "revoke-others"][..], "Continue?"),
        (
            &["cloud", "delete-version", "1", "--vault", "app"][..],
            "Delete version 1",
        ),
        (
            &["cloud", "delete-version", "--latest", "--vault", "app"][..],
            "Type app/development to confirm",
        ),
    ] {
        let (server, changes) = decline_server();
        let url = server.url();
        let p = Project::new();
        let cfg = p.path().join("cfg");
        support::cloud::login(&cfg, &url);
        let mut run = Pty::spawn_with(
            p.path(),
            args,
            &[
                ("EVNX_SERVER", url.as_str()),
                ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ],
        );
        run.wait_for(prompt);
        run.type_line("n");
        let (rest, code) = run.finish();
        assert_eq!(code, 4, "{args:?}: {rest}");
        for m in changes {
            m.assert();
        }
    }
}

/// `delete-version --latest` asks for the full vault name; typing it deletes the
/// latest version and nothing else.
#[test]
fn del_latest_typing_the_vault_name_deletes_the_latest() {
    let mut server = base_server();
    let del = server
        .mock(
            "DELETE",
            format!("/api/v1/vaults/{VAULT_ID}/versions/2").as_str(),
        )
        .with_status(200)
        .with_body(r#"{"deleted":2,"latest":1,"was_latest":true}"#)
        .expect(1)
        .create();
    let url = server.url();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &url);
    let mut run = Pty::spawn_with(
        p.path(),
        &["cloud", "delete-version", "--latest", "--vault", "app"],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    let seen = run.wait_for("Type app/development to confirm");
    assert!(
        seen.contains("Version 1") && seen.contains("becomes the latest"),
        "{seen}"
    );
    run.type_line("app/development");
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    del.assert();
}
