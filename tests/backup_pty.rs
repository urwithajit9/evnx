//! `backup` and `restore` prompts, through a real terminal (T-BR-06, T-BR-32).
//!
//! The passphrase prompt and the overwrite question cannot be reached through
//! a pipe; these press the keys a person would.
#![cfg(all(unix, feature = "backup"))]

mod support;

use support::pty::{Pty, ENTER};
use support::{run, Project};

const ENV: &str = "A=1\nB=two\n";
const PW: &str = "demo-passphrase-123\n";

/// Type a hidden passphrase once the prompt is reading it.
fn type_secret(run: &mut Pty, prompt: &str, secret: &str) {
    run.wait_for(prompt);
    run.wait_for_echo_off();
    run.type_line(secret);
}

/// T-BR-06: the prompt keeps its 8-character floor (now shared with the key
/// file and `EVNX_PASSWORD`, I-147).
#[test]
fn t_br06_a_short_typed_passphrase_is_refused() {
    let p = Project::new().env(ENV);
    let mut run = Pty::spawn(p.path(), &["backup"]);
    type_secret(&mut run, "Enter encryption password", "short");
    let (rest, code) = run.finish();
    assert_eq!(code, 2, "{rest}");
    assert!(rest.contains("at least 8 characters (got 5)"), "{rest}");
    assert!(!p.exists(".env.backup"));
}

#[test]
fn t_br06_a_mismatched_confirmation_is_exit_3() {
    let p = Project::new().env(ENV);
    let mut run = Pty::spawn(p.path(), &["backup"]);
    type_secret(&mut run, "Enter encryption password", "long-enough-1");
    type_secret(&mut run, "Confirm password", "long-enough-2");
    let (rest, code) = run.finish();
    assert_eq!(code, 3, "{rest}");
    assert!(!p.exists(".env.backup"));
}

#[test]
fn t_br06_a_typed_passphrase_round_trips() {
    let p = Project::new().env(ENV);
    let mut b = Pty::spawn(p.path(), &["backup"]);
    type_secret(&mut b, "Enter encryption password", "long-enough-1");
    type_secret(&mut b, "Confirm password", "long-enough-1");
    assert_eq!(b.finish().1, 0);

    let mut r = Pty::spawn(p.path(), &["restore", ".env.backup", "--output", "out.env"]);
    type_secret(&mut r, "Enter decryption password", "long-enough-1");
    assert_eq!(r.finish().1, 0);
    assert_eq!(p.read("out.env"), ENV);
}

/// Wait for the question restore asks after deriving the key.
///
/// ⚠️ Not [`Pty::wait_for`]'s 15 s. Before "Overwrite .env?" appears, each
/// test has derived the Argon2 key twice (its `backup`, then `restore`): about
/// 1.7 s each in a debug build on a 16-core desktop, several times that on a
/// 3-core macOS runner with the tests in parallel. Both t_br32 tests timed out
/// there once and passed on the re-run of the same commit (I-300).
fn wait_after_kdf(run: &mut Pty, text: &str) {
    run.try_wait_for(text, std::time::Duration::from_secs(60))
        .unwrap_or_else(|| panic!("never saw {text:?} within 60 s"));
}

/// A project with `.env.backup` (of [`ENV`]) and a different `.env`.
fn existing_output() -> Project {
    let p = Project::new().env(ENV).file("pw", PW);
    run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    std::fs::write(p.join(".env"), "OLD=1\n").unwrap();
    p
}

/// T-BR-32: `y` replaces the file and keeps the old one, as `--yes` does
/// (DEC-B11): a confirmed replacement is the same replacement.
#[test]
fn t_br32_yes_at_the_question_replaces_and_keeps_a_copy() {
    let p = existing_output();
    let mut run = Pty::spawn(p.path(), &["restore", ".env.backup", "--key-file", "pw"]);
    wait_after_kdf(&mut run, "Overwrite .env?");
    run.wait_for("[y/N]");
    run.press(&["y", ENTER]);
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert_eq!(p.read(".env"), ENV);
    assert_eq!(p.read(".env.bak"), "OLD=1\n");
}

/// Enter takes the default, No: exit 4, nothing changed.
#[test]
fn t_br32_enter_at_the_question_cancels() {
    let p = existing_output();
    let mut run = Pty::spawn(p.path(), &["restore", ".env.backup", "--key-file", "pw"]);
    wait_after_kdf(&mut run, "Overwrite .env?");
    run.wait_for("[y/N]");
    run.press(&[ENTER]);
    let (rest, code) = run.finish();
    assert_eq!(code, 4, "{rest}");
    assert_eq!(p.read(".env"), "OLD=1\n");
    assert!(!p.exists(".env.bak"));
}
