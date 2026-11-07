//! Name-aware rule labelling (C10, RM-92, CV-2): Clerk and UploadThing reuse
//! Stripe's `sk_live_` prefix, so the variable name decides whose key it is.
//!
//! ⚠️ Secret-shaped values are assembled at run time, so push protection never
//! sees one in the source.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn key() -> String {
    ["sk_live_", "6kmbhJgs1nKKT5fAo1N92rDz3dgFDhqy"].concat()
}

/// `(rule_id, line)` of every finding in a project holding `files`.
fn ids(files: &[(&str, String)]) -> Vec<(String, u64)> {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        fs::write(d.path().join(name), body).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", "."])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let doc: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rule_id"].as_str().unwrap().to_string(),
                f["line"].as_u64().unwrap_or(0),
            )
        })
        .collect()
}

fn only(files: &[(&str, String)]) -> String {
    let found = ids(files);
    assert_eq!(found.len(), 1, "exactly one finding, no double: {found:?}");
    found[0].0.clone()
}

#[test]
fn a_clerk_name_reports_clerk() {
    assert_eq!(
        only(&[(".env", format!("CLERK_SECRET_KEY={}\n", key()))]),
        "clerk-secret-key"
    );
}

#[test]
fn an_uploadthing_name_reports_uploadthing() {
    assert_eq!(
        only(&[(".env", format!("UPLOADTHING_SECRET={}\n", key()))]),
        "uploadthing-secret-key"
    );
}

#[test]
fn stripe_and_unknown_names_stay_stripe() {
    for name in ["STRIPE_SECRET_KEY", "FOO"] {
        assert_eq!(
            only(&[(".env", format!("{name}={}\n", key()))]),
            "stripe-live-secret-key",
            "{name}"
        );
    }
}

#[test]
fn plain_text_has_no_name_so_stripe_wins() {
    assert_eq!(
        only(&[("notes.txt", format!("CLERK_SECRET_KEY={}\n", key()))]),
        "stripe-live-secret-key"
    );
}

#[test]
fn a_yaml_leaf_is_labelled_by_its_key() {
    assert_eq!(
        only(&[(
            "config.yml",
            format!("auth:\n  clerk_secret_key: '{}'\n", key())
        )]),
        "clerk-secret-key"
    );
    assert_eq!(
        only(&[(
            "config.yml",
            format!("uploads:\n  UPLOADTHING_SECRET: '{}'\n", key())
        )]),
        "uploadthing-secret-key"
    );
    assert_eq!(
        only(&[(
            "config.yml",
            format!("billing:\n  stripe_key: '{}'\n", key())
        )]),
        "stripe-live-secret-key"
    );
}

#[test]
fn a_user_rule_with_names_fires_only_under_those_names() {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join(".evnx/rules")).unwrap();
    fs::write(
        d.path().join(".evnx/rules/acme.toml"),
        "version = 1\n[[rules]]\nid = \"acme-key\"\nname = \"Acme Key\"\nregex = 'acme_[a-z0-9]{20}'\nnames = ['^ACME_KEY$']\nseverity = \"high\"\n",
    )
    .unwrap();
    fs::write(
        d.path().join(".env"),
        "ACME_KEY=acme_k3x9q2m7v1zpa8w4n6r5\nOTHER=acme_k3x9q2m7v1zpa8w4n6r5\n",
    )
    .unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "--format", "json", "."])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let hits: Vec<_> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule_id"] == "acme-key")
        .collect();
    assert_eq!(hits.len(), 1, "{doc}");
}
