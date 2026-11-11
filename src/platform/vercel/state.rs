//! `.evnx/platform/vercel.json`: what Vercel looked like at the last pull.
//!
//! Each variable's id, targets, type and `updatedAt`, so that a later `push` can
//! stop with exit 14 when Vercel changed since (there is no etag to use).
//!
//! **No values and no hashes**, ever: a hash of a weak value can be brute-forced
//! offline, and the file is easy to commit by accident. It is still machine-local
//! state, so a pull that writes it hints at `.gitignore` when it is not ignored.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use super::api::RemoteVar;
use super::project::ProjectRef;

/// Where it lives, from the project root.
pub const PATH: &str = ".evnx/platform/vercel.json";

#[derive(Serialize)]
struct Entry<'a> {
    key: &'a str,
    id: &'a str,
    target: &'a [String],
    #[serde(rename = "type")]
    type_: &'a str,
    #[serde(rename = "updatedAt")]
    updated_at: &'a Value,
}

#[derive(Serialize)]
struct Record<'a> {
    project: &'a str,
    team: Option<&'a str>,
    environment: &'a str,
    entries: Vec<Entry<'a>>,
}

/// The file's current shape: one record per project, team and environment.
///
/// ⓘ Until 0.14 the file *was* a single record (`project`, `team`,
/// `environment`, `entries` at the top level), so pulling production replaced
/// development's and a development push could not see what changed on Vercel.
/// That shape is still read, and upgraded by the next write.
const VERSION: u64 = 2;

fn record_of(project: &ProjectRef, environment: &str, entries: &[RemoteVar]) -> Result<Value> {
    serde_json::to_value(Record {
        project: &project.project,
        team: project.team.as_deref(),
        environment,
        entries: entries
            .iter()
            .map(|e| Entry {
                key: &e.key,
                id: &e.id,
                target: &e.target,
                type_: &e.raw_type,
                updated_at: &e.updated_at,
            })
            .collect(),
    })
    .context("writing the state file")
}

/// Does `record` belong to this project, team and environment?
fn is_for(record: &Value, project: &ProjectRef, environment: &str) -> bool {
    is_for_project(record, project) && record["environment"].as_str() == Some(environment)
}

fn is_for_project(record: &Value, project: &ProjectRef) -> bool {
    record["project"].as_str() == Some(project.project.as_str())
        && record["team"].as_str() == project.team.as_deref()
}

/// The records a parsed file holds, from either shape.
fn records_of(file: &Value) -> Option<Vec<Value>> {
    if let Some(list) = file["records"].as_array() {
        return Some(list.clone());
    }
    // The old single-record shape.
    file["entries"].is_array().then(|| vec![file.clone()])
}

/// The state file's text after recording `entries` for this project and
/// environment: `existing` (the file as it is now, if any) with only that
/// environment's record replaced.
pub fn render(
    existing: Option<&str>,
    project: &ProjectRef,
    environment: &str,
    entries: &[RemoteVar],
) -> Result<String> {
    let mut records = existing
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| records_of(&v))
        .unwrap_or_default();
    let new = record_of(project, environment, entries)?;
    match records.iter().position(|r| is_for(r, project, environment)) {
        Some(i) => records[i] = new,
        None => records.push(new),
    }
    let file = serde_json::json!({ "version": VERSION, "records": records });
    let mut text = serde_json::to_string_pretty(&file).context("writing the state file")?;
    text.push('\n');
    Ok(text)
}

