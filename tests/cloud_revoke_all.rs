//! `evnx vault revoke --all-vaults --user <email>`: offboarding in one command.
//!
//! One confirmation naming every vault, one master password, then revoke and
//! re-key in each vault in turn with the one-vault code; vaults the caller
//! cannot revoke in are skipped (and named) before the question; the first
//! failure stops the run with a summary. These tests use real keys on a mock
//! server, as `tests/cloud_unlock.rs` does, so the re-key really decrypts and
//! re-encrypts every version.
#![cfg(feature = "cloud")]

mod support;

use evnx_crypto::{
    MasterKey, UserKeypair, VaultKey, b64_encode, derive_master_key, encrypt_private_key,
    encrypt_vault, generate_keypair, generate_salt, salt_to_base64, vault_aad,
    wrap_vault_key_with_master_key,
};
use support::Project;
use support::cloud::signed_in;

const PASSWORD: &str = "correct horse battery staple";
const BOB: &str = "bob@example.com";

/// A value that must never appear in any output: it is inside every version.
const CANARY: &str = "CANARYc4n4ryV4lu3";

/// Vaults: (id, name, environment, your role).
const APP: (&str, &str, &str, &str) = (
    "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a01",
    "app",
    "production",
    "owner",
);
const API: (&str, &str, &str, &str) = (
    "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a02",
    "api",
    "staging",
    "admin",
);
/// You are only a developer here: bob cannot be removed by you.
const WEB: (&str, &str, &str, &str) = (
    "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a03",
    "web",
    "development",
    "developer",
);
/// Bob is not a member here.
const SOLO: (&str, &str, &str, &str) = (
    "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a04",
    "solo",
    "test",
    "owner",
);

struct Account {
    me: String,
    master: MasterKey,
}

fn account() -> Account {
    let salt = generate_salt();
    let master = derive_master_key(PASSWORD.as_bytes(), &salt).unwrap();
    let keypair = generate_keypair();
    let sealed = encrypt_private_key(&keypair, &master).unwrap().to_base64();
    let me = format!(
        r#"{{"user_id":"u-me","email":"me@example.com","argon2_salt":"{}",
        "encrypted_private_key":"{sealed}","totp_enabled":false,"has_mlkem_key":true}}"#,
        salt_to_base64(&salt)
    );
    Account { me, master }
}

fn member(id: &str, email: &str, role: &str, you: bool) -> String {
    format!(
        r#"{{"user_id":"{id}","email":"{email}","role":"{role}",
        "granted_at":"2026-10-20T09:00:00Z","has_mlkem_key":true,"is_you":{you}}}"#
    )
}

fn vault_row(v: (&str, &str, &str, &str), versions: usize) -> String {
    format!(
        r#"{{"id":"{}","name":"{}","environment":"{}","role":"{}","version_count":{versions},
        "updated_at":"2026-10-20T09:00:00Z"}}"#,
        v.0, v.1, v.2, v.3
    )
}

/// The `.env` text of every version: names a checklist can place, values that
/// must stay inside the ciphertext.
fn plaintext() -> String {
    format!(
        "STRIPE_SECRET_KEY=sk_live_{CANARY}\n\
         AWS_SECRET_ACCESS_KEY={CANARY}AbCdEfGhIjKlMnOpQrStUv\n\
         DATABASE_URL=postgres://app:{CANARY}@db.internal/app\n\
         PORT=8080\n"
    )
}

