//! `evnx history` and `evnx undo`: the copies evnx keeps before it changes a
//! file (WP-7, FLD-9; names chosen by the owner, 2026-11-16).
//!
//! Every command that rewrites an env file in place (`remove`, `validate
//! --fix`, `diff --apply`, `restore`, `cloud pull`, `platform pull`) first
//! keeps the old text as `<file>.bak` (one slot per file) and records it in
//! `.evnx/backups.json` ([`crate::core::backups`]).
//!
//! - `history` lists those copies: file, command, time. Never a value.
//! - `undo FILE` **swaps** the file and its copy: the file gets the copy's
//!   text, and the copy gets what the file held, so running it again puts
//!   the newer text back. Nothing is lost either way.
//! - `undo --clear [FILE]` deletes the copies (they are a second plaintext
//!   set of the secrets) and forgets them.
//!
//! `evnx restore` is a different thing: it decrypts an archive written by
//! `evnx backup`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::core::backups::{self, Entry};
use crate::core::verdict;
use crate::utils::ui;

/// What `evnx undo` takes.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The file to put back; `None` takes the only one recorded.
    pub file: Option<String>,
    pub yes: bool,
    pub dry_run: bool,
    /// `--clear`: delete the copies instead.
    pub clear: bool,
}

/// `evnx history`.
pub fn history(json: bool) -> Result<()> {
    let root = backups::root_of_dir(Path::new("."));
    let entries = backups::list(&root);
    if json {
        let doc: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "file": e.file,
                    "copy": e.backup,
                    "command": e.command,
                    "time": e.time,
                    "exists": root.join(&e.backup).exists(),
                })
            })
            .collect();
        crate::core::json::emit(&serde_json::json!({ "backups": doc }), 1, None)?;
        return Ok(());
    }
    if entries.is_empty() {
        println!(
            "  No copies kept here. evnx keeps one (<file>.bak) when a command changes an env file."
        );
        return Ok(());
    }
    let w = entries
        .iter()
        .map(|e| e.file.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let c = entries
        .iter()
        .map(|e| e.command.len())
        .max()
        .unwrap_or(7)
        .max(7);
    println!("  {:<w$}  {:<c$}  {:<16}  COPY", "FILE", "COMMAND", "WHEN");
    for e in &entries {
        let gone = if root.join(&e.backup).exists() {
            String::new()
        } else {
            "  (copy gone)".dimmed().to_string()
        };
        println!(
            "  {:<w$}  {:<c$}  {:<16}  {}{gone}",
            e.file,
            e.command,
            local_time(&e.time),
            e.backup
        );
    }
    println!();
    println!(
        "  {}",
        "Put one back: evnx undo FILE   Delete the copies: evnx undo --clear".dimmed()
    );
    Ok(())
}

/// `evnx undo`.
pub fn run(opts: Options) -> Result<()> {
    let root = backups::root_of_dir(Path::new("."));
    if opts.clear {
        return clear(&root, &opts);
    }
    let file = which(&root, opts.file.as_deref())?;
    let copy = PathBuf::from(format!("{}.bak", file.display()));
    let entry = entry_for(&root, &file);
    if !copy.exists() {
        return Err(verdict::stopped(format!(
            "there is no copy of {} to put back ({} does not exist).\n  \
             evnx history lists the copies there are.",
            file.display(),
            copy.display()
        )));
    }

    let old = std::fs::read(&copy).with_context(|| format!("reading {}", copy.display()))?;
    let current = match std::fs::read(&file) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };

    // What changes, by name: never a value.
    let by = entry
        .as_ref()
        .map(|e| format!(" (kept by `{}`, {})", e.command, local_time(&e.time)))
        .unwrap_or_default();
    ui::say(format!("  {} ← {}{by}", file.display(), copy.display()));
    let changes = name_changes(current.as_deref(), &old);
    if changes.is_empty() {
        ui::say("    the same names and values".dimmed().to_string());
    }
    for line in &changes {
        ui::say(format!("    {line}"));
    }

    if opts.dry_run {
        ui::say("  Dry run: nothing was written.");
        return Ok(());
    }
    if !opts.yes {
        if !crate::utils::term::can_prompt() {
            return Err(verdict::stopped_needs_yes(format!(
                "undo replaces {} and needs a yes.\n  evnx undo {} --yes",
                file.display(),
                file.display()
            )));
        }
        let ok = crate::utils::prompt::yes_no()
            .with_prompt(format!("Put {} back as it was?", file.display()))
            .default(false)
            .interact()?;
        if !ok {
            ui::say("  Nothing was changed.");
            return Ok(());
        }
    }

    // The swap: the file's new text is staged first, then what it holds now
    // becomes the copy, then the staged text goes in. A failure before the
    // last step leaves the file as it was.
    let is_env = file
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| crate::core::env_name::kind(n) == crate::core::env_name::FileKind::Env);
    let staged = if is_env {
        crate::core::safe_write::stage_private(&file, &old)?
    } else {
        crate::core::safe_write::stage_shared(&file, &old)?
    };
    match &current {
        Some(now) => {
            crate::core::safe_write::backup(&file, now)?;
        }
        None => {
            std::fs::remove_file(&copy).with_context(|| format!("removing {}", copy.display()))?;
            let _ = backups::forget(&root, &[backups::name_in(&root, &copy)]);
        }
    }
    let written = staged.commit()?;
    crate::core::safe_write::report(&written);
    ui::success(format!(
        "{} is back as it was{}",
        file.display(),
        entry
            .as_ref()
            .map(|e| format!(" before `{}`", e.command))
            .unwrap_or_default()
    ));
    if current.is_some() {
        ui::say(format!(
            "  {}",
            format!(
                "what it held is now {}; evnx undo {} again puts it back",
                copy.display(),
                file.display()
            )
            .dimmed()
        ));
    }
    Ok(())
}

