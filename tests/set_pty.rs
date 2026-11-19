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

#[test]
fn a_value_from_a_file_is_confirmed_and_never_shown() {
    const CANARY: &str = "file-canary-7c41e0";
    let p = Project::new()
        .env("PORT=3000\n")
        .file("v.txt", &format!("{CANARY}\nsecond line\n"));
    let mut s = Pty::spawn(p.path(), &["set", "PORT", "--from-file", "v.txt"]);
    s.wait_for("Replace PORT in ./.env?");
    s.type_line("y");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(!rest.contains(CANARY), "the value was shown:\n{rest}");
    assert!(rest.contains("value hidden; "), "{rest}");
    assert_eq!(p.read(".env"), format!("PORT=\"{CANARY}\\nsecond line\"\n"));
}

#[test]
fn from_file_dash_on_a_terminal_is_refused() {
    let p = Project::new().env("PORT=3000\n");
    let mut s = Pty::spawn(p.path(), &["set", "PORT", "--from-file", "-", "--yes"]);
    s.wait_for("standard input is a terminal");
    let (rest, code) = s.finish();
    assert_eq!(code, 2, "{rest}");
    assert_eq!(p.read(".env"), "PORT=3000\n");
}

// ── set --fill ───────────────────────────────────────────────────────────

const FILL_ENV: &str = "\
# app
PORT=3000
API_TOKEN=
DATABASE_URL=YOUR_DATABASE_URL_HERE
SECRET_KEY=changeme
DEBUG=true
";

#[test]
fn fill_asks_in_file_order_hides_a_secret_and_skips_on_enter() {
    const TOKEN: &str = "tok-typed-hidden-31ab9e";
    let p = Project::new().env(FILL_ENV);
    let mut s = Pty::spawn(p.path(), &["set", "--fill"]);
    s.wait_for("Value for API_TOKEN (1 of 3)");
    s.wait_for_echo_off();
    s.type_line(TOKEN);
    s.wait_for("Value for DATABASE_URL (2 of 3)");
    s.type_line("postgres://db/app");
    s.wait_for("Value for SECRET_KEY (3 of 3)");
    s.wait_for_echo_off();
    s.type_line("");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(!rest.contains(TOKEN), "the secret was echoed:\n{rest}");
    assert!(rest.contains("filled API_TOKEN, DATABASE_URL"), "{rest}");
    assert!(rest.contains("skipped: SECRET_KEY"), "{rest}");
    assert_eq!(
        p.read(".env"),
        format!(
            "# app\nPORT=3000\nAPI_TOKEN={TOKEN}\nDATABASE_URL=postgres://db/app\nSECRET_KEY=changeme\nDEBUG=true\n"
        )
    );
    assert!(p.exists(".env.bak"));
}

#[test]
fn fill_with_every_answer_skipped_writes_nothing() {
    let p = Project::new().env(FILL_ENV);
    let mut s = Pty::spawn(p.path(), &["set", "--fill"]);
    for name in ["API_TOKEN", "DATABASE_URL", "SECRET_KEY"] {
        s.wait_for(&format!("Value for {name}"));
        s.type_line("");
    }
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("nothing was written"), "{rest}");
    assert_eq!(p.read(".env"), FILL_ENV);
    assert!(!p.exists(".env.bak"));
}

#[test]
fn fill_only_asks_for_the_names_that_match() {
    let p = Project::new().env(FILL_ENV);
    let mut s = Pty::spawn(p.path(), &["set", "--fill", "--only", "DATABASE_*"]);
    s.wait_for("Value for DATABASE_URL (1 of 1)");
    s.type_line("postgres://x");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(!rest.contains("Value for API_TOKEN"), "{rest}");
    assert!(p.read(".env").contains("DATABASE_URL=postgres://x"));
    assert!(p.read(".env").contains("API_TOKEN=\n"));
}

#[test]
fn fill_ctrl_c_before_the_end_writes_nothing() {
    let p = Project::new().env(FILL_ENV);
    let mut s = Pty::spawn(p.path(), &["set", "--fill"]);
    s.wait_for("Value for API_TOKEN");
    s.type_line("typed-before-interrupt");
    s.wait_for("Value for DATABASE_URL");
    s.press(&["\u{3}"]);
    let (_rest, _status) = s.finish_raw();
    assert_eq!(p.read(".env"), FILL_ENV);
    assert!(!p.exists(".env.bak"));
}
