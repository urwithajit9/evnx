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

use anyhow::{Context, Result, bail};
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

/// The reason codes a marker (`reason=<code>`) and a baseline entry may carry
/// (SCAN-REASONS). A code is a closed vocabulary so the summary can count by
/// it; the free text after the marker says the rest. Any other code is kept
/// as written and warned about.
pub const REASON_CODES: &[&str] = &["used-in-tests", "false-positive", "fix-later"];

/// Whether `code` is one of [`REASON_CODES`].
pub fn is_known_reason(code: &str) -> bool {
    REASON_CODES.contains(&code)
}

/// The warning for a reason code outside [`REASON_CODES`].
pub fn unknown_reason_warning(code: &str) -> String {
    format!(
        "unknown reason '{code}'; the codes are {}",
        REASON_CODES.join(", ")
    )
}

/// An `evnx:allow` marker on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The rule it silences. `None` is a bare marker, which silences nothing.
    pub rule: Option<String>,
    /// Why, in words, after `—`, `-` or `:`. Optional, but when neither this
    /// nor `reason_code` is given, its absence is warned about.
    pub reason: Option<String>,
    /// `reason=<code>` straight after the rule id (SCAN-REASONS).
    pub reason_code: Option<String>,
}

/// The marker on `line`, if any: `evnx:allow <rule-id> [reason=<code>] —
/// free text`, in any comment style, since only the text after `evnx:allow`
/// is read.
pub fn marker_in(line: &str) -> Option<Marker> {
    let rest = line.split_once("evnx:allow")?.1;
    let rest = rest.trim_start();
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    // `evnx:allow reason=…` has no rule: `reason` is the key, not an id.
    let id = if rest[id.len()..].starts_with('=') {
        String::new()
    } else {
        id
    };
    let mut after = rest[id.len()..].trim();
    let mut reason_code = None;
    if !id.is_empty() {
        if let Some(coded) = after.strip_prefix("reason=") {
            let code: String = coded
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !code.is_empty() {
                after = coded[code.len()..].trim();
                reason_code = Some(code.to_ascii_lowercase());
            }
        }
    }
    let reason = after
        .trim_start_matches(['—', '–', '-', ':'])
        .trim()
        .trim_end_matches("*/")
        .trim_end_matches("-->")
        .trim();
    Some(Marker {
        rule: (!id.is_empty()).then_some(id),
        reason: (!reason.is_empty()).then(|| reason.to_string()),
        reason_code,
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
    /// The rule set the baseline was written under (SCAN-RESCAN, D7): a hash,
    /// never a rule. Absent in a baseline written before, and an older evnx
    /// ignores it, so the file stays `schema_version` 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_digest: Option<String>,
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
    /// Why it was accepted: one of [`REASON_CODES`] (SCAN-REASONS, D7). Absent
    /// in a baseline written before, and an older evnx ignores it, so the
    /// file stays `schema_version` 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Baseline {
    /// Read `path`. A missing file is an empty baseline when it is the
    /// default, and an error when it was named: a typo in `--baseline` must
    /// not quietly report every accepted finding as new.
    pub fn load(path: &Path, named: bool) -> Result<Baseline> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !named => {
                return Ok(Baseline::default());
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading the baseline {}", path.display()));
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

    /// The entry for `fingerprint`, if it is accepted.
    pub fn get(&self, fingerprint: &str) -> Option<&Entry> {
        self.findings.iter().find(|e| e.fingerprint == fingerprint)
    }

    /// The baseline for `current` (fingerprint, rule, file) findings, keeping
    /// the `added` date and the reason of each one already accepted; `reason`
    /// is given only to the entries added now. `rules_digest` is the rule set
    /// this run used. Sorted, so the file diffs
    /// cleanly.
    pub fn rewrite(
        &self,
        current: &[(String, String, String)],
        today: &str,
        reason: Option<&str>,
        rules_digest: Option<&str>,
    ) -> Baseline {
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
                reason: match before.get(fp.as_str()) {
                    Some(e) => e.reason.clone(),
                    None => reason.map(str::to_string),
                },
            })
            .collect();
        findings.sort_by(|a, b| {
            (&a.file, &a.rule, &a.fingerprint).cmp(&(&b.file, &b.rule, &b.fingerprint))
        });
        findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
        Baseline {
            schema_version: 1,
            rules_digest: rules_digest.map(str::to_string),
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
                reason_code: None,
            })
        );
        assert_eq!(
            marker_in("// evnx:allow jwt").unwrap(),
            Marker {
                rule: Some("jwt".into()),
                reason: None,
                reason_code: None
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
            rules_digest: None,
            findings: vec![Entry {
                fingerprint: "f1".into(),
                rule: "jwt".into(),
                file: ".env".into(),
                added: "2026-01-01".into(),
                reason: None,
            }],
        };
        let new = old.rewrite(
            &[
                ("f1".into(), "jwt".into(), "./.env".into()),
                ("f2".into(), "jwt".into(), "./a.env".into()),
            ],
            "2026-11-07",
            None,
            None,
        );
        assert_eq!(new.findings[0].added, "2026-01-01");
        assert_eq!(new.findings[1].added, "2026-11-07");
        assert_eq!(new.findings[1].file, "a.env");
    }

    #[test]
    fn a_marker_may_carry_a_reason_code_before_its_text() {
        let m = marker_in("X=1 # evnx:allow jwt reason=used-in-tests — a fixture").unwrap();
        assert_eq!(m.rule.as_deref(), Some("jwt"));
        assert_eq!(m.reason_code.as_deref(), Some("used-in-tests"));
        assert_eq!(m.reason.as_deref(), Some("a fixture"));
        // Code alone, in a block comment.
        let m = marker_in("/* evnx:allow jwt reason=fix-later */").unwrap();
        assert_eq!(m.reason_code.as_deref(), Some("fix-later"));
        assert_eq!(m.reason, None);
        // No code: as before. `reason=` is not read without a rule id.
        assert_eq!(
            marker_in("# evnx:allow jwt — why").unwrap().reason_code,
            None
        );
        assert_eq!(
            marker_in("# evnx:allow reason=fix-later").unwrap().rule,
            None
        );
        // An unknown code is kept as written, and is not one of the codes.
        let m = marker_in("# evnx:allow jwt reason=Whatever").unwrap();
        assert_eq!(m.reason_code.as_deref(), Some("whatever"));
        assert!(!is_known_reason("whatever"));
        assert!(REASON_CODES.iter().all(|c| is_known_reason(c)));
    }

    /// Reasons: given to new entries only, kept on old ones, never required;
    /// and an older baseline (no `reason`) still reads, and one without a
    /// reason writes none (schema stays 1, D7).
    #[test]
    fn a_reason_goes_to_new_entries_and_old_ones_keep_theirs() {
        let old = Baseline {
            schema_version: 1,
            rules_digest: None,
            findings: vec![
                Entry {
                    fingerprint: "f1".into(),
                    rule: "jwt".into(),
                    file: ".env".into(),
                    added: "2026-01-01".into(),
                    reason: Some("false-positive".into()),
                },
                Entry {
                    fingerprint: "f2".into(),
                    rule: "jwt".into(),
                    file: ".env".into(),
                    added: "2026-01-01".into(),
                    reason: None,
                },
            ],
        };
        let cur = [
            ("f1".to_string(), "jwt".to_string(), ".env".to_string()),
            ("f2".to_string(), "jwt".to_string(), ".env".to_string()),
            ("f3".to_string(), "jwt".to_string(), "b.env".to_string()),
        ];
        let new = old.rewrite(&cur, "2026-11-19", Some("used-in-tests"), None);
        let by = |fp: &str| new.get(fp).unwrap().reason.clone();
        assert_eq!(by("f1").as_deref(), Some("false-positive"));
        assert_eq!(by("f2"), None);
        assert_eq!(by("f3").as_deref(), Some("used-in-tests"));
        assert_eq!(new.schema_version, 1);

        let text = serde_json::to_string(&old.rewrite(&cur, "d", None, None)).unwrap();
        assert!(!text.contains("reason\":null"), "{text}");
        let back: Baseline = serde_json::from_str(
            r#"{"schema_version":1,"findings":[{"fingerprint":"a","rule":"r","file":"f","added":"d"}]}"#,
        )
        .unwrap();
        assert_eq!(back.findings[0].reason, None);
    }

    /// SCAN-RESCAN, D7: the digest is an optional field, so the file is still
    /// schema 1, an older baseline reads, and one without a digest writes none.
    #[test]
    fn the_rules_digest_is_optional_and_the_schema_stays_1() {
        let old: Baseline = serde_json::from_str(r#"{"schema_version":1,"findings":[]}"#).unwrap();
        assert_eq!(old.rules_digest, None);
        let plain = serde_json::to_string(&old.rewrite(&[], "d", None, None)).unwrap();
        assert!(!plain.contains("rules_digest"), "{plain}");
        let new = old.rewrite(&[], "d", None, Some("abc"));
        assert_eq!(new.schema_version, 1);
        let text = serde_json::to_string(&new).unwrap();
        assert!(text.contains(r#""rules_digest":"abc""#), "{text}");
        let back: Baseline = serde_json::from_str(&text).unwrap();
        assert_eq!(back.rules_digest.as_deref(), Some("abc"));
    }
}
