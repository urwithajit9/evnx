//! `evnx set` asks for a value on a terminal: a secret with no echo.
#![cfg(unix)]

mod support;

use support::Project;
use support::pty::Pty;

const TYPED: &str = "typed-secret-5e8d21";

#[test]
fn a_secret_name_is_asked_for_with_no_echo_and_never_shown() {
    let p = Project::new().env("API_TOKEN=old-value\n");
    let mut s = Pty::spawn(p.path(), &["set", "API_TOKEN"]);
    s.wait_for("Value for API_TOKEN");
    s.wait_for_echo_off();
    s.type_line(TYPED);
    s.wait_for("Replace API_TOKEN in ./.env?");
    s.type_line("y");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(!rest.contains(TYPED), "the secret was echoed:\n{rest}");
    assert!(rest.contains("value hidden"), "{rest}");
    assert_eq!(p.read(".env"), format!("API_TOKEN={TYPED}\n"));
}

#[test]
fn a_plain_name_is_asked_for_with_echo_and_shows_old_and_new() {
    let p = Project::new().env("PORT=3000\n");
    let mut s = Pty::spawn(p.path(), &["set", "PORT"]);
    s.wait_for("Value for PORT");
    s.type_line("8080");
    s.wait_for("Replace PORT in ./.env?");
    s.type_line("y");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("3000 → 8080"), "{rest}");
    assert_eq!(p.read(".env"), "PORT=8080\n");
}

#[test]
fn answering_no_changes_nothing_and_exits_4() {
    let p = Project::new().env("PORT=3000\n");
    let mut s = Pty::spawn(p.path(), &["set", "PORT=1"]);
    s.wait_for("Replace PORT in ./.env?");
    s.type_line("n");
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert_eq!(p.read(".env"), "PORT=3000\n");
}
