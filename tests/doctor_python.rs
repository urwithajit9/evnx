//! `doctor` on a Python project that has no `pyproject.toml` (U8d, I-175).
//!
//! A Pipenv project (`Pipfile`) or one with only `poetry.lock` aborted the
//! whole run — `Error: No such file or directory`, exit 2 — because the
//! dependency check read `pyproject.toml` unconditionally. Decided: read the
//! file that defines the type, never abort.

mod support;

use support::{run, Project};

fn assert_detected(p: &Project, args: &[&str]) -> support::Output {
    let out = run(p.path(), args);
    assert_ne!(out.code, Some(2), "doctor gave no verdict:\n{}", out.all());
    // A plain run lists a passing check without its details; `-v` names the type.
    let wanted = if args.contains(&"--verbose") {
        "Detected Python project"
    } else {
        "✓  Detect project type"
    };
    assert!(out.all().contains(wanted), "{wanted}:\n{}", out.all());
    out
}

#[test]
fn t_i175_a_pipfile_project_gets_a_verdict() {
    let p = Project::new()
        .env("A=1\n")
        .file("Pipfile", "[packages]\nflask = \"*\"\n");
    assert_detected(&p, &["doctor"]);
    let out = assert_detected(&p, &["doctor", "--verbose"]);
    assert!(out.all().contains("Consider adding"), "{}", out.all());
}

#[test]
fn t_i175_a_poetry_lock_project_gets_a_verdict() {
    let p = Project::new().env("A=1\n").file("poetry.lock", "");
    assert_detected(&p, &["doctor"]);
    assert_detected(&p, &["doctor", "--verbose"]);
}

/// The file that defines the type is the one read: a Pipfile that lists
/// python-dotenv is not told to add it.
#[test]
fn t_i175_the_pipfile_is_what_is_read() {
    let p = Project::new()
        .env("A=1\n")
        .file("Pipfile", "[packages]\npython-dotenv = \"*\"\n");
    let out = assert_detected(&p, &["doctor", "--verbose"]);
    assert!(!out.all().contains("Consider adding"), "{}", out.all());
}
