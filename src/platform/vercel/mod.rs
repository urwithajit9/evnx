//! `evnx platform diff|pull|push --to vercel`: compare a local `.env` with a Vercel
//! environment, or pull Vercel's readable values into it.
//!
//! Vercel has two kinds of variable now. A **Config** value can be read back; a
//! **Secret** can never be, so `pull` cannot fetch it and `diff` can say only
//! that it exists. Values are never printed, by either command.
//!
//! `diff` and `pull` only read Vercel; `push` writes to it (see [`push`]). Deleting
//! what Vercel has and the local file lacks is `--prune`, a later phase.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::cli::PlatformCommon;

pub mod api;
pub mod auth;
pub mod diff;
pub mod plan;
pub mod project;
pub mod pull;
pub mod push;
pub mod state;

use crate::utils::protect::GitignoreMode;
use api::{Api, Kind, Listing, Team};
use plan::{Environment, Remote, Selection};
use project::ProjectRef;

/// Everything a command needs before it talks to Vercel.
pub struct Session {
    /// The HTTP client, token inside.
    pub api: Api,
    /// The project.
    pub project: ProjectRef,
    /// The Vercel environment.
    pub environment: Environment,
}

impl Session {
    /// Resolve project, token and API base. Nothing is sent.
    pub fn open(common: &PlatformCommon) -> Result<Session> {
        let env = |k: &str| std::env::var(k).ok();
        let project = project::resolve(
            common.project.as_deref(),
            common.team.as_deref(),
            env,
            Path::new("."),
        )?;
        let token = auth::resolve(common.token_file.as_deref(), env)?;
        let base = api::parse_base(common.api_url.as_deref().unwrap_or(api::DEFAULT_BASE))?;
        let team = project.team.as_deref().map(Team::from_ref);
        Ok(Session {
            api: Api::new(base, token, team)?,
            project,
            environment: common.env_name,
        })
    }

    /// List the project's variables and pick the chosen environment's entries.
    pub fn list(&self) -> Result<(Selection, Listing)> {
        let mut listing = self.api.list_envs(&self.project.project)?;
        let sel = plan::select(std::mem::take(&mut listing.vars), self.environment);
        Ok((sel, listing))
    }

    /// Read the value of every Config entry `wanted` accepts. A Config entry for
    /// which Vercel returns no value is treated as a Secret.
    pub fn read(
        &self,
        entries: &[api::RemoteVar],
        wanted: impl Fn(&str) -> bool,
    ) -> Result<Vec<Remote>> {
        let mut out = Vec::with_capacity(entries.len());
        for var in entries {
            let mut remote = Remote {
                var: var.clone(),
                kind: var.kind,
                value: None,
                read: false,
            };
            if var.kind == Kind::Config && wanted(&var.key) {
                remote.read = true;
                remote.value = self.api.read_value(&self.project.project, &var.id)?;
                if remote.value.is_none() {
                    remote.kind = Kind::Secret;
                }
            }
            out.push(remote);
        }
        Ok(out)
    }

    /// Warnings every command shares, on stderr: things the listing could not
    /// show. Names a count, never a value.
    pub fn warn(&self, sel: &Selection, listing: &Listing) {
        if listing.hidden_production > 0 {
            eprintln!(
                "warning: {} production variable{} hidden from this token, so not listed or compared.",
                listing.hidden_production,
                if listing.hidden_production == 1 { " is" } else { "s are" }
            );
        }
        if sel.skipped_branch > 0 {
            eprintln!(
                "warning: {} preview variable{} scoped to one git branch skipped (branch scoping is not supported yet).",
                sel.skipped_branch,
                if sel.skipped_branch == 1 { " is" } else { "s are" }
            );
        }
        for key in &sel.ambiguous {
            eprintln!(
                "warning: {key} has more than one entry for {}; the most recently updated is used.",
                self.environment.name()
            );
        }
    }
}

/// Write the D4 state file, then see that git will not pick it up (F4).
///
/// The state is per-machine, so `.evnx/platform/` belongs in `.gitignore`. When
/// it is not ignored: `--gitignore` adds it; on a terminal without `--yes` the
/// same yes/no question the env files get is asked; otherwise (CI, `--yes`) it
/// stays a hint on stderr. `--no-gitignore` says nothing.
pub fn record(
    session: &Session,
    entries: &[api::RemoteVar],
    gitignore: &GitignoreMode,
    yes: bool,
) -> Result<()> {
    let root = Path::new(".");
    let written = state::write(root, &session.project, session.environment, entries)?;
    crate::say!(
        "  {} recorded {} (ids only, no values)",
        "✓".green(),
        shown(&written)
    );
    protect_state(root, gitignore, yes)
}

/// The entry `.gitignore` gets for the state file.
const STATE_DIR_ENTRY: &str = ".evnx/platform/";

fn protect_state(root: &Path, mode: &GitignoreMode, yes: bool) -> Result<()> {
    use crate::core::gitignore;
    if matches!(mode, GitignoreMode::Skip) || !gitignore::in_work_tree(root) {
        return Ok(());
    }
    match gitignore::exposure(root, state::PATH) {
        gitignore::Exposure::Protected => return Ok(()),
        gitignore::Exposure::Tracked => {
            crate::utils::protect::warn_tracked(state::PATH);
            return Ok(());
        }
        gitignore::Exposure::Unignored => {}
    }
    let add = match mode {
        GitignoreMode::Auto => true,
        GitignoreMode::Skip => false,
        GitignoreMode::Default => {
            !yes && crate::utils::term::can_prompt() && {
                // Esc, or a prompt that cannot be drawn, is a no: the hint stays.
                crate::utils::prompt::yes_no()
                    .with_prompt(format!(
                        "'{STATE_DIR_ENTRY}' is per-machine state — add it to .gitignore?"
                    ))
                    .default(true)
                    .interact_opt()
                    .ok()
                    .flatten()
                    .unwrap_or(false)
            }
        }
    };
    if add {
        let cwd = std::env::current_dir()?;
        if let Some(file) = gitignore::find_gitignore_path(&cwd) {
            let git_root = file.parent().unwrap_or(&cwd);
            let rel = gitignore::relative_output_path(git_root, &cwd.join(STATE_DIR_ENTRY));
            if let Some(rel) = rel {
                let entry = format!("{}/", rel.to_string_lossy().trim_end_matches('/'));
                gitignore::append_entry(&file, &entry)?;
                crate::say!("  {} added '{entry}' to .gitignore", "✓".green());
                return Ok(());
            }
        }
    }
    eprintln!(
        "hint: {} is per-machine state; add `{STATE_DIR_ENTRY}` to .gitignore to keep it out of commits.",
        state::PATH
    );
    Ok(())
}

/// The path as the person typed it: `./.env` is shown as `.env`.
pub fn shown(path: &Path) -> String {
    path.strip_prefix(".").unwrap_or(path).display().to_string()
}