/// What a mock server for these tests holds, and which of its mocks a test
/// checks.
struct Fixture {
    server: mockito::ServerGuard,
    /// The final re-key swap per vault, by vault name.
    swaps: Vec<(&'static str, mockito::Mock)>,
    /// Requests that must not happen: changes in a skipped vault, and the
    /// key and blob reads of a vault that must not be touched.
    never: Vec<mockito::Mock>,
}

struct Options {
    /// `app`'s swap answers 500: the run must stop there.
    fail_app: bool,
    /// Include `web`, where you are a developer.
    with_web: bool,
}

fn fixture(o: Options) -> Fixture {
    let a = account();
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/auth/me")
        .with_status(200)
        .with_body(&a.me)
        .create();

    let mut rows = vec![vault_row(APP, 2), vault_row(API, 1)];
    if o.with_web {
        rows.push(vault_row(WEB, 1));
    }
    rows.push(vault_row(SOLO, 1));
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(r#"{{"vaults":[{}]}}"#, rows.join(",")))
        .create();

    // Members. `carol` and `boss` stay and need public keys for the re-wrap.
    let members = [
        (
            APP,
            vec![
                member("u-me", "me@example.com", "owner", true),
                member("u-bob", BOB, "developer", false),
                member("u-carol", "carol@example.com", "viewer", false),
            ],
        ),
        (
            API,
            vec![
                member("u-boss", "boss@example.com", "owner", false),
                member("u-me", "me@example.com", "admin", true),
                member("u-bob", BOB, "viewer", false),
            ],
        ),
        (
            WEB,
            vec![
                member("u-boss", "boss@example.com", "owner", false),
                member("u-me", "me@example.com", "developer", true),
                member("u-bob", BOB, "viewer", false),
            ],
        ),
        (SOLO, vec![member("u-me", "me@example.com", "owner", true)]),
    ];
    for (v, list) in &members {
        s.mock("GET", format!("/api/v1/vaults/{}/members", v.0).as_str())
            .with_status(200)
            .with_body(format!(r#"{{"members":[{}]}}"#, list.join(",")))
            .create();
    }
    for who in ["carol", "boss"] {
        let kp: UserKeypair = generate_keypair();
        s.mock(
            "GET",
            format!("/api/v1/users/{who}%40example.com/public-key").as_str(),
        )
        .with_status(200)
        .with_body(format!(
            r#"{{"x25519_public_key":"{}","mlkem_public_key":"{}"}}"#,
            kp.x25519_public_base64(),
            kp.mlkem_public_base64()
        ))
        .create();
    }

    let mut swaps = Vec::new();
    let mut never = Vec::new();
    for (v, versions) in [(APP, 2), (API, 1)] {
        let vk = VaultKey::generate();
        let wrapped = wrap_vault_key_with_master_key(&vk, &a.master).unwrap();
        // After a failure in `app`, `api` is not attempted: not even its key.
        let untouched = o.fail_app && v.1 == "api";
        let my_key = s
            .mock("GET", format!("/api/v1/vaults/{}/my-key", v.0).as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"encrypted_vault_key":"{}"}}"#,
                b64_encode(&wrapped)
            ));
        let my_key = if untouched { my_key.expect(0) } else { my_key };
        let my_key = my_key.create();
        if untouched {
            never.push(my_key);
        }
        let rows: Vec<String> = (1..=versions)
            .rev()
            .map(|n| {
                format!(
                    r#"{{"version_num":{n},"key_count":4,"key_names":["STRIPE_SECRET_KEY","AWS_SECRET_ACCESS_KEY","DATABASE_URL","PORT"],
                    "blob_hash":"h{n}","blob_size_bytes":40,"pushed_by":"u-me","pushed_at":"2026-10-20T09:00:00Z"}}"#
                )
            })
            .collect();
        s.mock("GET", format!("/api/v1/vaults/{}/versions", v.0).as_str())
            .with_status(200)
            .with_body(format!(r#"{{"versions":[{}]}}"#, rows.join(",")))
            .create();
        for n in 1..=versions {
            let b = encrypt_vault(plaintext().as_bytes(), &vk, &vault_aad(v.0, n as u32)).unwrap();
            let mut bytes = b.nonce.to_vec();
            bytes.extend_from_slice(&b.ciphertext);
            s.mock(
                "GET",
                format!("/api/v1/vaults/{}/versions/{n}/blob?purpose=rekey", v.0).as_str(),
            )
            .with_status(200)
            .with_body(bytes)
            .create();
        }
        s.mock(
            "POST",
            format!("/api/v1/vaults/{}/rekey/blobs", v.0).as_str(),
        )
        .with_status(201)
        .with_body_from_request(|req| {
            let body: serde_json::Value = serde_json::from_slice(req.body().unwrap()).unwrap();
            let n = body["version_num"].as_i64().unwrap();
            format!(r#"{{"version_num":{n},"blob_key":"staged-{n}","blob_size_bytes":40}}"#)
                .into_bytes()
        })
        .create();
        let swap = s
            .mock("POST", format!("/api/v1/vaults/{}/rekey", v.0).as_str())
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "remove_user_id": "u-bob"
            })));
        let swap = if o.fail_app && v.1 == "app" {
            swap.with_status(500)
                .with_body(r#"{"error":"Internal error","code":"INTERNAL_ERROR"}"#)
                .expect(1)
        } else if untouched {
            swap.expect(0)
        } else {
            swap.with_status(200).with_body("{}").expect(1)
        };
        swaps.push((v.1, swap.create()));
    }

    // Nothing may change in `web` or `solo`, and nothing is deleted anywhere
    // (a re-key removes the member inside its swap).
    for v in [WEB, SOLO] {
        never.push(
            s.mock(
                "POST",
                mockito::Matcher::Regex(format!("^/api/v1/vaults/{}/", v.0)),
            )
            .expect(0)
            .create(),
        );
        never.push(
            s.mock("GET", format!("/api/v1/vaults/{}/my-key", v.0).as_str())
                .expect(0)
                .create(),
        );
    }
    never.push(s.mock("DELETE", mockito::Matcher::Any).expect(0).create());
    Fixture {
        server: s,
        swaps,
        never,
    }
}

