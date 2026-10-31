//! `evnx validate` on input that is not a `.env` file (B2, D3): `--env -` reads
//! stdin, and `--input-format json` reads a flat object or `evnx parse`'s JSON.
//! The format is never guessed, and `--fix` refuses both: there is no file.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn project() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env.example"), "DEBUG=\nPORT=\nAPI_KEY=\n").unwrap();
    d
}

fn validate(d: &TempDir, stdin: &str, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["validate", "--env", "-"])
        .args(args)
        .write_stdin(stdin)
        .output()
        .unwrap()
}

/// `(type, location)` of each finding.
fn findings(out: &std::process::Output) -> Vec<(String, String)> {
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)));
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["type"].as_str().unwrap().to_string(),
                i["location"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn b2_stdin_is_checked_as_a_dotenv_file() {
    let d = project();
    let out = validate(&d, "DEBUG=True\nPORT=8080\n", &["--format", "json"]);
    assert_eq!(
        findings(&out),
        [
            ("missing_variable".into(), "<stdin>".into()),
            ("boolean_trap".into(), "<stdin>".into()),
        ]
    );
    assert_eq!(out.status.code(), Some(1));
    // Lines are stdin's own.
    let out = validate(
        &d,
        "DEBUG=true\nPORT=1\nAPI_KEY=a\rb\n",
        &["--format", "json"],
    );
    assert!(
        findings(&out).contains(&("lone_carriage_return".into(), "<stdin>:3".into())),
        "{:?}",
        findings(&out)
    );
}

#[test]
fn b2_stdin_that_does_not_parse_lists_every_problem() {
    let out = validate(&project(), "my.key=1\nB=\"x\n", &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("Failed to parse <stdin>: 2 problems"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn b2_json_reads_a_flat_object_and_parse_output() {
    let d = project();
    let flat = r#"{"DEBUG":"true","PORT":"8080","API_KEY":"s3cr3t\u0000x"}"#;
    let out = validate(&d, flat, &["--input-format", "json", "--format", "json"]);
    let found = findings(&out);
    // No line to point at: `—`, never a made-up line number.
    assert!(
        found.contains(&("nul_byte".into(), "<stdin>:—".into())),
        "{found:?}"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("s3cr3t"));

    let parsed = r#"{"schema_version":1,"file":".env","entries":[
        {"key":"DEBUG","line":1,"value":"True"},
        {"key":"PORT","line":2,"value":"8080"},
        {"key":"API_KEY","line":3,"value":"sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9"}],"problems":[]}"#;
    let out = validate(&d, parsed, &["--input-format", "json", "--format", "json"]);
    assert_eq!(findings(&out), [("boolean_trap".into(), "<stdin>".into())]);
}

/// The format is explicit (D3): JSON piped without `--input-format json` is
/// read as a `.env`, and fails as one.
#[test]
fn b2_json_is_not_guessed() {
    let out = validate(&project(), r#"{"PORT":"8080"}"#, &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn b2_json_errors_name_keys_never_values() {
    let d = project();
    let no_values = r#"{"schema_version":1,"entries":[{"key":"PORT","line":1}]}"#;
    let out = validate(&d, no_values, &["--input-format", "json"]);
    assert!(
        stderr(&out)
            .contains("PORT has no value: write it with `evnx parse --format json --values`"),
        "{}",
        stderr(&out)
    );
    let out = validate(&d, r#"{"PORT":8080}"#, &["--input-format", "json"]);
    assert!(
        stderr(&out).contains("PORT's value is not a string"),
        "{}",
        stderr(&out)
    );
    let out = validate(&d, r#"["s3cr3t"]"#, &["--input-format", "json"]);
    assert!(stderr(&out).contains("expected an object of variables"));
    assert!(!stderr(&out).contains("s3cr3t"));
}

#[test]
fn b2_fix_and_an_empty_pipe_are_refused() {
    let d = project();
    let out = validate(&d, "DEBUG=True\n", &["--fix"]);
    assert!(
        stderr(&out).contains("--fix rewrites a .env file, and this input is not one"),
        "{}",
        stderr(&out)
    );
    let out = validate(&d, "", &[]);
    assert!(
        stderr(&out).contains("nothing was piped into `evnx validate --env -`"),
        "{}",
        stderr(&out)
    );
    assert_ne!(out.status.code(), Some(0));
}
