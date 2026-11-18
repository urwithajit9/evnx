//! What `evnx add custom` says it did, name by name: written, kept, or added
//! to the template only. A name whose existing value was kept is never
//! reported as added.

mod support;

use support::{Project, run};

fn project() -> Project {
    Project::new()
        .env("FLAG=false\nSAME=1\n")
        .example("OTHER=1\n")
        .file(".gitignore", ".env\n")
        .private(".env")
}

#[test]
fn a_kept_value_is_not_counted_as_added() {
    let p = project();
    let out = run(p.path(), &["add", "custom", "FLAG=true", "--yes"]);
    out.assert_code(0)
        .assert_contains("FLAG")
        .assert_contains("already set to a different value in .env")
        .assert_contains("kept")
        .assert_not_contains("Added 1");
    // The value on disk is unchanged, and nothing about the name went into
    // either file.
    assert!(p.read(".env").contains("FLAG=false"));
    assert!(!p.read(".env").contains("FLAG=true"));
    assert!(!p.read(".env.example").contains("FLAG"));
}

/// The usual layout: the template has the name too (`FLAG=`). The kept value
/// in `.env` is still reported, and the template's placeholder difference is
/// not listed as a second conflict.
#[test]
fn a_kept_value_is_reported_when_the_template_has_the_name_too() {
    let p = Project::new()
        .env("FLAG=false\n")
        .example("FLAG=\n")
        .file(".gitignore", ".env\n")
        .private(".env");
    let out = run(p.path(), &["add", "custom", "FLAG=true", "--yes"]);
    out.assert_code(0)
        .assert_contains("already set to a different value in .env")
        .assert_contains("kept")
        .assert_not_contains("Added 1")
        .assert_not_contains("in .env.example: kept");
    assert!(p.read(".env").contains("FLAG=false"));
    assert_eq!(p.read(".env.example"), "FLAG=\n");

    let out = run(p.path(), &["add", "custom", "FLAG=true", "--yes", "--json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    let conflicts = doc["conflicts"].as_array().expect("conflicts");
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0]["name"], "FLAG");
    assert_eq!(conflicts[0]["file"], ".env");
}

#[test]
fn the_count_is_names_written_to_the_env_file() {
    let p = project();
    let out = run(
        p.path(),
        &["add", "custom", "FLAG=true", "NEW_ONE=2", "SAME=1", "--yes"],
    );
    out.assert_code(0)
        .assert_contains("Added 1 custom variable to .env")
        .assert_contains("Added to .env.example only")
        .assert_contains("SAME");
    assert!(p.read(".env").contains("NEW_ONE=2"));
}

#[test]
fn a_secret_looking_kept_name_prints_no_value() {
    let p = Project::new()
        .env("API_TOKEN=old-secret-value\n")
        .file(".gitignore", ".env\n")
        .private(".env");
    let out = run(
        p.path(),
        &["add", "custom", "API_TOKEN=new-secret-value", "--yes"],
    );
    out.assert_code(0)
        .assert_contains("API_TOKEN")
        .assert_not_contains("old-secret-value")
        .assert_not_contains("new-secret-value");
}

#[test]
fn json_lists_kept_names_under_conflicts_not_added() {
    let p = project();
    let out = run(
        p.path(),
        &["add", "custom", "FLAG=true", "NEW_ONE=2", "--yes", "--json"],
    );
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    let names = |key: &str| -> Vec<String> {
        doc[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(names("added"), ["NEW_ONE"]);
    assert_eq!(names("env_added"), ["NEW_ONE"]);
    assert_eq!(doc["conflicts"][0]["name"], "FLAG");
    assert_eq!(doc["conflicts"][0]["file"], ".env");
    assert!(doc["conflicts"][0].get("existing").is_none(), "{doc}");
    assert!(doc["conflicts"][0].get("new").is_none(), "{doc}");
}

#[test]
fn dry_run_says_the_value_would_be_kept() {
    let p = project();
    run(p.path(), &["add", "custom", "FLAG=true", "--dry-run"])
        .assert_code(0)
        .assert_contains("would keep: FLAG");
}

#[cfg(unix)]
mod next_steps {
    use super::*;
    use support::pty::{ENTER, Pty};

    fn answer_and_finish(p: &Project, arg: &str) -> String {
        let mut s = Pty::spawn(p.path(), &["add", "custom", arg]);
        s.wait_for("Append these variables to");
        s.press(&["y", ENTER]);
        let (rest, code) = s.finish();
        assert_eq!(code, 0, "{rest}");
        rest
    }

    #[test]
    fn no_placeholder_step_when_a_value_was_given() {
        let p = Project::new().env("A=1\n").example("A=1\n");
        let out = answer_and_finish(&p, "NEW_ONE=2");
        assert!(out.contains("Next steps"), "{out}");
        assert!(!out.contains("replace placeholder"), "{out}");
    }

    #[test]
    fn placeholder_step_when_a_name_has_no_value() {
        let p = Project::new().env("A=1\n").example("A=1\n");
        let out = answer_and_finish(&p, "NEW_ONE");
        assert!(out.contains("replace placeholder values"), "{out}");
    }
}

#[test]
fn a_kept_name_is_not_declared_in_the_contract() {
    let p = Project::new()
        .env("FLAG=false\nOTHER=1\n")
        .example("OTHER=1\n")
        .toml("[vars.OTHER]\n")
        .file(".gitignore", ".env\n")
        .private(".env");
    run(
        p.path(),
        &["add", "custom", "FLAG=true", "NEW_ONE=2", "--yes"],
    )
    .assert_code(0);
    let toml = p.read(".evnx.toml");
    assert!(!toml.contains("FLAG"), "{toml}");
    assert!(toml.contains("NEW_ONE"), "{toml}");
    run(p.path(), &["spec", "check"]).assert_code(0);
}
