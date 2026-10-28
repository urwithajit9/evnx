//! RM-04, phase 1: lines this project's readers read differently from evnx.
//! The values behind these findings are measured (`src/assets/dialects.json`):
//! Docker Compose and python-dotenv read the cases as evnx does; Node `dotenv`
//! does not.

mod support;

use support::{run, Project};

const ENV: &str = "PORT=8080\nAPI_KEY=   # fill this in later\nPW=Tr0ub4dor#3\n";

fn project() -> Project {
    Project::new()
        .file(".gitignore", ".env\n")
        .env(ENV)
        .example("PORT=\nAPI_KEY=\nPW=\n")
}

fn findings(out: &support::Output) -> Vec<serde_json::Value> {
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "reader_mismatch")
        .cloned()
        .collect()
}

#[test]
fn rm04_a_node_project_hears_about_the_lines_node_reads_differently() {
    let p = project().file("package.json", r#"{"dependencies":{"dotenv":"^18"}}"#);
    let out = run(p.path(), &["validate", "--format", "json"]);
    let found = findings(&out);
    assert_eq!(found.len(), 2, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "API_KEY");
    assert!(
        found[0]["message"]
            .as_str()
            .unwrap()
            .contains("Node dotenv reads it as empty"),
        "{}",
        found[0]
    );
    assert_eq!(found[1]["variable"], "PW");
    // A value is never printed.
    assert!(!out.stdout.contains("Tr0ub4dor"), "{}", out.stdout);

    let out = run(
        p.path(),
        &[
            "validate",
            "--format",
            "json",
            "--ignore",
            "reader_mismatch",
        ],
    );
    assert!(findings(&out).is_empty());
}

#[test]
fn rm04_readers_that_agree_with_evnx_say_nothing() {
    // Measured: Compose and python-dotenv read every case as evnx does.
    let p = project()
        .file("requirements.txt", "python-dotenv==1.2.2\n")
        .file("compose.yaml", "services: {}\n");
    assert!(findings(&run(p.path(), &["validate", "--format", "json"])).is_empty());
    // No reader detected: nothing either.
    assert!(findings(&run(project().path(), &["validate", "--format", "json"])).is_empty());
}

#[test]
fn rm04_doctor_names_the_readers_in_one_line() {
    let p = project().file("package.json", r#"{"devDependencies":{"dotenv":"^18"}}"#);
    let out = run(p.path(), &["doctor"]);
    assert!(
        out.all()
            .contains(".env is read by evnx and Node dotenv; 2 lines are read differently"),
        "{}",
        out.all()
    );
}
