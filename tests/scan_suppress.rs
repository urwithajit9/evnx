//! `evnx:allow` markers and the baseline (C5, RM-54; S8, S9).
//!
//! ⚠️ The secret-shaped value is assembled at run time, and the baseline is
//! grepped for it: the baseline must never hold a value.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGHq")
}

fn other_stripe() -> String {
    format!("sk_live_{}", "51Hxyz0987654321ZYXWVUTSq")
}

/// (exit code, JSON report, stderr)
fn scan(d: &TempDir, args: &[&str]) -> (i32, serde_json::Value, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json"])
        .args(args)
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (
        out.status.code().unwrap_or(-1),
        doc,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn project(env: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), env).unwrap();
    d
}

#[test]
fn a_marker_on_the_line_or_above_allows_its_rule_there() {
    let s = stripe();
    let d = project(&format!(
        "# evnx:allow stripe-live-secret-key — a revoked key kept for the docs\n\
         OLD_KEY={s}\n\
         SAME_LINE={s}x  # evnx:allow stripe-live-secret-key — fixture\n"
    ));
    let (code, doc, _) = scan(&d, &[".env"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["summary"]["total"], 0);
    assert_eq!(doc["summary"]["allowed"], 2);
    let a = &doc["allowed"][0];
    assert_eq!(a["status"], "allowed");
    assert_eq!(a["reason"], "a revoked key kept for the docs");
}

#[test]
fn a_marker_for_another_rule_or_a_bare_one_allows_nothing_and_says_so() {
    let s = stripe();
    let d = project(&format!(
        "# evnx:allow github-token — wrong rule\nA={s}\n# evnx:allow\nB={s}y\n"
    ));
    let (code, doc, err) = scan(&d, &[".env"]);
    assert_eq!(code, 1);
    assert_eq!(doc["summary"]["total"], 2);
    assert!(
        err.contains("evnx:allow github-token does not match"),
        "{err}"
    );
    assert!(err.contains("a bare evnx:allow allows nothing"), "{err}");
}

#[test]
fn a_marker_without_a_reason_works_and_is_warned_about() {
    let d = project(&format!(
        "A={}  # evnx:allow stripe-live-secret-key\n",
        stripe()
    ));
    let (code, doc, err) = scan(&d, &[".env"]);
    assert_eq!((code, doc["summary"]["allowed"].clone()), (0, 1.into()));
    assert!(err.contains("has no reason"), "{err}");
}

#[test]
fn a_baseline_survives_a_line_above_and_a_changed_value_is_new() {
    let s = stripe();
    let d = project(&format!("KEY={s}\n"));
    let (code, _, _) = scan(&d, &[".env", "--update-baseline"]);
    assert_eq!(code, 0);
    let baseline = fs::read_to_string(d.path().join(".evnx/baseline.json")).unwrap();
    // ⛔ S9: the baseline holds no value, not even a fragment of one.
    assert!(!baseline.contains(&s), "{baseline}");
    assert!(!baseline.contains("51Habc"), "{baseline}");
    let parsed: serde_json::Value = serde_json::from_str(&baseline).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["findings"][0]["rule"], "stripe-live-secret-key");
    assert_eq!(parsed["findings"][0]["file"], ".env");

    let (code, doc, _) = scan(&d, &[".env"]);
    assert_eq!((code, doc["summary"]["known"].clone()), (0, 1.into()));

    // A line inserted above: still known.
    fs::write(d.path().join(".env"), format!("NEW=1\n\nKEY={s}\n")).unwrap();
    let (code, doc, _) = scan(&d, &[".env"]);
    assert_eq!(
        (code, doc["summary"]["known"].clone()),
        (0, 1.into()),
        "{doc}"
    );
    assert_eq!(doc["known"][0]["status"], "known");

    // The value changed: new.
    fs::write(
        d.path().join(".env"),
        format!("NEW=1\n\nKEY={}\n", other_stripe()),
    )
    .unwrap();
    let (code, doc, _) = scan(&d, &[".env"]);
    assert_eq!(code, 1);
    assert_eq!(doc["summary"]["total"], 1);
    assert_eq!(doc["findings"][0]["status"], "new");
}

#[test]
fn a_named_baseline_that_does_not_exist_is_an_error() {
    let d = project("A=1\n");
    let (code, _, err) = scan(&d, &[".env", "--baseline", "nope.json"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("nope.json"), "{err}");
}

#[test]
fn rewriting_the_baseline_keeps_what_was_accepted_and_adds_the_new() {
    let d = project(&format!("A={}\n", stripe()));
    scan(&d, &[".env", "--update-baseline"]);
    fs::write(
        d.path().join(".env"),
        format!("A={}\nB={}\n", stripe(), other_stripe()),
    )
    .unwrap();
    let (code, _, _) = scan(&d, &[".env", "--update-baseline"]);
    assert_eq!(code, 0);
    let b: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(d.path().join(".evnx/baseline.json")).unwrap())
            .unwrap();
    assert_eq!(b["findings"].as_array().unwrap().len(), 2);
    let (code, doc, _) = scan(&d, &[".env"]);
    assert_eq!((code, doc["summary"]["known"].clone()), (0, 2.into()));
}
