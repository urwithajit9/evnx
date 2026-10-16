//! What evnx writes into a `.env` file reads back as the same value (U5, C-01).
//!
//! - I-114: `sync` wrote `KEY=value` raw, so `APP_NAME="My App # prod"` came
//!   back as `My App` — the rest read as a comment. `add custom` had the same
//!   defect (`MSG=hello # world`). Both now go through one writer that quotes
//!   when a value needs it.
//! - I-145: restore decided "is this a `.env`?" with its own line heuristic,
//!   which rejected `export A=1` and files with a PEM block, and wrote valid
//!   backups to `.env.restored`. The check is now the parser evnx reads `.env`
//!   with.

mod support;

use support::{run, Project};

/// Every key of `file` as `convert --to json` reads it back.
fn read_back(p: &Project, file: &str) -> serde_json::Value {
    let out = run(p.path(), &["convert", "--to", "json", "--env", file]);
    out.assert_code(0);
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

#[test]
fn t_i114_sync_actual_values_survive_a_round_trip() {
    let p = Project::new()
        .env("APP_NAME=\"My App # prod\"\nGREETING=\"hi there\"\nPRICE='$5 off'\nLOG=info\n")
        .file(
            "tc.json",
            r#"{"allow_actual":["APP_NAME","GREETING","PRICE","LOG"]}"#,
        );
    run(p.path(), &["sync", "-f", "--template-config", "tc.json"]).assert_code(0);
    let got = read_back(&p, ".env.example");
    assert_eq!(got["APP_NAME"], "My App # prod");
    assert_eq!(got["GREETING"], "hi there");
    assert_eq!(got["PRICE"], "$5 off");
    assert_eq!(got["LOG"], "info");
}

#[test]
fn t_i114_add_custom_values_survive_a_round_trip() {
    let p = Project::new().env("A=1\n");
    run(
        p.path(),
        &["add", "custom", "MSG=hello # world", "SPACED=a b", "--yes"],
    )
    .assert_code(0);
    // ⚠️ `.env`, not `.env.example`: since DEC-U8-1 (I-240) `add custom` puts a
    // placeholder in the template and the value only in `.env`.
    let got = read_back(&p, ".env");
    assert_eq!(got["MSG"], "hello # world");
    assert_eq!(got["SPACED"], "a b");
}

fn restore_of(content: &str) -> (Project, support::Output) {
    let p = Project::new()
        .env(content)
        .file("pw", "correct horse battery staple long\n");
    run(
        p.path(),
        &["backup", "--key-file", "pw", "--output", "e.bak"],
    )
    .assert_code(0);
    std::fs::remove_file(p.path().join(".env")).unwrap();
    let out = run(
        p.path(),
        &["restore", "e.bak", "--key-file", "pw", "--output", ".env"],
    );
    (p, out)
}

#[test]
fn t_i145_export_prefixed_file_restores_in_place() {
    let (p, out) = restore_of("export A=1\nexport B=2\n");
    out.assert_code(0);
    assert!(p.path().join(".env").exists(), "{}", out.all());
    assert!(!p.path().join(".env.restored").exists(), "{}", out.all());
}

#[test]
fn t_i145_pem_heavy_file_restores_in_place() {
    let body: String = (0..30)
        .map(|i| format!("QUJDREVGR0hJSktMTU5PUFFSU1RVVldY{i:02}\n"))
        .collect();
    let content = format!(
        "APP=1\nTLS_KEY=\"-----BEGIN PRIVATE KEY-----\n{body}-----END PRIVATE KEY-----\"\n"
    );
    let (p, out) = restore_of(&content);
    out.assert_code(0);
    assert!(p.path().join(".env").exists(), "{}", out.all());
    assert!(!p.path().join(".env.restored").exists(), "{}", out.all());
}

/// Still protected: content evnx cannot read as a `.env` goes aside.
#[test]
fn t_i145_prose_still_restores_aside() {
    let (p, out) = restore_of("This is a letter.\nIt has no assignments at all.\n");
    assert!(p.path().join(".env.restored").exists(), "{}", out.all());
    assert!(!p.path().join(".env").exists(), "{}", out.all());
}
