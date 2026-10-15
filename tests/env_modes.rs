//! The modes of the env files evnx creates (G9, U2 part 2).
//!
//! `.env` holds real values: created 0600. `.env.example` is meant to be
//! committed: created 0644. An existing file keeps the mode its owner set, and
//! evnx says so when others can read a file holding secrets (DEC-G6).
//!
//! I-237: `init` created `.env` with the umask's mode (644), and `evnx doctor`
//! then warned about the file init had just made.
#![cfg(unix)]

mod support;

use std::os::unix::fs::PermissionsExt;

use support::{run, Project};

fn mode(p: &Project, name: &str) -> u32 {
    std::fs::metadata(p.join(name))
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// T-INIT-27.
#[test]
fn t_i237_init_creates_env_0600_and_the_template_0644() {
    let p = Project::new();
    run(p.path(), &["init", "--with", "nextjs", "--yes"]).assert_code(0);
    assert_eq!(mode(&p, ".env"), 0o600);
    assert_eq!(mode(&p, ".env.example"), 0o644);
}

#[test]
fn t_i237_add_creates_env_0600() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert_eq!(mode(&p, ".env"), 0o600);
    assert_eq!(mode(&p, ".env.example"), 0o644);
}

#[test]
fn t_dec_g6_add_keeps_an_existing_env_mode_and_says_so() {
    let p = Project::new().env("KEEP=1\n").mode(".env", 0o644);
    let out = run(p.path(), &["add", "service", "redis", "--yes"]);
    out.assert_code(0);
    assert_eq!(mode(&p, ".env"), 0o644);
    assert!(
        out.stderr.contains("readable by other users"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("chmod 600 "), "{}", out.stderr);
}

#[test]
fn t_dec_g6_a_private_env_gets_no_warning() {
    let p = Project::new().env("KEEP=1\n").mode(".env", 0o600);
    let out = run(p.path(), &["add", "service", "redis", "--yes"]);
    out.assert_code(0);
    assert!(
        !out.all().contains("readable by other users"),
        "{}",
        out.all()
    );
}
