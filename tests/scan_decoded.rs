//! One level of decoding (RM-71, D8): base64, hex and percent-encoded values
//! are decoded once and checked against the format rules only.

use assert_cmd::cargo::cargo_bin_cmd;
use base64::Engine as _;
use std::fs;
use tempfile::TempDir;

fn scan(name: &str, text: &str) -> (i32, serde_json::Value, String) {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(name), text).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", name])
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (
        out.status.code().unwrap_or(-1),
        doc,
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Findings that came from decoding, or that a format rule made: what RM-71
/// adds. The entropy fallback may still read the encoded token as written.
fn from_rules(doc: &serde_json::Value) -> Vec<String> {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule_id"] != "high-entropy-string")
        .map(|f| f["rule_id"].as_str().unwrap().to_string())
        .collect()
}

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGHq")
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

#[test]
fn a_k8s_secret_data_value_is_decoded_and_the_finding_says_so() {
    let yaml = format!(
        "apiVersion: v1\nkind: Secret\ndata:\n  api_key: {}\n",
        b64(&stripe())
    );
    let (code, doc, raw) = scan("secret.yaml", &yaml);
    assert_eq!(code, 1, "{raw}");
    let f = &doc["findings"][0];
    assert_eq!(f["rule_id"], "stripe-live-secret-key");
    assert_eq!(f["encoded"], "base64");
    assert_eq!(f["location"].as_str().unwrap().split(':').nth(1), Some("4"));
    // One value, one finding: the name check on `api_key` is not a second one.
    assert_eq!(doc["findings"].as_array().unwrap().len(), 1, "{raw}");
    // Never the value, decoded or not.
    assert!(
        !raw.contains(&stripe()) && !raw.contains(&b64(&stripe())),
        "{raw}"
    );
}

#[test]
fn hex_and_percent_encoding_are_decoded_too() {
    let hex: String = stripe().bytes().map(|b| format!("{b:02x}")).collect();
    let (code, doc, _) = scan("run.sh", &format!("export BLOB={hex}\n"));
    assert_eq!(code, 1);
    assert_eq!(doc["findings"][0]["encoded"], "hex");

    let url = "postgres%3A%2F%2Fadmin%3AZq8mXv2LpR9tWc4%40db.internal.example.org%3A5432%2Fapp";
    let (code, doc, _) = scan(".env", &format!("DB={url}\n"));
    assert_eq!(code, 1);
    let f = &doc["findings"][0];
    assert_eq!(f["rule_id"], "connection-string-credentials");
    assert_eq!(f["encoded"], "url");
}

#[test]
fn a_value_as_written_has_no_encoded_field() {
    let (_, doc, _) = scan(".env", &format!("KEY={}\n", stripe()));
    assert!(doc["findings"][0].get("encoded").is_none());
}

#[test]
fn decoded_random_bytes_images_and_hashes_are_never_findings() {
    // D8: no entropy check on decoded text. Binary does not decode to text.
    let png = base64::engine::general_purpose::STANDARD.encode([
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0xde, 0xad, 0xbe, 0xef,
        0x13, 0x37, 0x42, 0x99,
    ]);
    let text =
        format!("# {png}\n# e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n");
    let (_, doc, raw) = scan("notes.md", &text);
    assert!(from_rules(&doc).is_empty(), "{raw}");
    assert!(
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f.get("encoded").is_none())
    );
}

#[test]
fn decoded_text_without_a_known_format_is_not_a_finding() {
    // D8: base64 of an ordinary sentence, or of a name=value pair with no
    // known prefix, is not reported: only format rules judge decoded text.
    let text = format!(
        "# {}\n# {}\n",
        b64("deploy notes for the staging cluster"),
        b64("password=hunter2hunter2")
    );
    let (_, doc, raw) = scan("notes.md", &text);
    assert!(from_rules(&doc).is_empty(), "{raw}");
}

#[test]
fn only_one_level_is_decoded() {
    let twice = b64(&b64(&stripe()));
    let (_, doc, raw) = scan("notes.md", &format!("# {twice}\n"));
    assert!(from_rules(&doc).is_empty(), "{raw}");
}

#[test]
fn a_value_over_the_cap_is_not_decoded() {
    let long = b64(&format!("{} {}", stripe(), "x".repeat(4000)));
    assert!(long.len() > 4096);
    let (_, doc, raw) = scan("notes.md", &format!("# {long}\n"));
    assert!(from_rules(&doc).is_empty(), "{raw}");
}

#[test]
fn an_allow_marker_still_allows_a_decoded_finding() {
    let line = format!(
        "KEY={}  # evnx:allow stripe-live-secret-key reason=used-in-tests\n",
        b64(&stripe())
    );
    let (code, doc, raw) = scan(".env", &line);
    assert_eq!(code, 0, "{raw}");
    assert_eq!(doc["allowed"][0]["encoded"], "base64");
}
