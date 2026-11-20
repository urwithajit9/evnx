//! The scan policy floor (WP-13): a file the repository cannot lower.
//!
//! `.evnx.toml` is committed, so whoever can open a pull request can disable a
//! detector or raise the severity floor for everyone who scans it. A policy
//! file lives somewhere the change under review does not reach (a CI
//! configuration, the runner's image, a path in `EVNX_POLICY`) and says what
//! the repository may not do: `required_rules` may not be disabled,
//! `max_severity_floor` is the highest `--severity` / `[scan] severity` allowed,
//! and `allow_disable` lists the only detectors a repository may disable.
//!
//! ⚠️ A conflict is exit 2, "no verdict", before any file is read. A scan the
//! policy forbids must not be able to report clean.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::core::config::ScanPolicy;
use crate::core::secretscan::Severity;

/// `allow_disable = false | true | ["name", ...]`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum AllowDisable {
    All(bool),
    Names(Vec<String>),
}

impl Default for AllowDisable {
    fn default() -> Self {
        AllowDisable::All(true)
    }
}

/// The parsed policy file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Body {
    #[serde(default)]
    required_rules: Vec<String>,
    max_severity_floor: Option<String>,
    #[serde(default)]
    allow_disable: AllowDisable,
}

/// A policy file and where it was read from.
#[derive(Debug, Clone)]
pub struct Policy {
    file: PathBuf,
    body: Body,
}

/// One id and name per check a scan can run, so `required_rules = ["aws-access-key"]`
/// and `scan.disable = ["AWS Access Key"]` are recognised as the same rule.
fn catalogue() -> Vec<(String, String)> {
    use crate::core::secretscan::detector::{
        DECLARED_SECRET, DECLARED_SECRET_ID, SENSITIVE_KEY_ID,
    };
    let mut out: Vec<(String, String)> = crate::core::secretscan::rules::builtin_rules()
        .into_iter()
        .map(|r| {
            let name = r.name.to_lowercase();
            (
                r.id.map(|i| i.to_lowercase())
                    .unwrap_or_else(|| name.clone()),
                name,
            )
        })
        .collect();
    for (id, name) in [
        (SENSITIVE_KEY_ID, "Sensitive config key"),
        ("high-entropy-string", "High-entropy string"),
        ("aws-secret-access-key", "AWS Secret Access Key"),
        (DECLARED_SECRET_ID, DECLARED_SECRET),
    ] {
        out.push((id.to_lowercase(), name.to_lowercase()));
    }
    out
}

/// The id of the rule `token` names (by id or name, any case), else `None`.
fn canonical(token: &str, catalogue: &[(String, String)]) -> Option<String> {
    let t = token.trim().to_lowercase();
    catalogue
        .iter()
        .find(|(id, name)| *id == t || *name == t)
        .map(|(id, _)| id.clone())
}

