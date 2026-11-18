//! `evnx cloud run --min-version N` / `EVNX_MIN_VERSION` (WP-11d, matrix row
//! M1): a vault below N is refused before the blob download, the password
//! prompt and any decryption; the child never runs; exit 2.
#![cfg(all(unix, feature = "cloud"))]

mod support;

use evnx_crypto::{
    VaultKey, b64_encode, derive_master_key, encrypt_private_key, encrypt_vault, generate_keypair,
    generate_salt, salt_to_base64, vault_aad, wrap_vault_key_with_master_key,
};
use support::cloud::signed_in;
use support::{Project, run_with};

const PW: &str = "correct horse battery staple";
const A_ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";
const B_ID: &str = "7c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f";
const SECRET: &str = "zq-minver-secret-value-55";

struct Server {
    s: mockito::ServerGuard,
    /// The unlock request (`/auth/me`): it needs the password.
    unlock: mockito::Mock,
    blobs: Vec<mockito::Mock>,
}

impl Server {
    fn url(&self) -> String {
        self.s.url()
    }
    /// Nothing was downloaded and nothing was unlocked.
    fn assert_untouched(&self) {
        self.unlock.assert();
        for b in &self.blobs {
            b.assert();
        }
    }
}

/// Two vaults, `a/dev` at `va` and `b/dev` at `vb`, each holding `A=1` plus a
/// secret. When `expect_use` is false the blob and unlock routes must see no
/// request at all.
fn server(va: i32, vb: i32, expect_use: bool) -> Server {
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
    let unlock = s
        .mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(me)
        .expect(if expect_use { 1 } else { 0 })
        .create();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[
            {{"id":"{A_ID}","name":"a","environment":"dev","role":"owner",
              "version_count":{va},"updated_at":"2026-01-01T00:00:00Z"}},
            {{"id":"{B_ID}","name":"b","environment":"dev","role":"owner",
              "version_count":{vb},"updated_at":"2026-01-01T00:00:00Z"}}]}}"#
        ))
        .create();
    let mut blobs = Vec::new();
    for (id, v, key) in [(A_ID, va, "A"), (B_ID, vb, "B")] {
        let vk = VaultKey::generate();
        let text = format!("{key}_SECRET={SECRET}\nV{key}={v}\n");
        let b = encrypt_vault(text.as_bytes(), &vk, &vault_aad(id, v as u32)).unwrap();
        let mut bytes = b.nonce.to_vec();
        bytes.extend_from_slice(&b.ciphertext);
        let wrapped = wrap_vault_key_with_master_key(&vk, &master).unwrap();
        s.mock(
            "GET",
            format!("/api/v1/vaults/{id}/versions/latest").as_str(),
        )
        .with_status(200)
        .with_body(format!(r#"{{"version_num":{v}}}"#))
        .create();
        blobs.push(
            s.mock(
                "GET",
                format!("/api/v1/vaults/{id}/versions/{v}/blob").as_str(),
            )
            .with_status(200)
            .with_body(bytes)
            .expect(if expect_use { 1 } else { 0 })
            .create(),
        );
        s.mock("GET", format!("/api/v1/vaults/{id}/my-key").as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"encrypted_vault_key":"{}"}}"#,
                b64_encode(&wrapped)
            ))
            .create();
    }
    Server { s, unlock, blobs }
}

/// `cloud run` with extra flags in front of `--`, a marker-touching child.
fn go(s: &Server, p: &Project, flags: &[&str]) -> support::Output {
    go_env(s, p, flags, &[])
}

fn go_env(s: &Server, p: &Project, flags: &[&str], env: &[(&str, &str)]) -> support::Output {
    let marker = p.path().join("ran");
    let script = format!("touch {}; echo ran", marker.display());
    let mut args = vec!["cloud", "run", "--password-stdin"];
    args.extend_from_slice(flags);
    args.extend_from_slice(&["--", "sh", "-c", &script]);
    let stdin = format!("{PW}\n");
    if env.is_empty() {
        signed_in(p, &s.url(), &args, Some(stdin.as_bytes()))
    } else {
        let cfg = p.path().join("cfg");
        support::cloud::login(&cfg, &s.url());
        let mut all = vec![
            ("EVNX_SERVER", s.url()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap().to_string()),
        ];
        all.extend(env.iter().map(|(k, v)| (*k, v.to_string())));
        let refs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
        run_with(p.path(), &args, &refs, Some(stdin.as_bytes()))
    }
}

fn ran(p: &Project) -> bool {
    p.path().join("ran").exists()
}

#[test]
fn m1_stale_vault_exits_2_and_child_never_runs() {
    let s = server(3, 3, false);
    let p = Project::new();
    let out = go(&s, &p, &["--vault", "a", "--min-version", "5"]);
    out.assert_code(2);
    assert!(!ran(&p), "the child ran");
    assert!(
        out.stderr.contains(
            "a/dev is at version 3; --min-version 5 requires at least 5. Nothing was run."
        ),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("stopped before"), "{}", out.stderr);
    s.assert_untouched();
    assert!(!out.all().contains(SECRET), "{}", out.all());
}

