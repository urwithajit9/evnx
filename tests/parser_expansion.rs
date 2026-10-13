//! I-141 at the binary level (T-SHARE-23): a `.env` built from its own values
//! reads the same through every command that prints values.
//!
//! The defect was in `core::Parser`, which all commands share: a bare `$NAME`
//! lost the character after it, so `URL=$BASE/api` reached `convert`,
//! `template` and the rest as `http://localhostapi`. The unit tests in
//! `src/core/parser.rs` pin the parser; this pins what users see.

mod support;

use support::{run, Project};

const ENV: &str = "BASE=http://localhost\n\
                   URL=$BASE/api\n\
                   PORTED=$BASE:8080\n\
                   HOSTED=$BASE.example.com\n\
                   SHELLISH=$HOME_NOT_SET/bin\n";

const WANT: &[(&str, &str)] = &[
    ("URL", "http://localhost/api"),
    ("PORTED", "http://localhost:8080"),
    ("HOSTED", "http://localhost.example.com"),
    // Undefined bare references stay literal, terminator included.
    ("SHELLISH", "$HOME_NOT_SET/bin"),
];

fn project() -> Project {
    Project::new().env(ENV).example(ENV)
}

#[test]
fn convert_keeps_the_character_after_a_reference() {
    let p = project();
    run(p.path(), &["convert", "--to", "json", "-o", "out.json"]).assert_code(0);
    let json: serde_json::Value = serde_json::from_str(&p.read("out.json")).unwrap();
    for (key, want) in WANT {
        assert_eq!(json[key], *want, "{key}");
    }
}

#[test]
fn template_keeps_the_character_after_a_reference() {
    let p = project().file(
        "t.tpl",
        "URL={{URL}}\nPORTED={{PORTED}}\nHOSTED={{HOSTED}}\nSHELLISH={{SHELLISH}}\n",
    );
    run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "out.txt",
            "--no-gitignore",
        ],
    )
    .assert_code(0);
    let out = p.read("out.txt");
    for (key, want) in WANT {
        assert!(out.contains(&format!("{key}={want}\n")), "{key}:\n{out}");
    }
}