fn revoke_all(f: &Fixture, extra: &[&str], stdin: Option<&[u8]>) -> support::Output {
    let mut args = vec!["vault", "revoke", "--all-vaults", "--user", BOB];
    args.extend_from_slice(extra);
    signed_in(&Project::new(), &f.server.url(), &args, stdin)
}

fn password() -> Vec<u8> {
    format!("{PASSWORD}\n").into_bytes()
}

fn assert_no_value(out: &support::Output) {
    for (stream, text) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        assert!(!text.contains(CANARY), "a value reached {stream}:\n{text}");
        assert!(
            !text.contains("sk_live_"),
            "a value reached {stream}:\n{text}"
        );
    }
}

/// Two vaults revoked and re-keyed after one password; the vault where you
/// are only a developer is skipped, named with the reason before anything is
/// asked, and the exit is 1 because bob keeps access there.
#[test]
fn n14_1_two_vaults_revoked_one_skipped_for_role() {
    let f = fixture(Options {
        fail_app: false,
        with_web: true,
    });
    let out = revoke_all(&f, &["--yes", "--password-stdin"], Some(&password()));
    out.assert_code(1);
    let o = &out.stdout;
    assert!(
        o.contains("bob@example.com is a member of 3 vault(s)"),
        "{o}"
    );
    assert!(
        o.contains("web/development  you are a developer; removing a member needs admin or owner"),
        "{o}"
    );
    assert!(
        o.contains("removed bob@example.com from app/production and rotated the vault key (2 version(s) re-encrypted)"),
        "{o}"
    );
    assert!(
        o.contains("removed bob@example.com from api/staging"),
        "{o}"
    );
    assert!(
        o.contains("revoked        app/production, api/staging"),
        "{o}"
    );
    assert!(
        o.contains("skipped        web/development  (they keep access there)"),
        "{o}"
    );
    assert!(!o.contains("solo/test"), "{o}");
    // The skip is listed before the work starts.
    assert!(
        o.find("Skipped").unwrap() < o.find("Re-encrypting").unwrap(),
        "{o}"
    );
    assert_no_value(&out);
    for (_, m) in &f.swaps {
        m.assert();
    }
    for m in &f.never {
        m.assert();
    }
}

/// Every vault revoked: exit 0.
#[test]
fn n14_1_every_vault_revoked_exits_0() {
    let f = fixture(Options {
        fail_app: false,
        with_web: false,
    });
    let out = revoke_all(&f, &["--yes", "--password-stdin"], Some(&password()));
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("revoked        app/production, api/staging"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("skipped"), "{}", out.stdout);
    assert_no_value(&out);
}

