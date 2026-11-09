//! `evnx sync --prune` (N14-11, plan §9 phase 2): take out of the template the
//! names that no environment file has.
//!
//! Lives under `commands/remove` because it deletes entries, and only this
//! module tree may (`tests/architecture.rs::r1_only_remove_deletes_env_entries`).
//! It shares `remove`'s planner and writer: the same editor ([`edit::cut_entries`],
//! which re-reads its result), the same two-phase apply with compare-and-swap,
//! the same prompt, and the same exit codes (`4` for a "no", `2` for a refusal).
//!
//! # What it compares
//!
//! "No environment file has it" means every environment file: `.env` and each
//! `.env.<name>` that [`env_name::available`] lists (backups and the template
//! family are not environments). `--env`, `--env-name` and `--direction` do not
//! apply and are refused by clap: a name one environment lacks may live in
//! another, so pruning against a single file would delete live names.
//!
//! # What it never removes
//!
//! * A name declared in `.evnx.toml [vars]`. The contract says the template
//!   should declare it, and `spec check` would fail afterwards. Such a name is
//!   listed as kept, with the `evnx remove NAME --contract --template` that
//!   drops both.
//! * Anything, when no environment file exists or every one is empty: that is
//!   a clone without its `.env`, and "nothing has it" is then true of
//!   everything.
//! * Anything, when an environment file has a line evnx cannot read: a key
//!   hidden behind it would be pruned wrongly.
//!
//! The template is the only file written. It is committed, so, as in `remove`,
//! git keeps the copy and no `.bak` is made. No value is printed: the template
//! holds none and the environment files are read only for their names.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::*;

use super::{edit, in_dir, read_place, Change, Kind};
use crate::core::env_name;
use crate::utils::path::label;
use crate::utils::prompt::Declined;
use crate::utils::string::count;
use crate::utils::ui;

/// Everything `evnx sync --prune` takes.
#[derive(Debug, Clone, Default)]
pub struct PruneOptions {
    /// The project folder; `.` from the CLI.
    pub path: String,
    /// The template as named by `--example`, `[defaults] example` or the
    /// default, relative to the project folder.
    pub example: String,
    /// Names `.evnx.toml [vars]` declares: kept.
    pub declared: BTreeSet<String>,
    pub dry_run: bool,
    pub yes: bool,
}

/// Run `sync --prune`.
pub fn run(opts: PruneOptions) -> Result<()> {
    run_with(opts, &mut |_, _| {})
}