/// Write the state file under `root`. Returns its path.
pub fn write(
    root: &Path,
    project: &ProjectRef,
    environment: &str,
    entries: &[RemoteVar],
) -> Result<PathBuf> {
    let path = root.join(PATH);
    if let Some(dir) = path.parent() {
        crate::core::safe_write::create_dirs(dir)?;
    }
    let existing = std::fs::read_to_string(&path).ok();
    let text = render(existing.as_deref(), project, environment, entries)?;
    crate::core::safe_write::write_private(&path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// What the state file said, for `push`'s check.
pub enum Loaded {
    /// There is no state file.
    Missing,
    /// There is one, and it cannot be used: the sentence says why.
    Unusable(String),
    /// Each entry's `updatedAt` as of the last pull or push.
    Usable(Seen),
}

/// The entries a state file recorded.
pub struct Seen {
    updated: std::collections::HashMap<String, Value>,
}

impl Seen {
    /// The `updatedAt` recorded for an entry id, if the file knew the entry.
    pub fn updated_at(&self, id: &str) -> Option<&Value> {
        self.updated.get(id)
    }
}

/// Read the state file under `root`. Only this project, team and environment's
/// record counts: another environment's is not evidence about this one.
pub fn load(root: &Path, project: &ProjectRef, environment: &str) -> Loaded {
    let path = root.join(PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(_) => return Loaded::Unusable("cannot be read".into()),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Loaded::Unusable("is not valid JSON".into());
    };
    let Some(records) = records_of(&v) else {
        return Loaded::Unusable("has no entries".into());
    };
    let Some(record) = records.iter().find(|r| is_for(r, project, environment)) else {
        return Loaded::Unusable(if records.iter().any(|r| is_for_project(r, project)) {
            format!("has no record for {environment} yet")
        } else {
            "is for another project".into()
        });
    };
    let Some(entries) = record["entries"].as_array() else {
        return Loaded::Unusable("has no entries".into());
    };
    let updated = entries
        .iter()
        .filter_map(|e| Some((e["id"].as_str()?.to_string(), e["updatedAt"].clone())))
        .collect();
    Loaded::Usable(Seen { updated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::vercel::api::Kind;

    fn project(name: &str) -> ProjectRef {
        ProjectRef {
            project: name.into(),
            team: Some("team_1".into()),
            source: "--project",
        }
    }

    fn var(id: &str, at: u64) -> RemoteVar {
        RemoteVar {
            id: id.into(),
            key: "API_URL".into(),
            kind: Kind::Config,
            raw_type: "encrypted".into(),
            target: vec!["development".into(), "preview".into()],
            git_branch: None,
            updated_at: Value::from(at),
            custom_environment_ids: vec![],
        }
    }

    #[test]
    fn the_file_has_the_agreed_shape_and_no_values() {
        let text = render(
            None,
            &project("prj_1"),
            "development",
            &[var("env_1", 1_700_000_000_000)],
        )
        .unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["version"], 2);
        let r = &v["records"][0];
        assert_eq!(r["project"], "prj_1");
        assert_eq!(r["team"], "team_1");
        assert_eq!(r["environment"], "development");
        assert_eq!(r["entries"][0]["key"], "API_URL");
        assert_eq!(r["entries"][0]["id"], "env_1");
        assert_eq!(r["entries"][0]["target"][1], "preview");
        assert_eq!(r["entries"][0]["type"], "encrypted");
        assert_eq!(r["entries"][0]["updatedAt"], 1_700_000_000_000u64);
        let keys: Vec<_> = r["entries"][0].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 5, "{keys:?}");
    }

    #[test]
    fn each_environment_keeps_its_own_record() {
        let dir = tempfile::tempdir().unwrap();
        let p = project("prj_1");
        assert!(matches!(
            load(dir.path(), &p, "development"),
            Loaded::Missing
        ));
        write(dir.path(), &p, "development", &[var("env_1", 7)]).unwrap();
        write(dir.path(), &p, "production", &[var("env_2", 9)]).unwrap();
        // Development's record survived production's write.
        match load(dir.path(), &p, "development") {
            Loaded::Usable(s) => {
                assert_eq!(s.updated_at("env_1"), Some(&Value::from(7u64)));
                assert_eq!(s.updated_at("env_2"), None);
            }
            _ => panic!("usable"),
        }
        match load(dir.path(), &p, "production") {
            Loaded::Usable(s) => assert_eq!(s.updated_at("env_2"), Some(&Value::from(9u64))),
            _ => panic!("usable"),
        }
        // Writing again replaces that record only.
        write(dir.path(), &p, "development", &[var("env_1", 8)]).unwrap();
        let text = std::fs::read_to_string(dir.path().join(PATH)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["records"].as_array().unwrap().len(), 2);
        // No record for preview; another project has none at all.
        assert!(
            matches!(load(dir.path(), &p, "preview"), Loaded::Unusable(w) if w.contains("no record for preview"))
        );
        assert!(
            matches!(load(dir.path(), &project("prj_2"), "development"), Loaded::Unusable(w) if w.contains("another project"))
        );
    }

    #[test]
    fn the_old_single_record_file_is_read_and_upgraded_by_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = project("prj_1");
        let old = serde_json::json!({
            "project": "prj_1", "team": "team_1", "environment": "production",
            "entries": [{ "key": "K", "id": "old_1", "target": ["production"],
                          "type": "encrypted", "updatedAt": 3 }],
        });
        std::fs::create_dir_all(dir.path().join(".evnx/platform")).unwrap();
        std::fs::write(dir.path().join(PATH), old.to_string()).unwrap();
        match load(dir.path(), &p, "production") {
            Loaded::Usable(s) => assert_eq!(s.updated_at("old_1"), Some(&Value::from(3u64))),
            _ => panic!("the old shape is read"),
        }
        write(dir.path(), &p, "development", &[var("env_1", 7)]).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(PATH)).unwrap()).unwrap();
        assert_eq!(v["version"], 2);
        assert_eq!(v["records"].as_array().unwrap().len(), 2);
        assert!(matches!(
            load(dir.path(), &p, "production"),
            Loaded::Usable(_)
        ));
    }
}
