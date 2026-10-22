//! `evnx doctor`: which file is the project's template (I-294).
//!
//! The `env_example` check looked for `.env.example` only. A project whose
//! template is `.env.sample` or `.env.template`, or that names it in
//! `.evnx.toml` (`[defaults] example`, the rule validate, sync, diff and spec
//! init follow — I-189), was told ".env.example not found" and checked against
//! a file it does not use. One rule now: `[defaults] example`, else the first
//! of `.env.example`, `.env.sample`, `.env.template`. `doctor PATH` reads
//! PATH's own `.evnx.toml`, as if run there.

mod support;

use support::{run, Project};

const IGNORE: &str = ".env*\n!.env.example\n!.env.sample\n!.env.template\n";

/// A git project with a private `.env`, a marker, and no template yet.
fn project() -> Project {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .file("package.json", "{}\n")
        .env("A=1\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p
}

#[test]
fn t_i294_a_sample_file_is_the_template() {
    let p = project().file(".env.sample", "A=\n");
    p.track(".env.sample");

    let out = run(p.path(), &["doctor", "--strict"]);
    out.assert_code(0);
    out.assert_not_contains(".env.example not found");
}

#[test]
fn t_i294_an_untracked_template_is_named() {
    let p = project().file(".env.template", "A=\n");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_contains(".env.template is not tracked in Git — run: git add .env.template");
}

#[test]
fn t_i294_defaults_example_names_the_template() {
    let p = project()
        .toml("[defaults]\nexample = \"config/env.template\"\n")
        .file("config/env.template", "A=\n");
    p.track("config/env.template");

    let out = run(p.path(), &["doctor", "--strict"]);
    out.assert_code(0);
    out.assert_not_contains("not found");
}

#[test]
fn t_i294_a_configured_template_that_is_missing_says_where_it_was_named() {
    let p = project()
        .toml("[defaults]\nexample = \".env.sample\"\n")
        .file(".env.example", "A=\n");
    p.track(".env.example");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_contains(".env.sample not found");
    out.assert_contains("[defaults] example");
}

#[test]
fn t_i294_doctor_path_reads_that_projects_config() {
    let outer = Project::new();
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .file("package.json", "{}\n")
        .toml("[defaults]\nexample = \".env.sample\"\n")
        .file(".env.sample", "A=\n")
        .env("A=1\n");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p.track(".env.sample");

    // Run from somewhere else, naming the project.
    let out = run(
        outer.path(),
        &["doctor", p.path().to_str().unwrap(), "--strict"],
    );
    out.assert_code(0);
    out.assert_not_contains("not found");
}

/// DEC-D8 with the template's real name: the error and the command to create
/// `.env` both use it.
#[test]
fn t_i294_a_template_without_env_names_the_real_template() {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .file("package.json", "{}\n")
        .file(".env.sample", "A=\n");
    p.track(".env.sample");

    let out = run(p.path(), &["doctor"]);
    out.assert_code(1);
    out.assert_contains(".env not found, though .env.sample is here");
    out.assert_contains("evnx sync --direction reverse --example .env.sample");
}

/// The JSON names the file in `details`; `name` and `description` stay what
/// programs parse.
#[test]
fn t_i294_json_keeps_the_check_name_and_description() {
    let p = project().file(".env.sample", "A=\n");

    let out = run(p.path(), &["doctor", "--format", "json"]);
    let report: serde_json::Value = serde_json::from_str(&out.stdout).expect("JSON");
    let check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "env_example")
        .expect("env_example")
        .clone();
    assert_eq!(
        check["description"],
        "Check .env.example exists and is Git-tracked"
    );
    assert!(
        check["details"]
            .as_str()
            .unwrap_or_default()
            .contains("git add .env.sample"),
        "{check}"
    );
}
