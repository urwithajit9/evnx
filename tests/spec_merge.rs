//! `evnx spec init` into a config that already has `[vars]` (U8d, I-178,
//! DEC-SP3; I-179 falls out of the same guard).
//!
//! `--force` appended a second `[vars]` block "to be merged by hand"; with any
//! repeated name TOML refuses the file and every command that reads the
//! config exits 2. Decided: `--force` is gone, with a hint; `--merge` adds only
//! the names not yet declared and keeps every hand edit; and no write may
//! leave a file that does not parse.

mod support;

use support::{run, Project};

fn app() -> Project {
    Project::new()
        .env("DATABASE_URL=postgres://u:p@db/app\nPORT=8080\nDEBUG=true\n")
        .example("DATABASE_URL=\nPORT=\nDEBUG=\n")
}

fn toml_of(p: &Project) -> String {
    std::fs::read_to_string(p.path().join(".evnx.toml")).unwrap()
}

const HAND: &str = "[scan]\nseverity = \"high\"\n\n# kept by hand\n[vars.PORT]\nrequired = false\n";

#[test]
fn t_i178_force_is_refused_with_a_hint() {
    let p = app().file(".evnx.toml", HAND);
    let out = run(p.path(), &["spec", "init", "--force"]);
    out.assert_code(2);
    assert!(out.all().contains("--merge"), "{}", out.all());
    assert_eq!(toml_of(&p), HAND, "nothing written");
}

#[test]
fn t_i178_merge_adds_only_missing_names_and_keeps_edits() {
    let p = app().file(".evnx.toml", HAND);
    let out = run(p.path(), &["spec", "init", "--merge"]);
    out.assert_code(0);
    let t = toml_of(&p);
    assert!(t.starts_with(HAND), "hand edits kept byte for byte:\n{t}");
    assert_eq!(t.matches("[vars.PORT]").count(), 1, "{t}");
    for name in ["[vars.DATABASE_URL]", "[vars.DEBUG]"] {
        assert_eq!(t.matches(name).count(), 1, "{name}:\n{t}");
    }
    // The result is readable by every command.
    let v = run(p.path(), &["validate"]);
    assert_ne!(v.code, Some(2), "{}", v.all());
}

#[test]
fn t_i178_merge_with_nothing_new_writes_nothing() {
    let p = app().file(".evnx.toml", HAND);
    run(p.path(), &["spec", "init", "--merge"]).assert_code(0);
    let once = toml_of(&p);
    let out = run(p.path(), &["spec", "init", "--merge"]);
    out.assert_code(0);
    assert_eq!(toml_of(&p), once);
    assert!(out.all().contains("already declared"), "{}", out.all());
}

#[test]
fn t_i178_without_merge_existing_vars_are_refused_with_the_hint() {
    let p = app().file(".evnx.toml", HAND);
    let out = run(p.path(), &["spec", "init"]);
    out.assert_code(2);
    assert!(out.all().contains("--merge"), "{}", out.all());
    assert_eq!(toml_of(&p), HAND);
}

/// I-179: an inline `vars = { … }` slipped past the textual guard and the
/// append produced a file TOML refuses. Now nothing is written that does not
/// parse — with or without `--merge`.
#[test]
fn t_i179_inline_vars_are_never_broken() {
    let inline = "vars = { PORT = { required = false } }\n";
    for args in [&["spec", "init"][..], &["spec", "init", "--merge"][..]] {
        let p = app().file(".evnx.toml", inline);
        let out = run(p.path(), args);
        assert_eq!(
            toml_of(&p),
            inline,
            "{args:?} changed the file:\n{}",
            out.all()
        );
        assert_ne!(run(p.path(), &["validate"]).code, Some(2), "{args:?}");
    }
}
