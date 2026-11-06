//! `evnx scan rules list | new | test` (C6, S3, S5).
//!
//! Rule files share `src/assets/rules.toml`'s schema, plus optional
//! `positive` and `near_miss` lists that `test` checks and a scan ignores.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::core::config::PatternRule;
use crate::core::secretscan::rules::{self, Sourced};
use crate::core::secretscan::{Confidence, PatternSet, Severity};
use crate::core::verdict::Verdict;

/// Every rule a scan in `project` would use, with where each came from.
fn all_rules(project: &Path, configured: &[PatternRule]) -> Result<Vec<Sourced>> {
    let mut out: Vec<Sourced> = rules::builtin_rules()
        .into_iter()
        .map(|rule| Sourced {
            rule,
            source: "evnx".to_string(),
        })
        .collect();
    out.extend(configured.iter().cloned().map(|rule| Sourced {
        rule,
        source: ".evnx.toml [[scan.patterns]]".to_string(),
    }));
    let loaded = rules::load(project, rules::personal_dir().as_deref(), &[])?;
    for w in &loaded.warnings {
        crate::utils::ui::warning(w);
    }
    out.extend(loaded.rules);
    Ok(out)
}

/// `list`: id, source, severity and confidence of each rule, as compiled.
pub fn list(project: &Path, configured: &[PatternRule], json: bool) -> Result<()> {
    let rules = all_rules(project, configured)?;
    // Compiled together, as a scan does, so a broken rule is reported here.
    PatternSet::compile(&rules.iter().map(|s| s.rule.clone()).collect::<Vec<_>>())?;
    let rows: Vec<(String, String, String, String, String)> = rules
        .iter()
        .map(|s| {
            let id = s
                .rule
                .id
                .clone()
                .unwrap_or_else(|| crate::core::secretscan::patternset::slug(&s.rule.name));
            let severity = s.rule.severity.clone().unwrap_or_else(|| "medium".into());
            let confidence = s.rule.confidence.clone().unwrap_or_else(|| "high".into());
            (
                id,
                s.rule.name.clone(),
                s.source.clone(),
                severity,
                confidence,
            )
        })
        .collect();
    if json {
        let doc: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, name, source, sev, conf)| {
                serde_json::json!({
                    "id": id, "name": name, "source": source,
                    "severity": sev, "confidence": conf,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(2);
    for (id, _, source, sev, conf) in &rows {
        println!("  {id:<width$}  {sev:<8}  {conf:<6}  {source}");
    }
    println!("\n  {} rules", rows.len());
    Ok(())
}

/// `new`: a rule file to fill in, never over an existing one.
pub fn new(project: &Path, id: &str, personal: bool) -> Result<PathBuf> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("'{id}' is not a rule id: use lowercase letters, digits and '-'");
    }
    if rules::builtin_rules()
        .iter()
        .any(|r| r.id.as_deref() == Some(id))
    {
        bail!("'{id}' is one of evnx's own rules; a rule file adds rules, so pick another id");
    }
    let dir = if personal {
        rules::personal_dir()
            .ok_or_else(|| anyhow::anyhow!("no personal config folder; set EVNX_CONFIG_DIR"))?
    } else {
        project.join(rules::PROJECT_RULES_DIR)
    };
    let path = dir.join(format!("{id}.toml"));
    if path.exists() {
        bail!(
            "{} already exists; edit it, or pick another id",
            path.display()
        );
    }
    crate::core::safe_write::create_dirs(&dir)?;
    let name: String = id
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let body = format!(
        "# A rule file for `evnx scan` (C6). Its rules are added to evnx's own;\n\
         # an id evnx or another file already uses is refused, never replaced.\n\
         # Same fields as evnx's own rules; check it with `evnx scan rules test`.\n\
         version = 1\n\n\
         [[rules]]\n\
         id = \"{id}\"\n\
         name = \"{name}\"\n\
         # The token's documented shape. Rust regex syntax, matched per line.\n\
         regex = 'REPLACE-ME-[A-Z0-9]{{32}}'\n\
         # How sure a match is (high, medium, low), and how bad if real\n\
         # (critical, high, medium, low).\n\
         confidence = \"high\"\n\
         severity = \"high\"\n\
         # url = \"https://… where to revoke one\"\n\n\
         # Values this rule must report, and close ones it must not.\n\
         positive = []\n\
         near_miss = []\n"
    );
    crate::core::safe_write::write_shared(&path, body.as_bytes())?;
    crate::utils::ui::success(format!(
        "wrote {}: set its regex and fixtures, then `evnx scan rules test`",
        crate::utils::path::label(&path.to_string_lossy())
    ));
    Ok(path)
}

/// `test`: each rule against its own fixtures, every failure reported.
pub fn test(project: &Path, file: Option<&str>) -> Result<Verdict> {
    let rules: Vec<Sourced> = match file {
        Some(f) => rules::read_rule_file(Path::new(f), f)?
            .into_iter()
            .map(|rule| Sourced {
                rule,
                source: f.to_string(),
            })
            .collect(),
        None => {
            let loaded = rules::load(project, rules::personal_dir().as_deref(), &[])?;
            for w in &loaded.warnings {
                crate::utils::ui::warning(w);
            }
            loaded.rules
        }
    };
    if rules.is_empty() {
        bail!(
            "no rule files: none in {} or your personal rules. Start one with \
             `evnx scan rules new <id>`",
            rules::PROJECT_RULES_DIR
        );
    }
    let (mut failures, mut checked) = (Vec::new(), 0usize);
    for s in &rules {
        let id = s.rule.id.clone().unwrap_or_else(|| s.rule.name.clone());
        let set = match PatternSet::compile(std::slice::from_ref(&s.rule)) {
            Ok(set) => set,
            Err(e) => {
                failures.push(format!("{id} ({}): {e:#}", s.source));
                continue;
            }
        };
        let positive = s.rule.positive.as_deref().unwrap_or_default();
        let near_miss = s.rule.near_miss.as_deref().unwrap_or_default();
        if positive.is_empty() && near_miss.is_empty() {
            crate::utils::ui::warning(format!("{id}: no positive or near_miss values to test"));
        }
        for v in positive {
            checked += 1;
            if set.find_all(v).is_empty() {
                failures.push(format!("{id}: does not report positive {v:?}"));
            }
        }
        for v in near_miss {
            checked += 1;
            if !set.find_all(v).is_empty() {
                failures.push(format!("{id}: reports near miss {v:?}"));
            }
        }
        // The levels parse, as a scan would require.
        if let Some(c) = &s.rule.confidence {
            if let Err(e) = c.parse::<Confidence>() {
                failures.push(format!("{id}: {e}"));
            }
        }
        if let Some(v) = &s.rule.severity {
            if let Err(e) = v.parse::<Severity>() {
                failures.push(format!("{id}: {e}"));
            }
        }
    }
    for f in &failures {
        crate::utils::ui::error(f);
    }
    if failures.is_empty() {
        crate::utils::ui::success(format!(
            "{} rule(s), {checked} fixture value(s): all pass",
            rules.len()
        ));
        Ok(Verdict::Clean)
    } else {
        println!("\n  {} failure(s)", failures.len());
        Ok(Verdict::Findings)
    }
}
