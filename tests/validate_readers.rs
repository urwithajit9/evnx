//! RM-04, phase 1: lines this project's readers read differently from evnx.
//! The values behind these findings are measured (`src/assets/dialects.json`):
//! Docker Compose and python-dotenv read most cases as evnx does; Node `dotenv`
//! does not. Compose reads `$$` as one `$` (P0.8).

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

/// D10 (0.13.0 P1): evnx expands top to bottom, so 0.12.0's P0.7 warning is
/// gone and the value is Compose's. A duplicate whose earlier assignment is
/// read in between is not called dead. `--ignore expansion_order` still runs.
/// No value is printed.
#[test]
fn p1_d10_values_read_top_to_bottom() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("HOST=s3cr3t-a\nURL=${HOST}/x\nHOST=s3cr3t-b\n")
        .example("HOST=\nURL=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let issues = json["issues"].as_array().unwrap();
    assert!(
        issues.iter().all(|i| i["type"] != "expansion_order"),
        "{}",
        out.stdout
    );
    let dup: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "duplicate_key")
        .collect();
    assert_eq!(dup.len(), 1, "{}", out.stdout);
    assert_eq!(
        dup[0]["message"],
        "HOST is assigned 2 times (lines 1, 3) — line 3 is its final value, and line 2 reads \
         an earlier one"
    );
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);

    // The retired type is accepted and changes nothing.
    let out = run(p.path(), &["validate", "--ignore", "expansion_order"]);
    assert_eq!(out.code, Some(0), "{}", out.all());

    // The value is Compose's: URL reads the HOST above it.
    let out = run(p.path(), &["convert", "--to", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(json["URL"], "s3cr3t-a/x", "{}", out.stdout);

    let out = run(p.path(), &["doctor"]);
    assert!(!out.all().contains("top to bottom"), "{}", out.all());
    assert!(!out.all().contains("s3cr3t"), "{}", out.all());
}

/// D10: a forward reference fails and names the line to move.
#[test]
fn p1_d10_a_forward_reference_names_the_line_to_move() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("URL=${HOST}/x\nHOST=s3cr3t\n")
        .example("HOST=\nURL=\n");
    let out = run(p.path(), &["validate"]);
    assert_ne!(out.code, Some(0), "{}", out.all());
    assert!(
        out.all().contains(
            "Undefined variable at line 1: ${HOST} is assigned below it, on line 2. evnx expands \
             top to bottom, as Docker Compose does: move that line above line 1"
        ),
        "{}",
        out.all()
    );
    assert!(!out.all().contains("s3cr3t"), "{}", out.all());
}

/// Compose reads `$$` as one `$`; evnx keeps both, so a Compose project hears
/// about it, with the measured fix, and never the value (D11, P0.8).
#[test]
fn p0_8_a_compose_project_hears_about_dollar_dollar() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .file("compose.yaml", "services: {}\n")
        .env("PW=s3cr3t$$x\nOK='s3cr3t$$y'\n")
        .example("PW=\nOK=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let found = findings(&out);
    assert_eq!(found.len(), 1, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "PW");
    assert_eq!(
        found[0]["message"],
        "PW (line 1): Docker Compose reads `$$` as a single `$`, while evnx reads it as written"
    );
    assert!(found[0]["suggestion"]
        .as_str()
        .unwrap()
        .starts_with("Single-quote the value"));
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);
}

/// D10: a bare `$NAME` naming a variable assigned below it is kept as written,
/// which 0.12.0 expanded silently. `validate` says so, without the value.
/// Compose (v5.7.0, measured) reads it as empty.
#[test]
fn p1_d10_a_bare_forward_reference_is_a_warning() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("M=$A/x\nOK=\"\\$A\"\nA=s3cr3t\n")
        .example("M=\nOK=\nA=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let found: Vec<_> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "forward_reference")
        .collect();
    assert_eq!(
        found.len(),
        1,
        "an escaped `\\$A` is not a reference: {}",
        out.stdout
    );
    assert_eq!(found[0]["variable"], "M");
    assert_eq!(found[0]["severity"], "warning");
    assert_eq!(
        found[0]["message"],
        "$A on line 1 names A, which is assigned below it on line 2. evnx expands top to \
         bottom, so it keeps the text $A; Docker Compose reads it as empty"
            .replace("line 2", "line 3")
    );
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);
}

/// D12: a name a shell cannot `export` is read, with a warning that says who
/// can and cannot receive it. Each name once, without its value.
#[test]
fn p1_d12_a_name_a_shell_cannot_export_is_a_warning() {
    let p = Project::new()
        .file(".gitignore", ".env\n")
        .env("spring.datasource.url=s3cr3t\nspring.datasource.url=s3cr3t2\nOK=1\n")
        .example("spring.datasource.url=\nOK=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let found: Vec<_> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "not_shell_name")
        .collect();
    assert_eq!(found.len(), 1, "{}", out.stdout);
    assert_eq!(found[0]["variable"], "spring.datasource.url");
    assert_eq!(found[0]["severity"], "warning");
    assert!(!out.stdout.contains("s3cr3t"), "{}", out.stdout);
    let out = run(
        p.path(),
        &["validate", "--ignore", "not_shell_name,duplicate_key"],
    );
    assert_eq!(out.code, Some(0), "{}", out.all());
}
