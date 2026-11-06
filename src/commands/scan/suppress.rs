//! Findings a project has looked at: `evnx:allow` markers and the baseline
//! (C5, RM-54; decisions S8 and S9).
//!
//! Neither one removes a finding. An allowed or known finding is still found,
//! listed in its own array (`allowed`, `known`) with its `status`, counted in
//! the summary and listed by `-v`. It only stops counting towards the exit
//! code, so `findings` stays exactly what fails a build.
//!
//! ⛔ **The baseline never holds a value or a value preview** (S9): only a
//! rule id, a file and a fingerprint. Committing it publishes nothing.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Where the baseline lives unless `--baseline` says otherwise (S9): beside
/// the project's rule files.
pub const DEFAULT_BASELINE: &str = ".evnx/baseline.json";

/// A finding's identity across runs (S9): BLAKE3 of the rule, the file, and
/// the variable or, with none, the line's text. No line **number**, so a line
/// inserted above keeps it; the line's text includes the value, so a changed
/// value is a new finding.
pub fn fingerprint(rule_id: &str, file: &str, variable: Option<&str>, line_text: &str) -> String {
    let file = file.strip_prefix("./").unwrap_or(file);
    let mut h = blake3::Hasher::new();
    for part in [rule_id, file, variable.unwrap_or(""), line_text.trim()] {
        h.update(part.as_bytes());
        h.update(&[0]);
    }
    h.finalize().to_hex().to_string()
}

/// An `evnx:allow` marker on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The rule it silences. `None` is a bare marker, which silences nothing.
    pub rule: Option<String>,
    /// Why, after `—`, `-` or `:`. Optional, but its absence is warned about.
    pub reason: Option<String>,
}

/// The marker on `line`, if any: `evnx:allow <rule-id> — reason`, in any
/// comment style, since only the text after `evnx:allow` is read.
pub fn marker_in(line: &str) -> Option<Marker> {
    let rest = line.split_once("evnx:allow")?.1;
    let rest = rest.trim_start();
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    let after = rest[id.len()..].trim();
    let reason = after
        .trim_start_matches(['—', '–', '-', ':'])
        .trim()
        .trim_end_matches("*/")
        .trim_end_matches("-->")
        .trim();
    Some(Marker {
        rule: (!id.is_empty()).then_some(id),
        reason: (!reason.is_empty()).then(|| reason.to_string()),
    })
}

/// The marker that allows `rule_id` on 1-based `line`: on that line or the one
/// above (S8).
pub fn allowing(lines: &[&str], line: usize, rule_id: &str) -> Option<Marker> {
    [line, line.saturating_sub(1)]
        .into_iter()
        .filter(|&n| n >= 1)
        .filter_map(|n| lines.get(n - 1))
        .filter_map(|l| marker_in(l))
        .find(|m| m.rule.as_deref() == Some(rule_id))
}

/// `.evnx/baseline.json` (S9).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Baseline {
    pub schema_version: u32,
    pub findings: Vec<Entry>,
}

/// One accepted finding. ⛔ No value, no value preview.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub fingerprint: String,
    pub rule: String,
    pub file: String,
    /// The day it was first accepted, `YYYY-MM-DD`.
    pub added: String,
}

impl Baseline {
    /// Read `path`. A missing file is an empty baseline when it is the
    /// default, and an error when it was named: a typo in `--baseline` must
    /// not quietly report every accepted finding as new.
    pub fn load(path: &Path, named: bool) -> Result<Baseline> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !named => {
                return Ok(Baseline::default())
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading the baseline {}", path.display()))
            }
        };
        let b: Baseline = serde_json::from_str(&text)
            .with_context(|| format!("{} is not an evnx baseline", path.display()))?;
        if b.schema_version != 1 {
            bail!(
                "{} has schema_version {}; this evnx reads 1",
                path.display(),
                b.schema_version
            );
        }
        Ok(b)
    }

    pub fn contains(&self, fingerprint: &str) -> bool {
        self.findings.iter().any(|e| e.fingerprint == fingerprint)
    }

    /// The baseline for `current` (fingerprint, rule, file) findings, keeping
    /// the `added` date of each one already accepted. Sorted, so the file
    /// diffs cleanly.
    pub fn rewrite(&self, current: &[(String, String, String)], today: &str) -> Baseline {
        let before: HashMap<&str, &Entry> = self
            .findings
            .iter()
            .map(|e| (e.fingerprint.as_str(), e))
            .collect();
        let mut findings: Vec<Entry> = current
            .iter()
            .map(|(fp, rule, file)| Entry {
                fingerprint: fp.clone(),
                rule: rule.clone(),
                file: file.strip_prefix("./").unwrap_or(file).to_string(),
                added: before
                    .get(fp.as_str())
                    .map(|e| e.added.clone())
                    .unwrap_or_else(|| today.to_string()),
            })
            .collect();
        findings.sort_by(|a, b| {
            (&a.file, &a.rule, &a.fingerprint).cmp(&(&b.file, &b.rule, &b.fingerprint))
        });
        findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
        Baseline {
            schema_version: 1,
            findings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_name_a_rule_and_may_give_a_reason() {
        assert_eq!(
            marker_in("API=x # evnx:allow aws-access-key — the documented example key"),
            Some(Marker {
                rule: Some("aws-access-key".into()),
                reason: Some("the documented example key".into()),
            })
        );
        assert_eq!(
            marker_in("// evnx:allow jwt").unwrap(),
            Marker {
                rule: Some("jwt".into()),
                reason: None
            }
        );
        assert_eq!(
            marker_in("/* evnx:allow jwt: fixture */")
                .unwrap()
                .reason
                .as_deref(),
            Some("fixture")
        );
        // A bare marker names nothing, so it allows nothing (S8).
        assert_eq!(marker_in("# evnx:allow").unwrap().rule, None);
        assert_eq!(marker_in("# nothing here"), None);
    }

    #[test]
    fn a_marker_allows_its_line_and_the_next_only_for_its_rule() {
        let lines = ["# evnx:allow jwt — test token", "TOKEN=eyJ…", "OTHER=eyJ…"];
        assert!(allowing(&lines, 2, "jwt").is_some());
        assert!(allowing(&lines, 1, "jwt").is_some());
        assert!(allowing(&lines, 3, "jwt").is_none());
        assert!(allowing(&lines, 2, "github-token").is_none());
    }

    #[test]
    fn a_fingerprint_ignores_the_line_number_and_follows_the_value() {
        let a = fingerprint("jwt", "./.env", Some("TOKEN"), "TOKEN=abc");
        assert_eq!(a, fingerprint("jwt", ".env", Some("TOKEN"), "  TOKEN=abc "));
        assert_ne!(a, fingerprint("jwt", ".env", Some("TOKEN"), "TOKEN=abd"));
        assert_ne!(
            a,
            fingerprint("github-token", ".env", Some("TOKEN"), "TOKEN=abc")
        );
    }

    #[test]
    fn rewriting_keeps_the_date_a_finding_was_first_accepted() {
        let old = Baseline {
            schema_version: 1,
            findings: vec![Entry {
                fingerprint: "f1".into(),
                rule: "jwt".into(),
                file: ".env".into(),
                added: "2026-01-01".into(),
            }],
        };
        let new = old.rewrite(
            &[
                ("f1".into(), "jwt".into(), "./.env".into()),
                ("f2".into(), "jwt".into(), "./a.env".into()),
            ],
            "2026-11-07",
        );
        assert_eq!(new.findings[0].added, "2026-01-01");
        assert_eq!(new.findings[1].added, "2026-11-07");
        assert_eq!(new.findings[1].file, "a.env");
    }
}
