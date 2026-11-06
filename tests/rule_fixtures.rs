//! Every built-in rule against its fixtures (C3, RM-51/52).
//!
//! `tests/fixtures/rules/<id>.toml` holds, for each rule in `rules.toml`:
//! `positive` values it must report, `near_miss` values it must not, and the
//! `expect_severity` it must carry. Values are stored as parts and joined here,
//! so no fixture file holds a whole token for push protection to reject.
//!
//! ⚠️ A rule with no fixture file fails this suite. That is what keeps it
//! complete as rules are added.

use evnx::core::secretscan::engine::{self, Context, Format, Kind};
use evnx::core::secretscan::rules::builtin_rules;
use evnx::core::secretscan::{DetectorRegistry, PatternSet};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    expect_severity: String,
    positive: Vec<Vec<String>>,
    near_miss: Vec<Vec<String>>,
}

fn fixtures() -> BTreeMap<String, Fixture> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules");
    std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| {
            let id = p.file_stem().unwrap().to_string_lossy().into_owned();
            let f: Fixture = toml::from_str(&std::fs::read_to_string(&p).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            (id, f)
        })
        .collect()
}

#[test]
fn every_rule_has_a_fixture_file_and_every_file_a_rule() {
    let ids: Vec<String> = builtin_rules().into_iter().filter_map(|r| r.id).collect();
    let files = fixtures();
    let missing: Vec<&String> = ids.iter().filter(|id| !files.contains_key(*id)).collect();
    assert!(
        missing.is_empty(),
        "rules with no tests/fixtures/rules/<id>.toml: {missing:?}"
    );
    let orphans: Vec<&String> = files.keys().filter(|k| !ids.contains(k)).collect();
    assert!(orphans.is_empty(), "fixture files for no rule: {orphans:?}");
}

/// Each positive is reported by its own rule, and that rule wins: no other
/// rule claims the value. Each near miss is not reported by the rule. The
/// severity is the one the fixture expects.
#[test]
fn every_rule_reports_its_positives_and_not_its_near_misses() {
    let rules = builtin_rules();
    let set = PatternSet::compile(&rules).unwrap();
    let mut failures = Vec::new();
    for (id, f) in fixtures() {
        let rule = rules.iter().find(|r| r.id.as_deref() == Some(&id)).unwrap();
        if rule.severity.as_deref() != Some(f.expect_severity.as_str()) {
            failures.push(format!(
                "{id}: severity {:?}, fixture expects {}",
                rule.severity, f.expect_severity
            ));
        }
        for parts in &f.positive {
            let value = parts.concat();
            let found = set.find_all(&value);
            match PatternSet::strongest_of(found.clone()) {
                Some(m) if m.id == id => {}
                Some(m) => failures.push(format!("{id}: a positive is claimed by {}", m.id)),
                None => failures.push(format!(
                    "{id}: a positive is not reported ({} chars)",
                    value.len()
                )),
            }
        }
        for parts in &f.near_miss {
            if set.find_all(&parts.concat()).iter().any(|m| m.id == id) {
                failures.push(format!("{id}: a near miss is reported: {parts:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The same positives through the engine, as a `.env` entry and as a YAML
/// value (C8): each file kind reports the rule's id.
#[test]
fn every_positive_is_found_in_a_dotenv_and_in_yaml() {
    let registry =
        DetectorRegistry::new().with_patterns(PatternSet::compile(&builtin_rules()).unwrap());
    let mut failures = Vec::new();
    for (id, f) in fixtures() {
        for parts in &f.positive {
            let value = parts.concat();
            for (kind, text) in [
                (Kind::Dotenv, format!("VALUE_A={value}\n")),
                (
                    Kind::Structured(Format::Yaml),
                    format!("value_a: '{value}'\n"),
                ),
            ] {
                let ctx = Context {
                    path: "fixture",
                    kind,
                    document: None,
                    leaves: None,
                    registry: &registry,
                };
                if !engine::scan_text(&text, &ctx)
                    .iter()
                    .any(|f| f.detection.rule_id == id)
                {
                    failures.push(format!("{id}: not found as {kind:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// C3: a PKCS#12 keystore is binary, so it is found by its content, not
/// read: critical, under any name. A DER certificate beside it is not.
#[test]
fn a_pkcs12_keystore_is_found_by_its_content() {
    use assert_cmd::cargo::cargo_bin_cmd;
    let d = tempfile::TempDir::new().unwrap();
    let mut p12 = vec![0x30, 0x82, 0x0a, 0x1b, 0x02, 0x01, 0x03, 0x30, 0x82, 0x09];
    p12.extend(std::iter::repeat_n(0xa5u8, 200));
    std::fs::write(d.path().join("client.p12"), &p12).unwrap();
    std::fs::write(d.path().join("renamed.bin"), &p12).unwrap();
    let mut cert = vec![0x30, 0x82, 0x03, 0x0b, 0x30, 0x82, 0x02, 0xf3];
    cert.extend(std::iter::repeat_n(0x5au8, 200));
    std::fs::write(d.path().join("server.der"), &cert).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "--format", "json", "."])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut found: Vec<(String, String)> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["location"].as_str().unwrap().to_string(),
                format!(
                    "{} {}",
                    f["rule_id"].as_str().unwrap(),
                    f["severity"].as_str().unwrap()
                ),
            )
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            (
                "./client.p12:1".to_string(),
                "pkcs12-keystore critical".to_string()
            ),
            (
                "./renamed.bin:1".to_string(),
                "pkcs12-keystore critical".to_string()
            ),
        ]
    );
    assert_eq!(out.status.code(), Some(1));
}
