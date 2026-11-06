//! `evnx scan` on YAML, JSON and TOML (C8, v0.13.0).
//!
//! Before C8 a structured file was read as text: only the regex rules and the
//! 32-character entropy fallback ran, so `password: <a real password>` in
//! `config.yml` scanned clean. Each string value is now checked by its key and
//! its value, alongside the per-line rules.
//!
//! ⚠️ Secret-shaped values are assembled at run time, so push protection never
//! sees one in the source.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn project(files: &[(&str, &str)]) -> TempDir {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    d
}

fn scan(d: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json"])
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{e}\n{text}\n{}", String::from_utf8_lossy(&out.stderr)))
}

/// A password no rule recognises: only its key says it is a secret.
fn password() -> String {
    format!("correct-{}", "horse-battery-staple")
}

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGH")
}

#[test]
fn a_password_in_yaml_json_and_toml_is_reported_with_its_key_and_line() {
    let p = password();
    let d = project(&[
        (
            "config.yml",
            &format!("app:\n  name: demo\n  db:\n    password: {p}\n"),
        ),
        (
            "settings.json",
            &format!(
                "{{\n  \"name\": \"demo\",\n  \"api\": {{\n    \"password\": \"{p}\"\n  }}\n}}\n"
            ),
        ),
        (
            "app.toml",
            &format!("name = \"demo\"\n\n[database]\npassword = \"{p}\"\n"),
        ),
    ]);
    let doc = scan(&d, &[]);
    let locations: Vec<&str> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["location"].as_str().unwrap())
        .collect();
    for want in [
        "./app.toml:4 (database.password)",
        "./config.yml:4 (app.db.password)",
        "./settings.json:4 (api.password)",
    ] {
        assert!(locations.contains(&want), "{want} not in {locations:?}");
    }
    assert_eq!(doc["summary"]["total"], 3);
    for f in doc["files"].as_array().unwrap() {
        assert_eq!(f["read_as"], "structured", "{f}");
    }
}

#[test]
fn references_and_non_strings_are_not_reported() {
    let d = project(&[(
        "config.yml",
        "secret: ${DB_SECRET}\ntoken: null\napi_key: 12345678901234567890\n",
    )]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 0);
}

#[test]
fn a_helm_template_falls_back_to_text_with_no_error() {
    let s = stripe();
    let d = project(&[(
        "templates/secret.yaml",
        // `{{- if }}` control lines are what make a chart invalid YAML; a
        // lone `{{ .Values.x }}` happens to parse, as a flow mapping.
        &format!(
            "{{{{- if .Values.enabled }}}}\napiVersion: v1\nkind: Secret\n\
             stringData:\n  password: {{{{ .Values.db.password }}}}\n  stripe: {s}\n{{{{- end }}}}\n"
        ),
    )]);
    let doc = scan(&d, &[]);
    let file = &doc["files"][0];
    assert_eq!(file["read_as"], "text", "{file}");
    assert_eq!(file["note"], "did not parse as yaml");
    // The line rules still ran.
    assert_eq!(doc["findings"][0]["rule_id"], "stripe-live-secret-key");
}

#[test]
fn a_value_in_a_yaml_comment_is_still_found() {
    let s = stripe();
    let d = project(&[("config.yml", &format!("name: demo\n# old key: {s}\n"))]);
    let doc = scan(&d, &[]);
    assert_eq!(doc["summary"]["total"], 1);
    assert_eq!(doc["findings"][0]["location"], "./config.yml:2");
}

#[test]
fn one_secret_found_by_both_paths_is_one_finding_with_its_key() {
    let s = stripe();
    let d = project(&[("config.yml", &format!("payments:\n  stripe_key: {s}\n"))]);
    let doc = scan(&d, &[]);
    assert_eq!(doc["summary"]["total"], 1, "{doc}");
    let f = &doc["findings"][0];
    assert_eq!(f["rule_id"], "stripe-live-secret-key");
    assert_eq!(f["location"], "./config.yml:2 (payments.stripe_key)");
}

/// A long sentence or a script in a structured value is not a secret, however
/// varied its characters: entropy judges tokens, as on the text path.
#[test]
fn prose_and_scripts_are_not_judged_by_entropy_as_a_whole() {
    let d = project(&[(
        "changelog.json",
        // Entropy 5.15 as one string, over the 4.8 threshold: a release note
        // like the ones that were reported on a real repository.
        "{\"text\": \"Reads YAML, JSON & TOML now: keys (db.password), lists [0..9], tabs; fixes #812/#820 @ v0.13!\"}\n",
    )]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 0);
    // A value with spaces under a secret's name is still reported by name.
    let d = project(&[("config.yml", "password: \"correct horse battery staple\"\n")]);
    assert_eq!(scan(&d, &[])["summary"]["total"], 1);
}
