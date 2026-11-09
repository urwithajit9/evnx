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
use super::plan::Environment;
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
struct State<'a> {
    project: &'a str,
    team: Option<&'a str>,
    environment: Environment,
    entries: Vec<Entry<'a>>,
}

/// The state file's text.
pub fn render(
    project: &ProjectRef,
    environment: Environment,
    entries: &[RemoteVar],
) -> Result<String> {
    let state = State {
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
    };
    let mut text = serde_json::to_string_pretty(&state).context("writing the state file")?;
    text.push('\n');
    Ok(text)
}

/// Write the state file under `root`. Returns its path.
pub fn write(
    root: &Path,
    project: &ProjectRef,
    environment: Environment,
    entries: &[RemoteVar],
) -> Result<PathBuf> {
    let path = root.join(PATH);
    if let Some(dir) = path.parent() {
        crate::core::safe_write::create_dirs(dir)?;
    }
    let text = render(project, environment, entries)?;
    crate::core::safe_write::write_private(&path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::vercel::api::Kind;

    #[test]
    fn the_file_has_the_agreed_shape_and_no_values() {
        let project = ProjectRef {
            project: "prj_1".into(),
            team: Some("team_1".into()),
            source: "--project",
        };
        let entries = vec![RemoteVar {
            id: "env_1".into(),
            key: "API_URL".into(),
            kind: Kind::Config,
            raw_type: "encrypted".into(),
            target: vec!["development".into(), "preview".into()],
            git_branch: None,
            updated_at: Value::from(1_700_000_000_000u64),
        }];
        let text = render(&project, Environment::Development, &entries).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["project"], "prj_1");
        assert_eq!(v["team"], "team_1");
        assert_eq!(v["environment"], "development");
        assert_eq!(v["entries"][0]["key"], "API_URL");
        assert_eq!(v["entries"][0]["id"], "env_1");
        assert_eq!(v["entries"][0]["target"][1], "preview");
        assert_eq!(v["entries"][0]["type"], "encrypted");
        assert_eq!(v["entries"][0]["updatedAt"], 1_700_000_000_000u64);
        let keys: Vec<_> = v["entries"][0].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 5, "{keys:?}");
    }
}
