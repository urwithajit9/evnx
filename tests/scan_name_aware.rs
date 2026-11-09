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

/// F14-3: a rule's `names` beats the built-in rule that matches the same
/// value in every file format, not only in `.env`. `secret_key` under a YAML
/// or JSON mapping reported Stripe, because the text pass (which has no key)
/// found Stripe's rule and its remediation URL outranked the named rule.
///
/// `names` are regular expressions tested against the leaf key as written, in
/// every format: `.env` is no exception, so a rule that means both
/// `SECRET_KEY` and `secret_key` says so (here, or with `(?i)`).
#[test]
fn f14_3_names_win_over_a_builtin_in_every_format() {
    let value = key();
    let cases: [(&str, String); 6] = [
        (".env", format!("SECRET_KEY={value}\n")),
        ("flat.yaml", format!("secret_key: {value}\n")),
        ("nested.yaml", format!("app:\n  secret_key: {value}\n")),
        (
            "deep.yml",
            format!("a:\n  b:\n    - secret_key: '{value}'\n"),
        ),
        ("flat.json", format!("{{\"secret_key\": \"{value}\"}}\n")),
        (
            "nested.json",
            format!("{{\"app\": {{\"secret_key\": \"{value}\"}}}}\n"),
        ),
    ];
    for (file, body) in cases {
        let d = TempDir::new().unwrap();
        fs::write(
            d.path().join(".evnx.toml"),
            "[[scan.patterns]]\nid = \"acme-app-secret\"\nname = \"Acme App Secret\"\n\
             regex = 'sk_live_[A-Za-z0-9]{16,}'\nnames = ['^(?:SECRET_KEY|secret_key)$']\n\
             severity = \"critical\"\n",
        )
        .unwrap();
        fs::write(d.path().join(file), body).unwrap();
        let out = cargo_bin_cmd!("evnx")
            .current_dir(d.path())
            .env("NO_COLOR", "1")
            .args(["scan", "--format", "json", file])
            .output()
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{file}: {e}\n{}", String::from_utf8_lossy(&out.stdout)));
        let rules: Vec<&str> = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["rule_id"].as_str().unwrap())
            .collect();
        assert_eq!(rules, ["acme-app-secret"], "{file}: {doc}");
    }
}

/// F14-3: the same rule stays out of the way under another name, in a nested
/// mapping too: Stripe's rule still reports it.
#[test]
fn f14_3_a_nested_key_the_names_do_not_match_stays_builtin() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".evnx.toml"),
        "[[scan.patterns]]\nid = \"acme-app-secret\"\nname = \"Acme App Secret\"\n\
         regex = 'sk_live_[A-Za-z0-9]{16,}'\nnames = ['^secret_key$']\nseverity = \"critical\"\n",
    )
    .unwrap();
    fs::write(
        d.path().join("nested.yaml"),
        format!("app:\n  other_key: {}\n", key()),
    )
    .unwrap();
    assert_eq!(
        only_in(&d, "nested.yaml"),
        "stripe-live-secret-key",
        "names must not widen"
    );
}

fn only_in(d: &TempDir, file: &str) -> String {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", file])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let f = doc["findings"].as_array().unwrap();
    assert_eq!(f.len(), 1, "{doc}");
    f[0]["rule_id"].as_str().unwrap().to_string()
}
