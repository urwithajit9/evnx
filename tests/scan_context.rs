//! Usage hints and test paths (SCAN-CONTEXT, D9).
//!
//! A hint moves the level a finding is shown at by one, for name, entropy and
//! medium findings only, and never changes the decision: the exit code, the
//! `--confidence` floor and the summary counts use the level before it.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

const TOKEN: &str = "Zk8Qm2Xv5Lr9Tn4Wc7Hb1Jd6Fs3Ga0Pe8Yu2Ni";

fn scan(files: &[(&str, &str)], args: &[&str]) -> (i32, serde_json::Value) {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", "."])
        .args(args)
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), doc)
}

fn only(doc: &serde_json::Value) -> &serde_json::Value {
    let f = doc["findings"].as_array().unwrap();
    assert_eq!(f.len(), 1, "{doc}");
    &f[0]
}

#[test]
fn a_value_in_an_auth_header_is_raised_one_level() {
    let line = format!("curl -H \"Authorization: Bearer {TOKEN}\" https://api.example.com/v1\n");
    let (code, doc) = scan(&[("call.sh", &line)], &[]);
    assert_eq!(code, 1, "{doc}");
    let f = only(&doc);
    assert_eq!(f["rule_id"], "high-entropy-string");
    assert_eq!(f["confidence"], "medium");
    assert_eq!(f["confidence_before_context"], "low");
    assert_eq!(f["context"][0], "usage");
}

#[test]
fn the_same_value_without_a_usage_is_unchanged() {
    let (_, doc) = scan(&[("notes.sh", &format!("echo {TOKEN}\n"))], &[]);
    let f = only(&doc);
    assert_eq!(f["confidence"], "low");
    assert!(f.get("context").is_none() && f.get("confidence_before_context").is_none());
}

#[test]
fn a_test_path_lowers_a_name_finding_but_never_drops_it() {
    let env = format!("API_PASSWORD={TOKEN}\n");
    let (code, doc) = scan(&[("tests/fixtures/.env", &env)], &[]);
    assert_eq!(code, 1, "{doc}");
    let f = only(&doc);
    assert_eq!(f["rule_id"], "sensitive-config-key");
    assert_eq!(f["confidence"], "medium");
    assert_eq!(f["confidence_before_context"], "high");
    assert_eq!(f["context"][0], "test-path");
    // Outside a test folder the same line is high.
    let (_, doc) = scan(&[(".env", &env)], &[]);
    assert_eq!(only(&doc)["confidence"], "high");
}

#[test]
fn the_exit_code_floor_and_counts_use_the_level_before_the_hint() {
    let env = format!("API_PASSWORD={TOKEN}\n");
    // High before, medium shown: `--confidence high` still reports it and
    // still fails, and the summary counts it as high.
    let (code, doc) = scan(&[("tests/.env", &env)], &["--confidence", "high"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(only(&doc)["confidence"], "medium");
    assert_eq!(doc["summary"]["high"], 1);
    assert_eq!(doc["high_confidence"], 1);
}

#[test]
fn a_high_fixed_prefix_rule_is_never_lowered_in_a_test_folder() {
    let key = format!("sk_live_{}", "51Habc1234567890ABCDEFGHq");
    let (code, doc) = scan(&[("tests/fixtures/.env", &format!("K={key}\n"))], &[]);
    assert_eq!(code, 1);
    let f = only(&doc);
    assert_eq!(f["rule_id"], "stripe-live-secret-key");
    assert_eq!(f["confidence"], "high");
    assert!(f.get("context").is_none());
}

#[test]
fn a_test_file_name_alone_is_not_a_test_path() {
    let env = format!("API_PASSWORD={TOKEN}\n");
    let (_, doc) = scan(&[("test.env", &env)], &[]);
    assert_eq!(only(&doc)["confidence"], "high");
}
