//! RM-68 (DEC-SF1 = A): `SSL_CERT_FILE` / `SSL_CERT_DIR` add trusted roots to
//! the bundled ones, and an unusable value is a clear error at first use.
//!
//! There is no test that a local HTTPS server signed by a private CA fails
//! without the variable and succeeds with it: it needs a TLS server and a
//! certificate generator (`rcgen`, `rustls`) that are not dev-dependencies, and
//! adding them only for one test is not worth a second TLS stack in the dev
//! tree. The unknown-issuer wording is covered in `core::http`'s unit tests.
#![cfg(feature = "cloud")]

mod support;

use support::cloud::login;
use support::{run_with, Output, Project};

const CA: &str = include_str!("fixtures/tls/ca-a.pem");

/// `evnx vault list` against a closed port, signed in, with `env` added.
fn vault_list(env: &[(&str, &str)]) -> Output {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let p = Project::new();
    let cfg = p.path().join("cfg");
    login(&cfg, &url);
    let mut vars = vec![
        ("EVNX_SERVER", url.as_str()),
        ("EVNX_CONFIG_DIR", cfg.to_str().unwrap()),
    ];
    vars.extend_from_slice(env);
    run_with(p.path(), &["vault", "list"], &vars, None)
}

#[test]
fn a_missing_ssl_cert_file_exits_13_naming_variable_and_path() {
    let out = vault_list(&[("SSL_CERT_FILE", "/nonexistent/corp-ca.pem")]);
    out.assert_code(13);
    assert!(out.stderr.contains("SSL_CERT_FILE"), "{}", out.stderr);
    assert!(
        out.stderr.contains("/nonexistent/corp-ca.pem"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_file_with_no_certificate_exits_13() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("empty.pem");
    std::fs::write(&f, "").unwrap();
    let out = vault_list(&[("SSL_CERT_FILE", f.to_str().unwrap())]);
    out.assert_code(13);
    assert!(out.stderr.contains("SSL_CERT_FILE"), "{}", out.stderr);
    assert!(out.stderr.contains("empty.pem"), "{}", out.stderr);
}

#[test]
fn a_missing_ssl_cert_dir_exits_13() {
    let out = vault_list(&[("SSL_CERT_DIR", "/nonexistent/certs")]);
    out.assert_code(13);
    assert!(out.stderr.contains("SSL_CERT_DIR"), "{}", out.stderr);
}

#[test]
fn good_roots_leave_the_normal_failure_alone() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("ca.pem");
    std::fs::write(&f, CA).unwrap();
    std::fs::write(d.path().join("also.crt"), "not a certificate").unwrap();
    let out = vault_list(&[
        ("SSL_CERT_FILE", f.to_str().unwrap()),
        ("SSL_CERT_DIR", d.path().to_str().unwrap()),
    ]);
    out.assert_code(13);
    assert!(out.stderr.contains("could not reach"), "{}", out.stderr);
    // An unreadable-or-invalid file inside the directory is a warning only.
    assert!(out.stderr.contains("warning: ignoring"), "{}", out.stderr);
}