#[test]
fn m1_equal_version_runs() {
    let s = server(5, 5, true);
    let p = Project::new();
    let out = go(&s, &p, &["--vault", "a", "--min-version", "5"]);
    out.assert_code(0);
    assert!(ran(&p));
}

#[test]
fn m1_higher_version_runs() {
    let s = server(7, 7, true);
    let p = Project::new();
    go(&s, &p, &["--vault", "a", "--min-version", "5"]).assert_code(0);
    assert!(ran(&p));
}

#[test]
fn m1_version_flag_below_min_refused_without_any_request() {
    // No mock is registered for any route: a request would be a 501.
    let s = mockito::Server::new();
    let p = Project::new();
    let marker = p.path().join("ran");
    let script = format!("touch {}", marker.display());
    let out = signed_in(
        &p,
        &s.url(),
        &[
            "cloud",
            "run",
            "--vault",
            "a",
            "--version",
            "2",
            "--min-version",
            "5",
            "--password-stdin",
            "--",
            "sh",
            "-c",
            &script,
        ],
        Some(format!("{PW}\n").as_bytes()),
    );
    out.assert_code(2);
    assert!(!marker.exists());
    assert!(
        out.stderr
            .contains("--version 2 is below --min-version 5. Nothing was run."),
        "{}",
        out.stderr
    );
}

#[test]
fn m1_version_flag_at_or_above_min_is_allowed() {
    let s = server(5, 5, true);
    let p = Project::new();
    go(
        &s,
        &p,
        &["--vault", "a", "--version", "5", "--min-version", "5"],
    )
    .assert_code(0);
    assert!(ran(&p));
}

#[test]
fn m1_env_var_equivalent() {
    let s = server(3, 3, false);
    let p = Project::new();
    let out = go_env(&s, &p, &["--vault", "a"], &[("EVNX_MIN_VERSION", "5")]);
    out.assert_code(2);
    assert!(!ran(&p));
    assert!(out.stderr.contains("--min-version 5"), "{}", out.stderr);
    s.assert_untouched();
}

#[test]
fn m1_flag_beats_env() {
    // The env var says 9 (would refuse version 3); the flag says 2.
    let s = server(3, 3, true);
    let p = Project::new();
    go_env(
        &s,
        &p,
        &["--vault", "a", "--min-version", "2"],
        &[("EVNX_MIN_VERSION", "9")],
    )
    .assert_code(0);
    assert!(ran(&p));
}

#[test]
fn m1_multi_vault_any_below_refuses_and_names_all_below() {
    let s = server(3, 4, false);
    let p = Project::new();
    let out = go(
        &s,
        &p,
        &["--vault", "a", "--vault", "b", "--min-version", "5"],
    );
    out.assert_code(2);
    assert!(!ran(&p));
    assert!(
        out.stderr.contains("a/dev is at version 3"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("b/dev is at version 4"),
        "{}",
        out.stderr
    );
    s.assert_untouched();
}

#[test]
fn m1_multi_vault_one_below_names_only_that_one() {
    let s = server(3, 6, false);
    let p = Project::new();
    let out = go(
        &s,
        &p,
        &["--vault", "a", "--vault", "b", "--min-version", "5"],
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("a/dev is at version 3"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("b/dev"), "{}", out.stderr);
    s.assert_untouched();
}

#[test]
fn m1_multi_vault_all_ok_runs() {
    let s = server(5, 6, true);
    let p = Project::new();
    go(
        &s,
        &p,
        &["--vault", "a", "--vault", "b", "--min-version", "5"],
    )
    .assert_code(0);
    assert!(ran(&p));
}

#[test]
fn m1_zero_min_is_a_usage_error() {
    let s = server(3, 3, false);
    let p = Project::new();
    go(&s, &p, &["--vault", "a", "--min-version", "0"]).assert_code(2);
    assert!(!ran(&p));
    go_env(&s, &p, &["--vault", "a"], &[("EVNX_MIN_VERSION", "0")]).assert_code(2);
    assert!(!ran(&p));
}

#[test]
fn m1_evnx_min_version_is_not_in_the_child_environment() {
    let s = server(5, 5, true);
    let p = Project::new();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &s.url());
    let url = s.url();
    let out = run_with(
        p.path(),
        &[
            "cloud",
            "run",
            "--vault",
            "a",
            "--password-stdin",
            "--no-mask",
            "--",
            "sh",
            "-c",
            "env | grep -c '^EVNX_MIN_VERSION=' || true",
        ],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
            ("EVNX_MIN_VERSION", "5"),
        ],
        Some(format!("{PW}\n").as_bytes()),
    );
    out.assert_code(0);
    assert_eq!(out.stdout.trim(), "0", "{}", out.all());
}