/// [`run`], with the hook `remove::run_with` has.
pub fn run_with(opts: PruneOptions, before_commit: &mut dyn FnMut(usize, &Path)) -> Result<()> {
    let project = Path::new(&opts.path);
    if !project.is_dir() {
        bail!("{} is not a folder", label(&opts.path));
    }

    // ── Read ─────────────────────────────────────────────────────────────────
    let template_path = in_dir(project, &opts.example);
    if !template_path.is_file() {
        bail!(
            "there is no template to prune: {} does not exist",
            label(&template_path.to_string_lossy())
        );
    }
    let template = read_place(Kind::Template, &template_path)?;

    let mut env_files: Vec<PathBuf> = Vec::new();
    if project.join(env_name::DEFAULT_FILE).is_file() {
        env_files.push(in_dir(project, env_name::DEFAULT_FILE));
    }
    for n in env_name::available(project) {
        env_files.push(in_dir(project, &format!(".env.{n}")));
    }
    if env_files.is_empty() {
        bail!(
            "there is no environment file (.env or .env.<name>) to compare {} with, so every \
             name in it would look unused. Nothing was changed.",
            template.label
        );
    }
    let mut held: BTreeSet<String> = BTreeSet::new();
    let mut compared: Vec<String> = Vec::new();
    for p in &env_files {
        let shown = label(&p.to_string_lossy());
        held.extend(env_names(p, &shown)?);
        compared.push(shown);
    }
    if held.is_empty() {
        bail!(
            "every environment file ({}) is empty, so every name in {} would look unused. \
             Nothing was changed.",
            compared.join(", "),
            template.label
        );
    }

    // ── Plan ─────────────────────────────────────────────────────────────────
    let stale: BTreeSet<String> = template
        .names
        .iter()
        .filter(|n| !held.contains(*n))
        .cloned()
        .collect();
    let (kept, prune): (BTreeSet<String>, BTreeSet<String>) =
        stale.into_iter().partition(|n| opts.declared.contains(n));

    let mut changes = Vec::new();
    let mut removed_names = BTreeSet::new();
    if !prune.is_empty() {
        let cut = edit::cut_entries(&template.original, &prune, &template.label)?;
        removed_names = prune.clone();
        changes.push(Change {
            place_index: 0,
            text: cut.text,
            removed: cut.removed,
            tables: Vec::new(),
        });
    }
    let places = [template];

    let show_plan = |verb: &str| {
        ui::say(format!(
            "  {:<9} {} against {}",
            "compared",
            places[0].label,
            compared.join(", ")
        ));
        for c in &changes {
            println!("  {:<18} {verb} {}", places[0].label, super::describe(c));
        }
        for n in &kept {
            ui::say(format!(
                "  {}",
                format!(
                    "kept {n}: declared in .evnx.toml [vars]; `evnx remove {n} --template --contract` drops both"
                )
                .dimmed()
            ));
        }
    };

    if changes.is_empty() {
        show_plan("");
        ui::say(format!(
            "  Nothing to prune: every name in {} is in an environment file{}.",
            places[0].label,
            if kept.is_empty() {
                ""
            } else {
                " or declared in [vars]"
            }
        ));
        return Ok(());
    }

    if opts.dry_run {
        show_plan("would remove");
        println!("Dry run: nothing was written.");
        return Ok(());
    }

    // ── Ask ──────────────────────────────────────────────────────────────────
    let everything = places[0].names.iter().all(|n| removed_names.contains(n));
    if !opts.yes {
        show_plan("would remove");
        let question = format!(
            "Remove {} from {}{}?",
            count(removed_names.len(), "unused name"),
            places[0].label,
            if everything { " (this empties it)" } else { "" }
        );
        let example = "evnx sync --prune --yes";
        if !crate::utils::prompt::confirm(&question, false, example)? {
            println!("  Cancelled; nothing was changed.");
            return Err(Declined.into());
        }
    } else {
        ui::say(format!("  template  {}", places[0].label));
    }

    // ── Apply ────────────────────────────────────────────────────────────────
    super::apply(&places, &changes, true, before_commit)?;
    for c in &changes {
        ui::success(format!(
            "{:<18} removed {}",
            places[0].label,
            super::describe(c)
        ));
    }
    for n in &kept {
        ui::say(format!(
            "  kept {n}: declared in .evnx.toml [vars]; `evnx remove {n} --template --contract` drops both"
        ));
    }
    Ok(())
}

/// The names an environment file assigns. Refuses a file it cannot read whole:
/// a line the parser rejects may hide a key, and the template would lose it.
fn env_names(path: &Path, shown: &str) -> Result<BTreeSet<String>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {shown}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("{shown} is not UTF-8 text, so nothing was changed"))?;
    let doc = edit::document(&text);
    if let Some(p) = doc.problems.first() {
        bail!(
            "{shown} has a line evnx cannot read, so a name could be hidden behind it and \
             nothing was changed:\n  {}\n  Fix it first; `evnx validate` lists every problem.",
            p.describe()
        );
    }
    Ok(edit::keys(&doc).into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        for (n, c) in files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        d
    }

    fn opts(d: &tempfile::TempDir) -> PruneOptions {
        PruneOptions {
            path: d.path().to_string_lossy().into_owned(),
            example: ".env.example".into(),
            yes: true,
            ..PruneOptions::default()
        }
    }

    fn read(d: &tempfile::TempDir, n: &str) -> String {
        std::fs::read_to_string(d.path().join(n)).unwrap()
    }

    #[test]
    fn a_name_any_environment_has_stays() {
        let d = project(&[
            (".env", "A=1\n"),
            (".env.staging", "B=1\n"),
            (".env.example", "A=\nB=\nGONE=\n"),
        ]);
        run(opts(&d)).unwrap();
        assert_eq!(read(&d, ".env.example"), "A=\nB=\n");
    }

    #[test]
    fn a_declared_name_is_kept() {
        let d = project(&[(".env", "A=1\n"), (".env.example", "A=\nDECL=\nGONE=\n")]);
        let mut o = opts(&d);
        o.declared.insert("DECL".into());
        run(o).unwrap();
        assert_eq!(read(&d, ".env.example"), "A=\nDECL=\n");
    }

    #[test]
    fn no_environment_file_changes_nothing() {
        let d = project(&[(".env.example", "A=\n")]);
        assert!(run(opts(&d)).is_err());
        assert_eq!(read(&d, ".env.example"), "A=\n");
    }

    #[test]
    fn empty_environments_change_nothing() {
        let d = project(&[(".env", "# nothing\n"), (".env.example", "A=\n")]);
        assert!(run(opts(&d)).is_err());
        assert_eq!(read(&d, ".env.example"), "A=\n");
    }
}