impl Policy {
    /// Read and check `file`. Missing, unreadable, unparsable or naming a
    /// rule evnx does not have: an error, so a typo cannot leave a rule
    /// unprotected.
    pub fn load(file: &Path) -> Result<Policy> {
        let shown = crate::utils::ui::short_path(file);
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("policy file {shown} could not be read"))?;
        let body: Body =
            toml::from_str(&text).with_context(|| format!("policy file {shown} is not valid"))?;
        let cat = catalogue();
        for name in body.required_rules.iter().chain(match &body.allow_disable {
            AllowDisable::Names(n) => n.iter(),
            AllowDisable::All(_) => [].iter(),
        }) {
            if canonical(name, &cat).is_none() {
                bail!(
                    "policy file {shown} names `{name}`, which is not a rule evnx has (see `evnx scan rules list`)"
                );
            }
        }
        if let Some(floor) = &body.max_severity_floor {
            floor
                .parse::<Severity>()
                .with_context(|| format!("policy file {shown}: max_severity_floor"))?;
        }
        Ok(Policy {
            file: file.to_path_buf(),
            body,
        })
    }

    /// The file this policy was read from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The policy's settings, for `evnx config show`: key and value as JSON.
    /// Rule names and a severity: nothing secret.
    #[must_use]
    pub fn settings(&self) -> Vec<(&'static str, serde_json::Value)> {
        use serde_json::json;
        vec![
            ("policy.required_rules", json!(self.body.required_rules)),
            (
                "policy.max_severity_floor",
                json!(self.body.max_severity_floor),
            ),
            (
                "policy.allow_disable",
                match &self.body.allow_disable {
                    AllowDisable::All(b) => json!(b),
                    AllowDisable::Names(n) => json!(n),
                },
            ),
        ]
    }

    /// Check the repository's `[scan]` section and this run's `--severity`
    /// against the policy. The first conflict is the error.
    pub fn check(&self, scan: &ScanPolicy, severity_flag: Option<&str>) -> Result<()> {
        let shown = crate::utils::ui::short_path(&self.file);
        let cat = catalogue();
        let disabled = scan.disable.as_deref().unwrap_or(&[]);

        for want in &self.body.required_rules {
            let id = canonical(want, &cat);
            let hit: Vec<&String> = disabled
                .iter()
                .filter(|d| canonical(d, &cat) == id)
                .collect();
            if !hit.is_empty() {
                bail!(
                    "policy {shown} forbids scan.disable = {} (required_rules lists `{want}`)",
                    quote(&hit)
                );
            }
        }
        match &self.body.allow_disable {
            AllowDisable::All(true) => {}
            AllowDisable::All(false) if !disabled.is_empty() => bail!(
                "policy {shown} forbids scan.disable = {} (allow_disable = false)",
                quote(&disabled.iter().collect::<Vec<_>>())
            ),
            AllowDisable::All(false) => {}
            AllowDisable::Names(allowed) => {
                let allowed: Vec<Option<String>> =
                    allowed.iter().map(|a| canonical(a, &cat)).collect();
                let bad: Vec<&String> = disabled
                    .iter()
                    .filter(|d| !allowed.contains(&canonical(d, &cat)))
                    .collect();
                if !bad.is_empty() {
                    bail!(
                        "policy {shown} forbids scan.disable = {} (allow_disable lists only {})",
                        quote(&bad),
                        quote(&self.listed())
                    );
                }
            }
        }
        if scan.builtins == Some(false) {
            if !self.body.required_rules.is_empty() {
                bail!("policy {shown} forbids scan.builtins = false (required_rules)");
            }
            if self.body.allow_disable != AllowDisable::All(true) {
                bail!(
                    "policy {shown} forbids scan.builtins = false (allow_disable restricts disabling)"
                );
            }
        }
        if let Some(floor) = &self.body.max_severity_floor {
            let max: Severity = floor.parse()?;
            let candidates = [
                ("scan.severity", scan.severity.as_deref()),
                ("--severity", severity_flag),
            ];
            for (what, value) in candidates {
                if let Some(v) = value {
                    if v.parse::<Severity>()? > max {
                        bail!(
                            "policy {shown} forbids {what} = \"{v}\" (max_severity_floor = \"{floor}\")"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn listed(&self) -> Vec<&String> {
        match &self.body.allow_disable {
            AllowDisable::Names(n) => n.iter().collect(),
            AllowDisable::All(_) => Vec::new(),
        }
    }
}

fn quote(names: &[&String]) -> String {
    let inner: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    format!("[{}]", inner.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(text: &str) -> Policy {
        Policy {
            file: PathBuf::from("policy.toml"),
            body: toml::from_str(text).unwrap(),
        }
    }

    fn scan(toml: &str) -> ScanPolicy {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn a_required_rule_is_matched_by_id_or_name() {
        let p = policy(r#"required_rules = ["sensitive-config-key"]"#);
        let err = p
            .check(&scan(r#"disable = ["Sensitive config key"]"#), None)
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("policy policy.toml forbids scan.disable")
        );
        assert!(
            p.check(&scan(r#"disable = ["aws-access-key"]"#), None)
                .is_ok()
        );
    }

    #[test]
    fn the_floor_compares_severities_not_strings() {
        let p = policy(r#"max_severity_floor = "medium""#);
        assert!(
            p.check(&scan(r#"severity = "low""#), Some("medium"))
                .is_ok()
        );
        assert!(p.check(&scan(r#"severity = "high""#), None).is_err());
        assert!(p.check(&scan(""), Some("critical")).is_err());
    }

    #[test]
    fn allow_disable_defaults_to_unrestricted() {
        assert!(
            policy("")
                .check(&scan(r#"disable = ["Sensitive config key"]"#), None)
                .is_ok()
        );
    }

    #[test]
    fn unknown_policy_keys_are_refused() {
        assert!(toml::from_str::<Body>("required_rule = []").is_err());
    }
}