/// `undo --clear [FILE]`: delete the copies and forget them.
fn clear(root: &Path, opts: &Options) -> Result<()> {
    let mut copies: Vec<PathBuf> = match &opts.file {
        Some(f) => vec![PathBuf::from(format!("{f}.bak"))],
        None => backups::list(root)
            .iter()
            .map(|e| shown(root, &e.backup))
            .collect(),
    };
    copies.retain(|c| c.exists());
    if copies.is_empty() {
        ui::say("  No copies to delete.");
        return Ok(());
    }
    let names: Vec<String> = copies.iter().map(|c| c.display().to_string()).collect();
    ui::say(format!(
        "  {} plaintext {}: {}",
        copies.len(),
        if copies.len() == 1 { "copy" } else { "copies" },
        names.join(", ")
    ));
    if opts.dry_run {
        ui::say("  Dry run: nothing was deleted.");
        return Ok(());
    }
    if !opts.yes {
        if !crate::utils::term::can_prompt() {
            return Err(verdict::stopped_needs_yes(
                "deleting the copies needs a yes.\n  evnx undo --clear --yes".to_string(),
            ));
        }
        let ok = crate::utils::prompt::yes_no()
            .with_prompt("Delete them? They cannot be put back afterwards.")
            .default(false)
            .interact()?;
        if !ok {
            ui::say("  Nothing was deleted.");
            return Ok(());
        }
    }
    for c in &copies {
        std::fs::remove_file(c).with_context(|| format!("deleting {}", c.display()))?;
    }
    // By name as recorded, however the path was spelled.
    let recorded: Vec<String> = backups::list(root)
        .into_iter()
        .filter(|e| !shown(root, &e.backup).exists())
        .map(|e| e.backup)
        .collect();
    backups::forget(root, &recorded)?;
    ui::success(format!(
        "deleted {} {}",
        copies.len(),
        if copies.len() == 1 { "copy" } else { "copies" }
    ));
    Ok(())
}

/// The file to put back: the one named, or the only one recorded.
fn which(root: &Path, named: Option<&str>) -> Result<PathBuf> {
    if let Some(f) = named {
        return Ok(PathBuf::from(f));
    }
    let entries = backups::list(root);
    match entries.as_slice() {
        [only] => Ok(shown(root, &only.file)),
        [] => Err(verdict::stopped(
            "there is no copy to put back here.\n  evnx history lists the copies evnx kept."
                .to_string(),
        )),
        many => {
            let files: Vec<&str> = many.iter().map(|e| e.file.as_str()).collect();
            Err(verdict::stopped(format!(
                "copies of {} are kept; name the file to put back:\n  evnx undo {}",
                files.join(", "),
                files[0]
            )))
        }
    }
}

/// A registry name as a path from where the command runs: as written when
/// that is the registry's root, else joined to it.
fn shown(root: &Path, name: &str) -> PathBuf {
    let here = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default();
    match root.strip_prefix(&here) {
        Ok(rest) => rest.join(name),
        Err(_) => root.join(name),
    }
}

fn entry_for(root: &Path, file: &Path) -> Option<Entry> {
    let name = backups::name_in(root, file);
    backups::list(root).into_iter().find(|e| e.file == name)
}

/// `+ NAME  back`, `- NAME  goes`, `~ NAME  value changes`: from `current`
/// to `old`, by name.
fn name_changes(current: Option<&[u8]>, old: &[u8]) -> Vec<String> {
    let read = |b: &[u8]| -> indexmap::IndexMap<String, crate::core::secret::Secret> {
        let text = String::from_utf8_lossy(b);
        let doc = crate::commands::remove::edit::document(&text);
        doc.entries.into_iter().map(|e| (e.key, e.value)).collect()
    };
    let now = current.map(read).unwrap_or_default();
    let then = read(old);
    let mut out = Vec::new();
    for (k, v) in &then {
        match now.get(k) {
            None => out.push(format!("{} {k}  back", "+".green())),
            Some(n) if n != v => out.push(format!("{} {k}  value changes", "~".yellow())),
            _ => {}
        }
    }
    for k in now.keys().filter(|k| !then.contains_key(*k)) {
        out.push(format!("{} {k}  goes", "-".red()));
    }
    out
}

/// RFC 3339 in UTC → local `2026-11-16 19:31`; the text as is when unreadable.
fn local_time(t: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(t)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| t.to_string())
}
