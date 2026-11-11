//! `schema.json`'s `scan` section against the scanner (RM-91).
//!
//! `src/assets/schema.json` lists the variables `evnx init` writes. Its `scan`
//! section says, for the ones a scan treats as secret, how `scan` sees them:
//!
//! * a rule id: the rule that reports the variable's value;
//! * `"heuristic"`: no rule for it, only the generic name and entropy checks;
//! * `"public"`: behind a browser-exposed prefix (`PUBLIC_PREFIXES`), so inlined
//!   into the client bundle.
//!
//! Three files describe one thing: that section, `rules.toml` and the prefix
//! list. This holds them in step, so a renamed or removed rule, a variable
//! dropped from the catalogue or a prefix added to the list breaks a test here
//! instead of leaving `schema.json` saying something `scan` no longer does.
//!
//! The catalogue RM-61 describes (a target-format catalogue) is not needed: the
//! rule ids come from the rule definitions themselves.

use evnx::core::classify::is_sensitive_key;
use evnx::core::secretscan::engine::{self, Context, Kind};
use evnx::core::secretscan::rules::{builtin_rules, catalogue_ids, public_prefix, CHECK_IDS};
use evnx::core::secretscan::{DetectorRegistry, PatternSet};
use evnx::schema::loader::{scan_view, ScanView};
use std::collections::{BTreeMap, BTreeSet};

fn schema() -> serde_json::Value {
    serde_json::from_str(include_str!("../src/assets/schema.json")).unwrap()
}

/// Every variable any component declares, anywhere in the catalogue.
fn variables(v: &serde_json::Value, out: &mut BTreeSet<String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "vars" {
                    out.extend(
                        v.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|n| n.as_str().map(str::to_string)),
                    );
                } else if k != "scan" {
                    variables(v, out);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| variables(i, out)),
        _ => {}
    }
}

fn entries() -> BTreeMap<String, String> {
    schema()["scan"]
        .as_object()
        .expect("schema.json has a `scan` section")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().expect("a string").to_string()))
        .collect()
}

#[derive(serde::Deserialize)]
struct Fixture {
    positive: Vec<Vec<String>>,
}

/// A value the rule reports: its fixture's first positive (parts, joined). The
/// two checks that are not rules get a value built here.
fn sample(rule: &str) -> String {
    if rule == "aws-secret-access-key" {
        // 40 base64 characters, which that check reads by the variable's name.
        return ["Zx9Qm2LkP7wRt4Yv", "B8nC3dHs6JfG1aEu", "5XoI0pVq"].concat();
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rules")
        .join(format!("{rule}.toml"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{rule}: no fixture file to take a value from"));
    let f: Fixture = toml::from_str(&text).unwrap();
    f.positive[0].concat()
}

/// The rule ids a scan reports for `NAME=value` in a `.env`.
fn reported(name: &str, value: &str) -> Vec<String> {
    let registry =
        DetectorRegistry::new().with_patterns(PatternSet::compile(&builtin_rules()).unwrap());
    let ctx = Context {
        path: "f",
        kind: Kind::Dotenv,
        document: None,
        leaves: None,
        registry: &registry,
    };
    engine::scan_text(&format!("{name}={value}\n"), &ctx)
        .into_iter()
        .map(|f| f.detection.rule_id)
        .collect()
}

#[test]
fn the_section_exists_and_the_rest_of_the_file_is_unchanged_in_shape() {
    let v = schema();
    for key in [
        "languages",
        "services",
        "infrastructure",
        "stacks",
        "version",
    ] {
        assert!(v.get(key).is_some(), "schema.json lost `{key}`");
    }
    // The loader still reads the whole file, and reads the new section.
    assert!(evnx::schema::loader::schema().is_ok());
    assert_eq!(
        scan_view("STRIPE_SECRET_KEY"),
        Some(ScanView::Rule("stripe-live-secret-key"))
    );
    assert_eq!(scan_view("NEXTAUTH_SECRET"), Some(ScanView::Heuristic));
    assert_eq!(scan_view("VITE_APP_TITLE"), Some(ScanView::Public));
    assert_eq!(scan_view("PORT"), None);
}

/// Every variable the catalogue declares that `scan` would treat as a
/// credential has an entry, and every entry is for a variable that exists.
#[test]
fn every_secret_variable_has_an_entry_and_every_entry_a_variable() {
    let mut vars = BTreeSet::new();
    variables(&schema(), &mut vars);
    let entries = entries();

    let missing: Vec<&String> = vars
        .iter()
        .filter(|v| {
            (public_prefix(v).is_some() || is_sensitive_key(v)) && !entries.contains_key(*v)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "secret or public variables with no `scan` entry in schema.json: {missing:?}"
    );

    let orphans: Vec<&String> = entries.keys().filter(|k| !vars.contains(*k)).collect();
    assert!(
        orphans.is_empty(),
        "`scan` entries for variables no component declares: {orphans:?}"
    );
}

/// `public` is exactly the prefix list's answer, and nothing behind a prefix is
/// described any other way.
#[test]
fn public_means_a_listed_prefix_and_only_that() {
    for (name, how) in entries() {
        assert_eq!(
            how == "public",
            public_prefix(&name).is_some(),
            "{name}: `{how}` disagrees with PUBLIC_PREFIXES"
        );
    }
}

/// Every rule id an entry names is one `scan` can print, so a renamed or removed
/// rule breaks here.
#[test]
fn every_named_rule_exists() {
    let known: BTreeSet<String> = catalogue_ids().into_iter().collect();
    for (name, how) in entries() {
        if how == "heuristic" || how == "public" {
            continue;
        }
        assert!(
            known.contains(&how),
            "{name}: schema.json names the rule `{how}`, which scan does not have"
        );
    }
    // A rule is never named `heuristic` or `public`: those words are the other
    // two answers.
    for word in ["heuristic", "public"] {
        assert!(!known.contains(word), "a rule is called `{word}`");
    }
}

/// The claims are true of the scanner, not only of the file: a rule's variable
/// reports that rule for the rule's own sample, and a `heuristic` one is
/// reported by the generic checks alone.
#[test]
fn the_scanner_says_what_the_section_says() {
    let mut failures = Vec::new();
    for (name, how) in entries() {
        match how.as_str() {
            "public" => {}
            "heuristic" => {
                // Any long random value under the name.
                let got = reported(&name, "Zx9Qm2LkP7wRt4YvB8nC3dHs6JfG1aEu5XoI0pVq");
                if got.is_empty() || got.iter().any(|id| !CHECK_IDS.contains(&id.as_str())) {
                    failures.push(format!(
                        "{name}: marked heuristic, but a random value is reported as {got:?}"
                    ));
                }
            }
            rule => {
                let got = reported(&name, &sample(rule));
                if !got.iter().any(|id| id == rule) {
                    failures.push(format!("{name}: expected {rule}, scan reports {got:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
