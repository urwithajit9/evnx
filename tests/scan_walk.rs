//! Which files a folder scan reads (U8b, I-047, DEC-U8-2).
//!
//! The walk read only files whose extension was on a built-in list, so
//! `config/id_rsa.pem` — a real private key — and a live key in `app.swift`
//! were never opened, and the scan said "no secrets". The owner's decision:
//!
//! - **Default: every file is read.** Binary and over-size files are skipped,
//!   and the scan says which, so nothing is passed over in silence.
//! - **The extension list is opt-in and announced**: `--extensions` or
//!   `[scan] extensions` in `.evnx.toml`, and the run says it is limited.
//! - A file named on the command line is always scanned, as before.

mod support;

use support::{run, run_with, Project};

const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEowIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF0qZ3xOEqQ7dZmvCe0pPjyC2Kr8E\n\
-----END RSA PRIVATE KEY-----\n";

fn project() -> Project {
    Project::new()
        .env("APP=1\n")
        .file("config/id_rsa.pem", PEM)
        .file(
            "src/app.swift",
            "let key = \"sk_live_51H1234567890abcdefghijk\"\n",
        )
}

#[test]
fn t_i047_files_off_the_old_list_are_scanned() {
    let p = project();
    let out = run(p.path(), &["scan", "."]);
    out.assert_code(1);
    assert!(out.all().contains("id_rsa.pem"), "{}", out.all());
    assert!(out.all().contains("app.swift"), "{}", out.all());
}

#[test]
fn t_i047_binary_and_huge_files_are_skipped_and_named() {
    let p = Project::new().env("APP=1\n");
    std::fs::write(
        p.path().join("logo.png"),
        [0x89, b'P', b'N', b'G', 0, 0, 0, 13, 0xFF, 0x00, 0x13, 0x37],
    )
    .unwrap();
    std::fs::write(p.path().join("big.log"), "x=1\n".repeat(400_000)).unwrap();
    let out = run(p.path(), &["scan", ".", "-v"]);
    out.assert_code(0);
    assert!(out.stderr.contains("skipped"), "{}", out.stderr);
    assert!(out.stderr.contains("logo.png"), "{}", out.stderr);
    assert!(out.stderr.contains("big.log"), "{}", out.stderr);
    // Without -v it is still said, as a count.
    let out = run(p.path(), &["scan", "."]);
    out.assert_code(0);
    assert!(out.stderr.contains("skipped 2"), "{}", out.stderr);
}

#[test]
fn t_i047_the_extension_list_is_opt_in_and_announced() {
    let p = project();
    let out = run(p.path(), &["scan", ".", "--extensions", "env,txt"]);
    out.assert_code(0);
    assert!(!out.all().contains("Private Key"), "{}", out.all());
    assert!(
        out.stderr.contains("only") && out.stderr.contains("--extensions"),
        "the limit is announced:\n{}",
        out.stderr
    );
}

#[test]
fn t_i047_the_extension_list_from_config_is_announced() {
    let p = project().file(".evnx.toml", "[scan]\nextensions = [\"env\"]\n");
    let out = run_with(p.path(), &["scan", "."], &[], None);
    out.assert_code(0);
    // ⚠️ The override line itself, not just the word: an unknown-key warning
    // also contains "extensions", and passed the first version of this test.
    assert!(
        out.stderr.contains("scan.extensions="),
        "a config that narrows the scan is announced as an override:\n{}",
        out.stderr
    );
    assert!(!out.all().contains("Private Key"), "{}", out.all());
}

/// A file named on the command line is read whatever the list says.
#[test]
fn t_i047_a_named_file_is_always_scanned() {
    let p = project();
    let out = run(
        p.path(),
        &["scan", "config/id_rsa.pem", "--extensions", "env"],
    );
    out.assert_code(1);
    assert!(out.all().contains("Private Key"), "{}", out.all());
}
