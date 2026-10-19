//! Keeping a file evnx wrote out of git: one flow for every command that writes
//! real values to a path the person names.
//!
//! Moved out of `commands::template` in 0.10.5 (R-template), where it was the
//! only complete copy. `init` and `add` print their own "still not ignored"
//! lines, and `restore` and `cloud pull --file` have no such step at all; each adopts
//! this module in its own patch (see their `R-` notes). `convert -o` adopted
//! it in 0.10.9 (R-convert).
//!
//! ⚠️ This module asks and prints; [`crate::core::gitignore`] only answers
//! (finds the file, asks git, appends a line). The split is deliberate: a core
//! function that prompted could hang a caller that has no terminal.

use std::path::Path;

use anyhow::Result;

use crate::core::gitignore;
use crate::utils::ui;

/// How a command updates `.gitignore` after writing a file that holds secrets.
///
/// Resolved from the command's flags before the write; terminal detection and
/// the question happen in [`protect_output`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitignoreMode {
    /// `--gitignore`: write the entry without asking. Suitable for CI scripts.
    Auto,
    /// `--no-gitignore`: skip every check and warning.
    Skip,
    /// No flag: ask when there is a terminal, warn only when there is not.
    Default,
}

impl GitignoreMode {
    /// From a command's `--gitignore` / `--no-gitignore` pair (`template`,
    /// `convert`); clap keeps the two exclusive.
    pub fn from_flags(gitignore: bool, no_gitignore: bool) -> Self {
        if gitignore {
            GitignoreMode::Auto
        } else if no_gitignore {
            GitignoreMode::Skip
        } else {
            GitignoreMode::Default
        }
    }
}

/// After `output` was written, make sure git will not pick it up.
///
/// - `Auto`: add it to `.gitignore` without asking.
/// - `Skip`: do nothing and print nothing.
/// - `Default` with a terminal: ask (`Y/n`, answered on Enter).
/// - `Default` without one (CI): warn, change nothing.
///
/// Whatever the mode (except `Skip`), git is asked, not the text of
/// `.gitignore` (I-139, DEC-T11): an exact-line match called `out.yml` covered
/// when a later `!out.yml` re-included it, missed `*.yml` and `config/` (and so
/// added redundant lines), and could not see a file that is already tracked.
///
/// Says nothing when there is no git repository, except under `Auto`, where the
/// flag asked for something that cannot be done. An output outside the
/// repository is a warning.
pub fn protect_output(output: &str, mode: &GitignoreMode) -> Result<()> {
    if matches!(mode, GitignoreMode::Skip) {
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let Some(gitignore_path) = gitignore::find_gitignore_path(&cwd) else {
        if matches!(mode, GitignoreMode::Auto) {
            ui::warning("Not a git repository — cannot update .gitignore");
        }
        return Ok(());
    };

    let git_root = gitignore_path
        .parent()
        .expect(".gitignore path always has a parent directory");

    // Absolute, so `relative_output_path` can strip the repository root.
    let output_path = {
        let p = Path::new(output);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };

    let Some(entry_path) = gitignore::relative_output_path(git_root, &output_path) else {
        ui::warning(format!(
            "'{}' is outside the git repository — .gitignore cannot protect it",
            output
        ));
        return Ok(());
    };
    let entry = entry_path.to_string_lossy().to_string();

    match gitignore::exposure(git_root, &entry) {
        gitignore::Exposure::Protected => {
            ui::success(format!("'{}' is already ignored by git", entry));
            return Ok(());
        }
        gitignore::Exposure::Tracked => {
            warn_tracked(&entry);
            return Ok(());
        }
        gitignore::Exposure::Unignored => {}
    }

    let should_write = match mode {
        GitignoreMode::Auto => true,
        GitignoreMode::Skip => false, // unreachable, handled above
        GitignoreMode::Default => ask_to_add(&entry),
    };

    if should_write {
        gitignore::append_entry(&gitignore_path, &entry)?;
        ui::success(format!("Added '{}' to .gitignore", entry));
        // A later `!entry` (or a nested .gitignore) can still win: say so,
        // and leave the user's line alone (G8).
        if !gitignore::is_protected(git_root, &entry) {
            warn_still_unignored(&entry);
        }
    }

    Ok(())
}

/// `Default` mode: the question, or the warning when it cannot be asked.
///
/// Esc is "no". A prompt that fails (stderr is not a terminal although stdin
/// is) falls back to the warning: the file is already written, so failing the
/// command now would report an error for a run that did its job.
fn ask_to_add(entry: &str) -> bool {
    let unprotected = || {
        ui::warning(format!(
            "'{}' may contain secrets and is not in .gitignore",
            entry
        ));
        false
    };
    if !crate::utils::term::interactive() {
        return unprotected();
    }
    match crate::utils::prompt::yes_no()
        .with_prompt(format!(
            "'{}' may contain secrets — add it to .gitignore?",
            entry
        ))
        .default(true)
        .interact_opt()
    {
        Ok(answer) => answer.unwrap_or(false),
        Err(_) => unprotected(),
    }
}

/// `entry` is tracked by git: `.gitignore` cannot help, `git rm --cached` can.
pub fn warn_tracked(entry: &str) {
    ui::warning(format!(
        "'{entry}' is tracked by git: a commit includes it whatever .gitignore says"
    ));
    ui::warning_detail(format!("     →  git rm --cached {entry}"));
}

/// `entry` was added to `.gitignore` and git still does not ignore it.
pub fn warn_still_unignored(entry: &str) {
    ui::warning(format!(
        "'{entry}' is still not ignored: a later rule re-includes it. \
         Check with: git check-ignore -v {entry}"
    ));
}
