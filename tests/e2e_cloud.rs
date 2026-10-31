//! The cloud commands against a real evnx-server (RM-44), not mockito.
//!
//! Skipped unless `EVNX_E2E_SERVER` names a running server, and
//! `EVNX_E2E_CONTAINER` the Docker container it runs in: verification links go
//! to its log (`EMAIL_TRANSPORT=log`), and the test reads them from there.
//! `.github/workflows/cloud-e2e.yml` starts one nightly, at the commit
//! production runs; locally:
//!
//! ```text
//! EVNX_E2E_SERVER=http://127.0.0.1:18080 EVNX_E2E_CONTAINER=e2e-api \
//!     cargo test --all-features --test e2e_cloud -- --test-threads=1
//! ```
//!
//! ⛔ Never point it at api.evnx.dev: it registers accounts.
#![cfg(all(unix, feature = "cloud"))]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A person with their own home, config and password.
struct Person {
    email: String,
    home: PathBuf,
    password: String,
}

struct Stack {
    server: String,
    container: String,
    _root: tempfile::TempDir,
}

impl Stack {
    fn from_env() -> Option<Stack> {
        let server = std::env::var("EVNX_E2E_SERVER").ok()?;
        assert!(
            !server.contains("api.evnx.dev"),
            "EVNX_E2E_SERVER must be a local or test server, never production"
        );
        Some(Stack {
            server,
            container: std::env::var("EVNX_E2E_CONTAINER")
                .expect("EVNX_E2E_CONTAINER: the server's Docker container"),
            _root: tempfile::TempDir::new().unwrap(),
        })
    }

    fn person(&self, name: &str, run: &str) -> Person {
        let home = self._root.path().join(name);
        std::fs::create_dir_all(home.join("work")).unwrap();
        Person {
            email: format!("{name}-{run}@e2e.test"),
            home,
            password: format!("e2e only {name} passphrase {run}"),
        }
    }

    /// `evnx <args>` as `p`, in their work folder, with their password on
    /// stdin when `password` is set.
    fn evnx(&self, p: &Person, password: bool, args: &[&str]) -> Output {
        let mut cmd = Command::new(support::evnx_bin());
        cmd.args(args)
            .current_dir(p.home.join("work"))
            .env("HOME", &p.home)
            .env("XDG_CONFIG_HOME", p.home.join(".config"))
            .env("EVNX_CONFIG_DIR", p.home.join("evnx"))
            .env("EVNX_SERVER", &self.server)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env_remove("EVNX_TOKEN")
            .env_remove("EVNX_MASTER_PASSWORD")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("run evnx");
        {
            use std::io::Write;
            let mut stdin = child.stdin.take().unwrap();
            if password {
                writeln!(stdin, "{}", p.password).unwrap();
            }
        }
        child.wait_with_output().unwrap()
    }

    /// Click every verification link the server has logged.
    fn verify_all(&self) {
        let logs = Command::new("docker")
            .args(["logs", &self.container])
            .output()
            .expect("docker logs");
        let text = String::from_utf8_lossy(&logs.stdout).into_owned()
            + &String::from_utf8_lossy(&logs.stderr);
        let tokens: std::collections::BTreeSet<&str> = text
            .split("verify-email/?token=")
            .skip(1)
            .filter_map(|rest| {
                rest.split(|c: char| !(c.is_ascii_alphanumeric() || "_%.-".contains(c)))
                    .next()
            })
            .collect();
        assert!(!tokens.is_empty(), "no verification link in the server log");
        for t in tokens {
            let ok = Command::new("curl")
                .args(["-sf", "-o", "/dev/null"])
                .arg(format!(
                    "{}/api/v1/auth/verify-email?token={t}",
                    self.server
                ))
                .status()
                .expect("curl");
            // A link from an earlier run may be spent already; only ours matter.
            let _ = ok;
        }
    }
}

impl Stack {
    /// `p`'s access token, from the session the CLI saved at login.
    fn token(&self, p: &Person) -> String {
        let store =
            evnx::cloud::creds::Store::load_from(&p.home.join("evnx").join("credentials.json"))
                .expect("credentials saved at login");
        store
            .session(&self.server)
            .expect("a session for this server")
            .access_token
            .expose()
            .to_string()
    }

    /// `curl` as `p`: (status, body).
    fn api(&self, p: &Person, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
        let mut cmd = Command::new("curl");
        cmd.args(["-s", "-w", "\n%{http_code}", "-X", method])
            .arg("-H")
            .arg(format!("Authorization: Bearer {}", self.token(p)))
            .arg(format!("{}/api/v1{path}", self.server));
        if let Some(b) = body {
            cmd.args(["-H", "Content-Type: application/json", "--data", b]);
        }
        let out = String::from_utf8(cmd.output().expect("curl").stdout).unwrap();
        let (body, code) = out.rsplit_once('\n').unwrap();
        (code.trim().parse().unwrap(), body.to_string())
    }
}

fn ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed ({:?}):\n{}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