/// A failure in the first vault stops the run: the second is not attempted
/// (not even its key is read), the summary says so, and the exit code is the
/// failure's (15, server error).
#[test]
fn n14_1_a_failure_stops_the_run_and_reports() {
    let f = fixture(Options {
        fail_app: true,
        with_web: false,
    });
    let out = revoke_all(&f, &["--yes", "--password-stdin"], Some(&password()));
    out.assert_code(15);
    let o = &out.stdout;
    assert!(o.contains("failed         app/production"), "{o}");
    assert!(o.contains("not attempted  api/staging"), "{o}");
    assert!(!o.contains("revoked        "), "{o}");
    assert!(out.stderr.contains("internal error"), "{}", out.stderr);
    assert_no_value(&out);
    for (_, m) in &f.swaps {
        m.assert();
    }
    for m in &f.never {
        m.assert();
    }
}

/// `--dry-run` lists what would happen and sends nothing that changes state:
/// no POST, no DELETE, no key read, no password.
#[test]
fn n14_1_dry_run_sends_no_change() {
    let mut s = mockito::Server::new();
    s.mock("GET", "/api/v1/vaults")
        .with_status(200)
        .with_body(format!(
            r#"{{"vaults":[{},{}]}}"#,
            vault_row(APP, 2),
            vault_row(WEB, 1)
        ))
        .create();
    for (v, me_role, boss_role) in [(APP, "owner", "admin"), (WEB, "developer", "owner")] {
        s.mock("GET", format!("/api/v1/vaults/{}/members", v.0).as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"members":[{},{},{}]}}"#,
                member("u-boss", "boss@example.com", boss_role, false),
                member("u-me", "me@example.com", me_role, true),
                member("u-bob", BOB, "viewer", false)
            ))
            .create();
    }
    let mut never: Vec<mockito::Mock> = ["POST", "DELETE", "PATCH", "PUT"]
        .iter()
        .map(|m| s.mock(m, mockito::Matcher::Any).expect(0).create())
        .collect();
    never.push(
        s.mock("GET", mockito::Matcher::Regex("/my-key$".into()))
            .expect(0)
            .create(),
    );
    never.push(s.mock("GET", "/api/v1/auth/me").expect(0).create());
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "revoke",
            "--all-vaults",
            "--user",
            BOB,
            "--dry-run",
        ],
        None,
    );
    out.assert_code(1);
    assert!(
        out.stdout
            .contains("app/production  (viewer, 2 version(s), 2 member(s) stay)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Dry run: nothing was changed."),
        "{}",
        out.stdout
    );
    for m in never {
        m.assert();
    }

    // With --json: the plan as data.
    let out = signed_in(
        &Project::new(),
        &s.url(),
        &[
            "vault",
            "revoke",
            "--all-vaults",
            "--user",
            BOB,
            "-n",
            "--json",
        ],
        None,
    );
    out.assert_code(1);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["dry_run"], true);
    assert_eq!(doc["vaults"][0]["result"], "would_revoke");
    assert_eq!(doc["vaults"][1]["result"], "skipped");
}

/// No terminal and no `--yes`: exit 4 naming `--yes`, before the password.
#[test]
fn n14_1_no_terminal_without_yes_exits_4() {
    let f = fixture(Options {
        fail_app: false,
        with_web: true,
    });
    let out = revoke_all(&f, &[], None);
    out.assert_code(4);
    assert!(
        out.stderr
            .contains("evnx vault revoke --all-vaults --user bob@example.com --yes"),
        "{}",
        out.stderr
    );
    // No swap may have been sent: the mocks expect one, so check by hand.
    for (name, m) in f.swaps {
        assert!(
            !m.matched(),
            "{name}: a re-key was sent without confirmation"
        );
    }
}

