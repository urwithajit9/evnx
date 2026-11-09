//! Which Vercel project (and team) a command means.
//!
//! In order: `--project` / `--team`, then `VERCEL_PROJECT_ID` / `VERCEL_ORG_ID`,
//! then `.vercel/project.json` (`projectId`, `orgId`) from `vercel link`. Found
//! nowhere, the command stops and names all three.
//!
//! The project id and the team id are each taken from the first place that has
//! one, with one exception: `orgId` from `.vercel/project.json` is used only when
//! the project id came from that file as well. The file pairs them; pairing a
//! file's team with a project named on the command line would address a project
//! in a team it is not in.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// `VERCEL_PROJECT_ID`.
pub const ENV_PROJECT: &str = "VERCEL_PROJECT_ID";
/// `VERCEL_ORG_ID`.
pub const ENV_TEAM: &str = "VERCEL_ORG_ID";
/// Written by `vercel link`.
pub const LINK_FILE: &str = ".vercel/project.json";

/// The project a command works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    /// Project id (`prj_…`) or name.
    pub project: String,
    /// Team id (`team_…`) or slug; `None` for a personal project.
    pub team: Option<String>,
    /// Where the project came from, for the run's own output.
    pub source: &'static str,
}

impl ProjectRef {
    /// `team/project`, or just the project.
    pub fn label(&self) -> String {
        match &self.team {
            Some(t) => format!("{t}/{}", self.project),
            None => self.project.clone(),
        }
    }
}

/// Resolve the project. `env` looks a variable up (so tests need not touch the
/// process environment); `dir` is where `.vercel/project.json` is looked for.
pub fn resolve(
    project: Option<&str>,
    team: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
    dir: &Path,
) -> Result<ProjectRef> {
    let nonblank = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let flag_project = nonblank(project.map(str::to_string));
    let flag_team = nonblank(team.map(str::to_string));
    let env_project = nonblank(env(ENV_PROJECT));
    let env_team = nonblank(env(ENV_TEAM));

    let (project_id, project_source) = if let Some(p) = flag_project {
        (Some(p), "--project")
    } else if let Some(p) = env_project {
        (Some(p), ENV_PROJECT)
    } else {
        (None, LINK_FILE)
    };

    let mut team_id = flag_team
        .map(|t| (t, "--team"))
        .or_else(|| env_team.map(|t| (t, ENV_TEAM)));

    let (project_id, source) = match project_id {
        Some(p) => (p, project_source),
        None => {
            let Some((p, linked_team)) = read_link(dir)? else {
                bail!(
                    "no Vercel project given. Name it one of three ways:\n  \
                     --project <id> [--team <id>]\n  \
                     {ENV_PROJECT} (and {ENV_TEAM} for a team project)\n  \
                     {LINK_FILE}, written by `vercel link`"
                );
            };
            if team_id.is_none() {
                team_id = linked_team.map(|t| (t, LINK_FILE));
            }
            (p, LINK_FILE)
        }
    };

    check_segment(&project_id, "project")?;
    if let Some((t, _)) = &team_id {
        check_segment(t, "team")?;
    }
    Ok(ProjectRef {
        project: project_id,
        team: team_id.map(|(t, _)| t),
        source,
    })
}

/// `projectId` and `orgId` from `.vercel/project.json`, if the file exists.
fn read_link(dir: &Path) -> Result<Option<(String, Option<String>)>> {
    let path = dir.join(LINK_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow!("{} is not valid JSON ({e})", path.display()))?;
    let get = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let Some(project) = get("projectId") else {
        bail!(
            "{} has no projectId; run `vercel link` again",
            path.display()
        );
    };
    Ok(Some((project, get("orgId"))))
}

/// An id or slug goes into a URL path, so it is held to the characters Vercel
/// uses in them. `/`, `?`, `..` and the like never reach a request.
fn check_segment(s: &str, what: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        bail!("the Vercel {what} {s:?} is not an id or a name (letters, digits, - _ . only)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join(".vercel")).unwrap();
        std::fs::write(dir.join(LINK_FILE), body).unwrap();
    }

    fn envs(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn flags_beat_the_environment_beat_the_file() {
        let d = tempfile::tempdir().unwrap();
        linked(d.path(), r#"{"projectId":"prj_file","orgId":"team_file"}"#);
        let e = envs(&[(ENV_PROJECT, "prj_env"), (ENV_TEAM, "team_env")]);

        let p = resolve(Some("prj_flag"), Some("team_flag"), &e, d.path()).unwrap();
        assert_eq!(
            (p.project.as_str(), p.team.as_deref()),
            ("prj_flag", Some("team_flag"))
        );
        assert_eq!(p.source, "--project");

        let p = resolve(None, None, &e, d.path()).unwrap();
        assert_eq!(
            (p.project.as_str(), p.team.as_deref()),
            ("prj_env", Some("team_env"))
        );
        assert_eq!(p.source, ENV_PROJECT);

        let p = resolve(None, None, envs(&[]), d.path()).unwrap();
        assert_eq!(
            (p.project.as_str(), p.team.as_deref()),
            ("prj_file", Some("team_file"))
        );
        assert_eq!(p.source, LINK_FILE);
    }

    #[test]
    fn a_files_team_never_pairs_with_another_projects_id() {
        let d = tempfile::tempdir().unwrap();
        linked(d.path(), r#"{"projectId":"prj_file","orgId":"team_file"}"#);
        let p = resolve(Some("prj_flag"), None, envs(&[]), d.path()).unwrap();
        assert_eq!(p.team, None);
    }

    #[test]
    fn a_team_flag_applies_to_the_linked_project() {
        let d = tempfile::tempdir().unwrap();
        linked(d.path(), r#"{"projectId":"prj_file","orgId":"team_file"}"#);
        let p = resolve(None, Some("team_flag"), envs(&[]), d.path()).unwrap();
        assert_eq!(p.team.as_deref(), Some("team_flag"));
        assert_eq!(p.project, "prj_file");
    }

    #[test]
    fn nothing_found_names_all_three_ways() {
        let d = tempfile::tempdir().unwrap();
        let e = resolve(None, None, envs(&[]), d.path())
            .unwrap_err()
            .to_string();
        for needle in ["--project", ENV_PROJECT, ENV_TEAM, ".vercel/project.json"] {
            assert!(e.contains(needle), "{needle} missing from {e}");
        }
    }

    #[test]
    fn a_broken_link_file_is_an_error_only_when_it_is_needed() {
        let d = tempfile::tempdir().unwrap();
        linked(d.path(), "{ nope");
        assert!(resolve(Some("prj_x"), None, envs(&[]), d.path()).is_ok());
        let e = resolve(None, None, envs(&[]), d.path())
            .unwrap_err()
            .to_string();
        assert!(e.contains("not valid JSON"), "{e}");
        linked(d.path(), r#"{"orgId":"team_x"}"#);
        assert!(resolve(None, None, envs(&[]), d.path())
            .unwrap_err()
            .to_string()
            .contains("no projectId"));
    }

    #[test]
    fn an_id_that_could_change_the_path_is_refused() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["a/b", "..", "a?b", "a b", "a#b"] {
            assert!(
                resolve(Some(bad), None, envs(&[]), d.path()).is_err(),
                "{bad}"
            );
        }
        assert!(resolve(Some("prj_Ab-1.x"), Some("my-team"), envs(&[]), d.path()).is_ok());
    }

    #[test]
    fn blank_values_count_as_unset() {
        let d = tempfile::tempdir().unwrap();
        assert!(resolve(Some("  "), None, envs(&[(ENV_PROJECT, "")]), d.path()).is_err());
    }
}
