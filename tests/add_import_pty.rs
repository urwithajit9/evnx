//! `evnx add import` asks before it writes, on a terminal.
#![cfg(unix)]

mod support;

use support::Project;
use support::pty::Pty;

const CANARY: &str = "pty-import-canary-88e1";

fn project() -> Project {
    Project::new().env("KEEP=1\n").example("KEEP=\n").file(
        "s.json",
        &format!(r#"{{"db":{{"pass":"{CANARY}"}},"port":"1"}}"#),
    )
}

#[test]
fn it_shows_the_mapping_asks_and_writes_on_yes() {
    let p = project();
    let mut s = Pty::spawn(p.path(), &["add", "import", "s.json"]);
    let asked = s.wait_for("Append these variables to");
    s.type_line("y");
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(
        asked.contains("db.pass") && asked.contains("DB__PASS"),
        "{asked}"
    );
    assert!(
        !asked.contains(CANARY) && !rest.contains(CANARY),
        "a value was shown:\n{asked}{rest}"
    );
    assert!(p.read(".env").contains(&format!("DB__PASS={CANARY}")));
}

#[test]
fn answering_no_writes_nothing_and_exits_4() {
    let p = project();
    let mut s = Pty::spawn(p.path(), &["add", "import", "s.json"]);
    s.wait_for("Append these variables to");
    s.type_line("n");
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert_eq!(p.read(".env"), "KEEP=1\n");
}
