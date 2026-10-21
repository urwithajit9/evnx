//! What `evnx scan` reports, and how it words it (R-scan, 0.10.14).
//!
//! * I-050, N-20 — a format rule matching a stand-in value: a run of one filler
//!   character (`sk_live_xxxx…`), a documented example key, or the
//!   `user:password@localhost` URL evnx itself writes into new projects. None
//!   can be a working credential, so none is a finding.
//! * I-287 — a name-only finding whose value is plainly an address.
//! * I-051 — a private key's preview names the key type.
//! * I-052 — a connection-string finding names the scheme instead of the
//!   generic "live key format" line.
//! * I-288 — `--help` says what `--severity high` keeps.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

/// (exit code, stdout) for `evnx scan <args>` in a folder holding `files`.
fn scan_in(files: &[(&str, &str)], args: &[&str]) -> (i32, String) {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        fs::write(d.path().join(name), body).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .arg("scan")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn patterns_found(files: &[(&str, &str)]) -> Vec<String> {
    let (_, stdout) = scan_in(files, &["--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["pattern"].as_str().unwrap().to_string())
        .collect()
}

// ── I-050 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i050_a_run_of_filler_after_a_provider_prefix_is_not_a_finding() {
    for v in [
        "sk_live_xxxxxxxxxxxxxxxxxxxxxxxx",
        "ghp_000000000000000000000000000000000000",
        "AKIA****************",
    ] {
        let (code, stdout) = scan_in(&[("a.env", &format!("KEY={v}\n"))], &["a.env"]);
        assert_eq!(code, 0, "{v}\n{stdout}");
    }
}

/// The documented AWS example key is a stand-in by definition.
#[test]
fn t_i050_the_documented_aws_example_key_is_not_a_finding() {
    let (code, stdout) = scan_in(
        &[("a.env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n")],
        &["a.env"],
    );
    assert_eq!(code, 0, "{stdout}");
}

/// The half that keeps I-050 safe: real-shaped keys are still reported.
#[test]
fn t_i050_a_real_shaped_key_is_still_a_finding() {
    for v in [
        "sk_live_51H1234567890abcdefghijk",
        "AKIA4OZRMFJ3VREALKEY",
        "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
    ] {
        let (code, stdout) = scan_in(&[("a.env", &format!("KEY={v}\n"))], &["a.env"]);
        assert_eq!(code, 1, "{v}\n{stdout}");
        assert!(stdout.contains("not a placeholder"), "{v}\n{stdout}");
    }
}

// ── N-20 ────────────────────────────────────────────────────────────────────

#[test]
fn t_n20_evnx_s_own_database_url_placeholders_are_not_findings() {
    for url in [
        "postgresql://user:password@localhost:5432/dbname",
        "postgres://user:password@localhost:5432/dbname",
        "mysql://user:password@localhost:3306/dbname",
        "mongodb://user:password@localhost:27017/dbname",
        "mongodb+srv://user:password@cluster.example.net/dbname",
        "amqp://user:password@localhost:5672/",
    ] {
        let found = patterns_found(&[(".env", &format!("DATABASE_URL={url}\n"))]);
        assert!(found.is_empty(), "{url}: {found:?}");
    }
}

/// A real password, or the placeholder words on a real host, is still one.
#[test]
fn t_n20_a_real_connection_string_is_still_a_finding() {
    for url in [
        "postgresql://admin:s3cr3tPassw0rd@db.internal:5432/app",
        "postgres://user:password@db.prod.company.io:5432/app",
        "postgres://app:password1@localhost:5432/app",
    ] {
        let found = patterns_found(&[(".env", &format!("DATABASE_URL={url}\n"))]);
        assert_eq!(
            found,
            vec!["Credentials in a connection string".to_string()],
            "{url}"
        );
    }
}

/// One per blueprint that writes the URL (I-250): `init` then `scan` is clean.
fn t_n20_blueprint(blueprint: &str) {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .args(["init", "--yes", "--blueprint", blueprint, "--path"])
        .arg(d.path())
        .assert()
        .success();
    let env = fs::read_to_string(d.path().join(".env")).unwrap();
    assert!(env.contains("user:password@localhost"), "{env}");
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "."])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{blueprint}:\n{stdout}");
}

#[test]
fn t_n20_fastapi_ai_then_scan_is_clean() {
    t_n20_blueprint("fastapi_ai");
}

#[test]
fn t_n20_rust_high_perf_then_scan_is_clean() {
    t_n20_blueprint("rust_high_perf");
}

#[test]
fn t_n20_django_enterprise_then_scan_is_clean() {
    t_n20_blueprint("django_enterprise");
}

// ── I-287 end to end ────────────────────────────────────────────────────────

#[test]
fn t_i287_a_next_js_env_reports_its_secrets_and_not_its_urls() {
    let found = patterns_found(&[(
        ".env",
        "NEXTAUTH_URL=http://localhost:3000\n\
         NEXTAUTH_SECRET=639b0e2a4c6d8f1a3b5c7e9d0f2a4b6c8e0d2f4a\n\
         AUTH0_DOMAIN=myapp.us.auth0.com\n",
    )]);
    assert_eq!(
        found,
        vec!["Sensitive config key: NEXTAUTH_SECRET".to_string()],
        "{found:?}"
    );
}

// ── I-051, I-052 ────────────────────────────────────────────────────────────

#[test]
fn t_i051_a_private_key_preview_names_the_key_type() {
    let (code, stdout) = scan_in(
        &[(
            "id.pem",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\n-----END RSA PRIVATE KEY-----\n",
        )],
        &["id.pem"],
    );
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("-----BEGIN RSA PRIVATE KEY-----"),
        "{stdout}"
    );
    assert!(!stdout.contains("----…----"), "{stdout}");
    // The key body never appears.
    assert!(!stdout.contains("MIIBOg"), "{stdout}");
}

#[test]
fn t_i052_a_connection_string_finding_names_its_scheme() {
    let (_, stdout) = scan_in(
        &[(
            ".env",
            "DATABASE_URL=postgres://admin:s3cr3tPassw0rd@db.internal:5432/app\n",
        )],
        &[".env"],
    );
    assert!(
        stdout.contains("a password in a postgres:// URL"),
        "{stdout}"
    );
    assert!(!stdout.contains("matches a live key format"), "{stdout}");
}

// ── I-288 ───────────────────────────────────────────────────────────────────

/// `--help` said `--severity high` keeps only format matches; a long value
/// under a secret-like name is high too, and stays high so a committed
/// `JWT_SECRET` still fails a `--severity high` gate.
#[test]
fn t_i288_severity_help_says_what_high_keeps() {
    let out = cargo_bin_cmd!("evnx")
        .args(["scan", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        !help.contains("reports only detections that matched a format"),
        "{help}"
    );
    assert!(help.contains("named like a secret"), "{help}");

    let (code, stdout) = scan_in(
        &[(".env", "JWT_SECRET=6e5a1c9f0b2d4e8a7c3f5d1b9e0a2c4d\n")],
        &["--severity", "high"],
    );
    assert_eq!(code, 1, "{stdout}");
}
