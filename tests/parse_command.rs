//! `evnx parse` (RM-36, RM-88; D1, D2): line, key and flags, never a value
//! unless `--values` is passed; every problem listed, exit 1 when there is one.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

const ENV: &str =
    "PORT=8080\nAPI_KEY=\nSECRET_KEY=changeme\nTOKEN=ghp_s3cr3tS3CR3Ts3cr3tS3CR3Ts3cr3tS3CR\n";

fn parse(dir: &TempDir, stdin: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut cmd = cargo_bin_cmd!("evnx");
    cmd.current_dir(dir.path())
        .env("NO_COLOR", "1")
        .arg("parse")
        .args(args);
    if let Some(s) = stdin {
        cmd.write_stdin(s);
    }
    cmd.output().unwrap()
}

fn dir_with_env(content: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), content).unwrap();
    d
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn rm88_the_listing_names_keys_and_flags_never_values() {
    let d = dir_with_env(ENV);
    let out = parse(&d, None, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "   1  PORT\n   2  API_KEY     empty,secret\n   3  SECRET_KEY  placeholder,secret\n   4  TOKEN       secret\n"
    );
    let json = text(&parse(&d, None, &["--format", "json"]).stdout);
    for value in ["8080", "changeme", "s3cr3t"] {
        assert!(!json.contains(value), "{value} in {json}");
    }
}

#[test]
fn rm88_values_appear_only_when_asked() {
    let d = dir_with_env(ENV);
    let out = parse(&d, None, &["--format", "json", "--values"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["entries"][0]["value"], "8080");
    assert_eq!(json["entries"][3]["key"], "TOKEN");
    // `--yes` means nothing without `--values`.
    assert_eq!(parse(&d, None, &["--yes"]).status.code(), Some(2));
}

#[test]
fn rm36_stdin_and_every_problem_with_exit_1() {
    let d = TempDir::new().unwrap();
    let out = parse(
        &d,
        Some("A=1\nmy.key=2\nB=\"x\n"),
        &["--env", "-", "--format", "json"],
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["file"], "<stdin>");
    assert_eq!(json["entries"][0]["key"], "A");
    let lines: Vec<u64> = json["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines, [2, 3]);
}

/// `parse --format json --values` is what `validate --input-format json` reads.
#[test]
fn b2_parse_output_feeds_validate() {
    let d = dir_with_env("DEBUG=True\nPORT=8080\n");
    fs::write(d.path().join(".env.example"), "DEBUG=\nPORT=\n").unwrap();
    let parsed = parse(&d, None, &["--format", "json", "--values"]).stdout;
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "validate",
            "--env",
            "-",
            "--input-format",
            "json",
            "--format",
            "json",
        ])
        .write_stdin(parsed)
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let types: Vec<&str> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["boolean_trap"]);
}

#[test]
fn rm36_a_missing_file_says_so() {
    let out = parse(&TempDir::new().unwrap(), None, &[]);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).contains(".env does not exist"),
        "{}",
        text(&out.stderr)
    );
}
