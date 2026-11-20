//! WP-9b: every `--format json` output starts with the same `run` header.
//!
//! Walks `evnx commands --json`: each command with a `--format` argument whose
//! values include `json` must be classified below as local (run here, in a
//! small fixture, with `--json`) or network (skipped here; the cloud_* and
//! platform_* mock-server tests cover it). A new JSON command that is in
//! neither list fails this test, so the header cannot be forgotten.

use assert_cmd::Command;
use serde_json::Value;

#[allow(deprecated)]
fn evnx(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("evnx").unwrap();
    c.current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("cfg"))
        .env("EVNX_CONFIG_DIR", dir.join("cfg"))
        .env("NO_COLOR", "1")
        .env_remove("EVNX_TOKEN")
        .env_remove("EVNX_KEY_BUNDLE");
    c
}

/// Commands that need a server (or a network at all): covered by the mock tests
/// in tests/cloud_*.rs and tests/platform_*.rs.
const NETWORK: &[&str] = &[
    "auth key-bundle",
    "auth token create",
    "auth sessions list",
    "auth devices list",
    "org list",
    "org members",
    "org invites",
    "org billing",
    "vault list",
    "vault members",
    "cloud history",
    "cloud diff",
    "cloud audit",
    "platform diff",
    "platform push",
    "update", // `update --check` asks GitHub; `update --list` is run below
];

/// Local, but exempt: the top level is an array, which has no place for `run`
/// (`core::json::emit_unwrapped`).
const NO_HEADER: &[&str] = &["scan rules list"];

/// Local commands: path -> the arguments that make it run in the fixture.
fn local_args(path: &str) -> Option<Vec<&'static str>> {
    Some(match path {
        "init" => vec!["init", "--dry-run"],
        "add" | "add custom" => vec!["add", "custom", "NEWVAR", "--dry-run"],
        "add service" => vec!["add", "service", "postgres", "--dry-run"],
        "add import" => vec!["add", "import", "vars.json", "--dry-run"],
        "add framework" => vec!["add", "framework", "django", "--dry-run"],
        "add blueprint" => vec!["add", "blueprint", "t3_modern", "--dry-run"],
        "remove" => vec!["remove", "--check", "B"],
        "set" => vec!["set", "B=2", "--dry-run"],
        "get" => vec!["get", "A"],
        "validate" => vec!["validate"],
        "scan" => vec!["scan"],
        "diff" => vec!["diff"],
        "sync" => vec!["sync"],
        "align" => vec!["align"],
        "doctor" => vec!["doctor"],
        "spec check" => vec!["spec", "check"],
        "history" => vec!["history"],
        "parse" => vec!["parse", "--env", ".env"],
        "commands" => vec!["commands"],
        // Reads local state only; no request.
        "auth status" => vec!["auth", "status"],
        "cloud status" => vec!["cloud", "status"],
        _ => return None,
    })
}

#[test]
fn every_json_output_carries_the_run_header() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\nB=hello\n").unwrap();
    std::fs::write(dir.path().join(".env.example"), "A=\nB=\nC=\n").unwrap();
    std::fs::write(dir.path().join("vars.json"), "{\"NEW_ONE\":\"1\"}").unwrap();

    let out = evnx(dir.path())
        .args(["commands", "--json"])
        .output()
        .unwrap();
    let surface: Value = serde_json::from_slice(&out.stdout).unwrap();
    let features = surface["features"].clone();
    assert_eq!(surface["run"]["features"], features);

    let mut ran = 0;
    for cmd in surface["commands"].as_array().unwrap() {
        let path = cmd["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let has_json = cmd["args"].as_array().is_some_and(|args| {
            args.iter().any(|a| {
                a["long"] == "format"
                    && a["possible_values"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|x| x == "json"))
            })
        });
        if !has_json || NETWORK.contains(&path.as_str()) || NO_HEADER.contains(&path.as_str()) {
            continue;
        }
        let args = local_args(&path).unwrap_or_else(|| {
            panic!("`evnx {path}` has --format json: add it to `local_args` or `NETWORK`")
        });
        let out = evnx(dir.path()).args(&args).arg("--json").output().unwrap();
        // Exit 1 is a finding (validate, diff, doctor, remove --check); 2 is a failure.
        assert_ne!(
            out.status.code(),
            Some(2),
            "evnx {path}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Exactly one JSON document: parsing the whole of stdout proves it.
        let doc: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("evnx {path}: stdout is not one JSON document: {e}"));
        let run = &doc["run"];
        assert_eq!(run["tool"], "evnx", "{path}");
        assert_eq!(run["version"], env!("CARGO_PKG_VERSION"), "{path}");
        assert_eq!(run["features"], features, "{path}");
        assert!(run["schema_version"].as_u64().is_some(), "{path}");
        assert!(
            run.get("ruleset").is_some(),
            "{path}: ruleset is null, not absent"
        );
        assert!(
            run.get("timestamp").is_none(),
            "{path}: no timestamp in run"
        );
        ran += 1;
    }
    assert!(ran >= 15, "walked too few commands: {ran}");
}

#[test]
fn update_list_carries_the_header() {
    let dir = tempfile::tempdir().unwrap();
    let out = evnx(dir.path())
        .args(["update", "--list", "--json"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["run"]["tool"], "evnx");
    assert!(doc["copies"].is_array());
}

#[test]
fn scan_names_its_rule_set() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    let out = evnx(dir.path()).args(["scan", "--json"]).output().unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        doc["run"]["ruleset"],
        format!("builtin@{}", env!("CARGO_PKG_VERSION"))
    );
    // The command's own deprecated key stays beside it.
    assert_eq!(doc["schema_version"], 2);
}