/// `--json`: names only, the `run` header first, one entry per vault.
#[test]
fn n14_1_json_shape() {
    let f = fixture(Options {
        fail_app: false,
        with_web: true,
    });
    let out = revoke_all(
        &f,
        &["--yes", "--password-stdin", "--json"],
        Some(&password()),
    );
    out.assert_code(1);
    assert!(
        out.stdout.trim_start().starts_with(
            r#"{
  "run": {"#
        ),
        "{}",
        out.stdout
    );
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["user"], BOB);
    assert_eq!(doc["dry_run"], false);
    assert_eq!(doc["rekey"], true);
    let results: Vec<(String, String, String)> = doc["vaults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["vault"].as_str().unwrap().to_string(),
                v["environment"].as_str().unwrap().to_string(),
                v["result"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            ("app".into(), "production".into(), "revoked".into()),
            ("api".into(), "staging".into(), "revoked".into()),
            ("web".into(), "development".into(), "skipped".into()),
        ]
    );
    assert_eq!(doc["vaults"][0]["versions_reencrypted"], 2);
    assert!(
        doc["vaults"][2]["reason"]
            .as_str()
            .unwrap()
            .contains("developer")
    );
    assert_no_value(&out);
}

/// Your own address is refused for the whole command, before any change.
#[test]
fn n14_1_revoking_yourself_everywhere_is_refused() {
    let f = fixture(Options {
        fail_app: false,
        with_web: false,
    });
    let out = signed_in(
        &Project::new(),
        &f.server.url(),
        &[
            "vault",
            "revoke",
            "--all-vaults",
            "--user",
            "me@example.com",
            "-y",
        ],
        None,
    );
    out.assert_code(2);
    assert!(
        out.stderr.contains("evnx vault leave <vault>"),
        "{}",
        out.stderr
    );
    for (_, m) in f.swaps {
        assert!(!m.matched());
    }
}

/// A member of no vault you can see: said so, exit 0, nothing sent.
#[test]
fn n14_1_not_a_member_anywhere_is_nothing_to_do() {
    let f = fixture(Options {
        fail_app: false,
        with_web: false,
    });
    let out = signed_in(
        &Project::new(),
        &f.server.url(),
        &[
            "vault",
            "revoke",
            "--all-vaults",
            "--user",
            "nobody@example.com",
            "-y",
        ],
        None,
    );
    out.assert_code(0);
    assert!(
        out.stdout
            .contains("nobody@example.com is not a member of any vault you can see"),
        "{}",
        out.stdout
    );
}

/// A vault and --all-vaults together, or neither, is a usage error.
#[test]
fn n14_1_a_vault_or_all_vaults_exactly_one() {
    let p = Project::new();
    for args in [
        &["vault", "revoke", "app", "--all-vaults", "--user", BOB][..],
        &["vault", "revoke", "--user", BOB][..],
    ] {
        let out = signed_in(&p, "http://127.0.0.1:9", args, None);
        out.assert_code(2);
    }
}

/// Answering "n" at the one question exits 4 and changes nothing.
#[cfg(unix)]
#[test]
fn n14_1_a_no_at_the_question_exits_4() {
    use support::pty::Pty;
    let f = fixture(Options {
        fail_app: false,
        with_web: true,
    });
    let url = f.server.url();
    let p = Project::new();
    let cfg = p.path().join("cfg");
    support::cloud::login(&cfg, &url);
    let mut run = Pty::spawn_with(
        p.path(),
        &["vault", "revoke", "--all-vaults", "--user", BOB],
        &[
            ("EVNX_SERVER", url.as_str()),
            ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
        ],
    );
    run.wait_for("Remove bob@example.com from these 2 vault(s)?");
    run.type_line("n");
    let (rest, code) = run.finish();
    assert_eq!(code, 4, "{rest}");
    for (_, m) in f.swaps {
        assert!(!m.matched());
    }
}

/// One vault takes `--dry-run` and `--json` too: the plan, and no change.
#[test]
fn n14_1_one_vault_dry_run_json_sends_no_change() {
    let mut f = fixture(Options {
        fail_app: false,
        with_web: false,
    });
    let posts = f
        .server
        .mock("POST", mockito::Matcher::Any)
        .expect(0)
        .create();
    let out = signed_in(
        &Project::new(),
        &f.server.url(),
        &[
            "vault",
            "revoke",
            "app",
            "--user",
            BOB,
            "--dry-run",
            "--json",
        ],
        None,
    );
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["vaults"][0]["vault"], "app");
    assert_eq!(doc["vaults"][0]["result"], "would_revoke");
    posts.assert();
    for (_, m) in f.swaps {
        assert!(!m.matched());
    }
}
