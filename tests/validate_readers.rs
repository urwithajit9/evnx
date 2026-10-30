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

/// B2 P0.5: a carriage return inside a value is reported by `validate` and
/// `doctor` whatever the readers, since python-dotenv and Node dotenv split the
/// line there. The value is never printed.
#[test]
fn p0_5_a_lone_carriage_return_is_reported_by_validate_and_doctor() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("PORT=8080\nTOKEN=s3cr3t\rLEAK=w\n")
        .example("PORT=\nTOKEN=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let found: Vec<_> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "lone_carriage_return")
        .collect();
    assert_eq!(found.len(), 1, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "TOKEN");
    assert_eq!(found[0]["severity"], "warning");
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);

    let out = run(p.path(), &["doctor"]);
    assert!(
        out.all()
            .contains(".env: a carriage return inside TOKEN — python-dotenv and Node dotenv"),
        "{}",
        out.all()
    );
    assert!(!out.all().contains("s3cr3t"), "{}", out.all());
}

/// B2 P0.6: a NUL byte in a value is an error in `validate` and `doctor`. It
/// parses, but `exec` and Docker refuse such an environment. Measured: Compose's
/// container fails with "invalid environment variable: contains nul byte".
#[test]
fn p0_6_a_nul_byte_is_an_error_in_validate_and_doctor() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("PORT=8080\nTOKEN=s3cr3t\0x\n")
        .example("PORT=\nTOKEN=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    assert_eq!(out.code, Some(1), "{}", out.all());
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let found: Vec<_> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "nul_byte")
        .collect();
    assert_eq!(found.len(), 1, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "TOKEN");
    assert_eq!(found[0]["severity"], "error");
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);

    let out = run(p.path(), &["doctor"]);
    assert!(
        out.all()
            .contains(".env: a NUL byte inside TOKEN — no process can receive it"),
        "{}",
        out.all()
    );
    assert!(!out.all().contains("s3cr3t"), "{}", out.all());
}

/// D10, B2 P0.7: a value Docker Compose reads differently top to bottom is a
/// warning in `validate` and one line in `doctor`, whatever the readers. The
/// value evnx reads is unchanged, and no value is printed.
#[test]
fn p0_7_a_value_read_differently_top_to_bottom_is_reported() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("HOST=s3cr3t-a\nURL=${HOST}/x\nHOST=s3cr3t-b\n")
        .example("HOST=\nURL=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let found: Vec<_> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "expansion_order")
        .collect();
    assert_eq!(found.len(), 1, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "URL");
    assert_eq!(found[0]["severity"], "warning");
    assert_eq!(
        found[0]["message"],
        "URL (line 2) uses HOST, assigned again below it on line 3. Docker Compose and \
         python-dotenv expand top to bottom, so they read a different value for URL than evnx does"
    );
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);

    let out = run(p.path(), &["doctor"]);
    assert!(
        out.all().contains(
            ".env: 1 value is read differently top to bottom by Docker Compose and python-dotenv (URL)"
        ),
        "{}",
        out.all()
    );
    assert!(!out.all().contains("s3cr3t"), "{}", out.all());
}