fn write(p: &Person, name: &str, body: &str) {
    std::fs::write(Path::new(&p.home).join("work").join(name), body).unwrap();
}

/// register → verify → login → vault create → push → pull (byte-identical) →
/// share → share again (I-313: 409, nothing changed) → member pulls → revoke →
/// the member is refused.
#[test]
fn rm44_a_vault_round_trip_on_a_real_server() {
    let Some(s) = Stack::from_env() else {
        eprintln!("skipped: EVNX_E2E_SERVER is not set");
        return;
    };
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string();
    let owner = s.person("owner", &run);
    let member = s.person("member", &run);

    for p in [&owner, &member] {
        ok(
            &s.evnx(
                p,
                true,
                &["auth", "register", "--email", &p.email, "--password-stdin"],
            ),
            "register",
        );
    }
    s.verify_all();
    for p in [&owner, &member] {
        ok(
            &s.evnx(
                p,
                true,
                &["auth", "login", "--email", &p.email, "--password-stdin"],
            ),
            "login",
        );
    }

    let vault = format!("e2e-{run}");
    let target = format!("{vault}/production");
    let body = "# e2e fixture, not a secret\nDATABASE_URL=postgres://app:e2e-only@db:5432/app\nAPI_KEY=e2e-only-000\n";
    write(&owner, ".env", body);
    ok(
        &s.evnx(
            &owner,
            true,
            &[
                "vault",
                "create",
                &vault,
                "--env",
                "production",
                "--password-stdin",
            ],
        ),
        "vault create",
    );
    ok(
        &s.evnx(
            &owner,
            true,
            &[
                "cloud",
                "push",
                "--vault",
                &target,
                "--file",
                ".env",
                "--password-stdin",
            ],
        ),
        "push",
    );
    let pulled = s.evnx(
        &owner,
        true,
        &[
            "cloud",
            "pull",
            "--vault",
            &target,
            "--file",
            "-",
            "--password-stdin",
        ],
    );
    ok(&pulled, "pull");
    assert_eq!(String::from_utf8_lossy(&pulled.stdout), body, "round trip");

    let share = [
        "vault",
        "share",
        &target,
        "--with",
        &member.email,
        "--role",
        "viewer",
        "--password-stdin",
    ];
    ok(&s.evnx(&owner, true, &share), "share");

    // The CLI refuses a second share before asking the server…
    let again = [
        "vault",
        "share",
        &target,
        "--with",
        &member.email,
        "--role",
        "admin",
        "--password-stdin",
    ];
    let out = s.evnx(&owner, true, &again);
    assert!(
        !out.status.success(),
        "a second share must be refused:\n{}",
        text(&out)
    );

    // …and the server refuses it on its own (I-313, SV-3), for any client that
    // does not check. It was an upsert that rewrote the member's role and key.
    let (_, list) = s.api(&owner, "GET", "/vaults", None);
    let vaults: serde_json::Value = serde_json::from_str(&list).expect("vault list JSON");
    let id = vaults
        .as_array()
        .or_else(|| vaults["vaults"].as_array())
        .expect("a list of vaults")
        .iter()
        .find(|v| v["name"] == vault.as_str())
        .and_then(|v| v["id"].as_str())
        .expect("the vault's id")
        .to_string();
    let junk = "A".repeat(44);
    let (status, reply) = s.api(
        &owner,
        "POST",
        &format!("/vaults/{id}/members"),
        Some(&format!(
            r#"{{"user_email":"{}","role":"admin","encrypted_vault_key":"{junk}","eph_pub_key":"{junk}","mlkem_ciphertext":"{junk}"}}"#,
            member.email
        )),
    );
    assert_eq!(
        status, 409,
        "a share to a member must be a conflict: {reply}"
    );
    assert!(reply.contains("already a member"), "{reply}");
    let members = s.evnx(&owner, false, &["vault", "members", &target]);
    ok(&members, "members");
    let listed = text(&members);
    let row = listed
        .lines()
        .find(|l| l.contains(&member.email))
        .unwrap_or_else(|| panic!("member not listed:\n{listed}"));
    assert!(row.contains("viewer"), "the role changed: {row}");

    let theirs = s.evnx(
        &member,
        true,
        &[
            "cloud",
            "pull",
            "--vault",
            &target,
            "--file",
            "-",
            "--password-stdin",
        ],
    );
    ok(&theirs, "member pull");
    assert_eq!(String::from_utf8_lossy(&theirs.stdout), body);

    ok(
        &s.evnx(
            &owner,
            true,
            &[
                "vault",
                "revoke",
                &target,
                "--user",
                &member.email,
                "--yes",
                "--password-stdin",
            ],
        ),
        "revoke",
    );
    let refused = s.evnx(
        &member,
        true,
        &[
            "cloud",
            "pull",
            "--vault",
            &target,
            "--file",
            "-",
            "--password-stdin",
        ],
    );
    assert!(!refused.status.success(), "a revoked member still pulled");
    assert!(refused.stdout.is_empty(), "a revoked member got bytes");
}
