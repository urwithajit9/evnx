//! What `doctor` says it did not do: the report ends with what it did not
//! examine, "no agent configuration" is not a pass, and a missing dotenv
//! library is a remark that never changes the exit code.

mod support;

use support::{run, Project};

fn healthy() -> Project {
    let p = Project::new()
        .git()
        .env("A=1\n")
        .example("A=\n")
        .file(".gitignore", ".env*\n!.env.example\n")
        .private(".env");
    p.track(".env.example");
    p
}

#[test]
fn every_text_report_ends_with_what_was_not_examined() {
    let p = healthy();
    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    let text = out.stdout;
    let tail = text.split("Not examined here:").nth(1).expect("block");
    for needle in ["evnx validate", "evnx sync --check", "evnx scan"] {
        assert!(tail.contains(needle), "{needle} missing:\n{text}");
    }
    // It is the last thing before the docs hint.
    assert!(text.find("checks passed").unwrap() < text.find("Not examined here:").unwrap());
}

#[test]
fn a_project_with_no_agent_configuration_is_told_nothing_was_checked() {
    let p = healthy();
    let out = run(p.path(), &["doctor"]);
    out.assert_contains("AI coding agents and .env")
        .assert_contains("nothing was checked")
        .assert_not_contains("cannot read .env");
}

#[test]
fn no_dotenv_library_is_info_and_never_fails_strict() {
    let p = healthy().file("requirements.txt", "flask\n");
    let out = run(p.path(), &["doctor", "--strict"]);
    out.assert_code(0)
        .assert_contains("No library that loads .env");
    let json = run(p.path(), &["doctor", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json.stdout).expect("json");
    let check = doc["checks"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["name"] == "project_structure"))
        .expect("project_structure");
    assert_eq!(check["severity"], "info", "{check}");
}

#[test]
fn another_loader_is_accepted() {
    for req in [
        "pydantic-settings==2.1\n",
        "django_environ\n",
        "environs[django]>=9\n",
        "python-decouple\n",
    ] {
        let p = healthy().file("requirements.txt", req);
        run(p.path(), &["doctor"])
            .assert_code(0)
            .assert_not_contains("No library that loads .env");
    }
}

#[test]
fn node_env_file_flag_is_accepted() {
    let p = healthy().file(
        "package.json",
        r#"{"scripts":{"start":"node --env-file=.env server.js"}}"#,
    );
    run(p.path(), &["doctor", "--verbose"])
        .assert_code(0)
        .assert_not_contains("No .env loader found");
}
