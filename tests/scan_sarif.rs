//! `evnx scan --format sarif` against the official SARIF 2.1.0 schema
//! (T-SCAN-62; decided 2026-10-11: vendored schema plus a validator).
//!
//! The schema is OASIS's own, vendored unchanged as
//! `tests/schemas/sarif-schema-2.1.0.json` from
//! `https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json`
//! (sha256 `c3b4bb2d…2682e`). GitHub code scanning rejects an upload that does
//! not validate, and nothing here checked it before: the golden files lock the
//! bytes, not the contract.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn validator() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("schemas/sarif-schema-2.1.0.json")).unwrap();
    jsonschema::draft4::new(&schema).expect("the vendored SARIF schema compiles")
}

fn sarif_for(files: &[(&str, &str)]) -> serde_json::Value {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--format", "sarif", "--exit-zero"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"))
}

#[track_caller]
fn assert_valid(doc: &serde_json::Value) {
    let errors: Vec<String> = validator()
        .iter_errors(doc)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "not valid SARIF 2.1.0:\n{}\n{doc:#}",
        errors.join("\n")
    );
}

#[test]
fn t_scan_62_a_clean_run_is_valid_sarif() {
    let doc = sarif_for(&[(".env", "APP_NAME=demo\n")]);
    assert!(doc["runs"][0]["results"].as_array().unwrap().is_empty());
    assert_valid(&doc);
}

#[test]
fn t_scan_62_one_finding_is_valid_sarif() {
    let doc = sarif_for(&[(".env", "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n")]);
    assert_eq!(doc["runs"][0]["results"].as_array().unwrap().len(), 1);
    assert_valid(&doc);
}

/// Every kind of finding: provider formats with and without a revoke link,
/// a connection string, a name-only guess, a public prefix, a private key in
/// a file of its own, and an entropy finding in source.
#[test]
fn t_scan_62_many_findings_are_valid_sarif() {
    let doc = sarif_for(&[
        (
            ".env",
            "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n\
             STRIPE_SECRET_KEY=sk_live_51H1234567890abcdefghijk\n\
             DATABASE_URL=postgres://admin:s3cr3tPassw0rd@db.internal:5432/app\n\
             DB_PASSWORD=correct-horse-battery\n\
             NEXT_PUBLIC_TOKEN=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIB\n",
        ),
        (
            "config/id_rsa.pem",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu\n-----END RSA PRIVATE KEY-----\n",
        ),
        ("src/app.py", "blob = \"q8R9s0T1u2V3w4X5y6Z7a8B9cDeFgHiJkL\"\n"),
    ]);
    assert!(
        doc["runs"][0]["results"].as_array().unwrap().len() >= 7,
        "{doc:#}"
    );
    assert_valid(&doc);
}

/// The validator is not vacuous: a document missing what 2.1.0 requires fails.
#[test]
fn t_scan_62_the_validator_rejects_a_broken_document() {
    let mut doc = sarif_for(&[(".env", "AWS_ACCESS_KEY_ID=AKIA4OZRMFJ3VREALKEY\n")]);
    doc["runs"][0]["tool"]["driver"]
        .as_object_mut()
        .unwrap()
        .remove("name");
    assert!(validator().iter_errors(&doc).next().is_some());
    doc["version"] = serde_json::json!("2.0.0");
    assert!(validator().iter_errors(&doc).next().is_some());
}
