//! `init` says only what it did: it does not claim "nothing was generated"
//! when it wrote `.gitignore` or `.evnx.toml`, says where the contract's names
//! came from, and does not promise to append names to an existing `.env`.

mod support;

use support::{run, Project};

const TEMPLATE: &str = "PORT=3000\nDATABASE_URL=postgres://u:p@h/db\nAPI_SECRET_KEY=x\nNAME=a\n";

#[test]
fn the_contract_line_says_where_the_names_came_from() {
    let p = Project::new().example(TEMPLATE).env("PORT=1\n");
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0)
        .assert_contains("4 names from .env.example → .evnx.toml (2 secret, 2 with a format)");
    assert!(p.exists(".evnx.toml"));
}

#[test]
fn no_claim_that_nothing_was_generated_when_files_were_written() {
    let p = Project::new().example(TEMPLATE).env("PORT=1\n");
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0)
        .assert_not_contains("nothing was generated")
        .assert_not_contains("Nothing was generated");
    assert!(p.exists(".gitignore"));
}

#[test]
fn an_env_file_that_lacks_names_is_told_the_command_that_adds_them() {
    let p = Project::new().example(TEMPLATE).env("PORT=1\n");
    run(p.path(), &["init", "--yes"])
        .assert_code(0)
        .assert_contains(".env lacks 3 names")
        .assert_contains("evnx sync --direction reverse");
    assert_eq!(p.read(".env"), "PORT=1\n", "init must leave .env alone");
}

#[test]
fn help_does_not_promise_appending() {
    let p = Project::new();
    let out = run(p.path(), &["init", "--help"]);
    out.assert_code(0)
        .assert_not_contains("names it lacks are appended")
        .assert_contains("evnx sync --direction reverse");
}
