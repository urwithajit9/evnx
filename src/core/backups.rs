//! What evnx knows about the backups it keeps (WP-7: VAL-2, REM-4, REM-6,
//! CLD-6, FLD-9).
//!
//! [`crate::core::safe_write::backup`] keeps one `<file>.bak` per file: an
//! exact byte copy, so `mv .env.bak .env` undoes the last change. A copy of an
//! env file is a second copy of its secrets, so after writing one this module:
//!
//! - makes sure git ignores it: `*.bak` (and the registry below) in the
//!   repository's `.gitignore`, created if needed, unless the user's own rules
//!   already cover them. Outside a repository there is nothing to ignore.
//! - records it in `.evnx/backups.json` at the repository's root (or the
//!   file's folder outside one): which file, which backup, when, and which
//!   command. One entry per backup file, replaced with it, so the list never
//!   grows past the files evnx changed. Nothing secret is in it.
//!
//! Both are best effort: a backup that was written is never undone because
//! the bookkeeping failed. A failure is one warning line.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Where the registry lives, relative to the root it describes.
pub const REGISTRY: &str = ".evnx/backups.json";

/// One backup evnx keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The file that was changed, relative to the registry's root.
    pub file: String,
    /// Its backup (`<file>.bak`), relative to the same root.
    pub backup: String,
    /// When the backup was written, RFC 3339 (UTC).
    pub time: String,
    /// The evnx command that wrote it: `remove`, `validate`, `sync` …
    pub command: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    version: u32,
    backups: Vec<Entry>,
}

/// The folder a registry for `file` belongs to: the repository holding it, or
/// its own folder outside one.
pub fn root_for(file: &Path) -> PathBuf {
    let abs = located(file);
    let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    repo_root(&dir).unwrap_or(dir)
}

/// `file` with its folder made absolute, and the file itself **not** followed:
/// a `.env` that is a symlink is recorded, and its backup kept, where the link
/// is (`safe_write::backup` writes `<link>.bak`), not where it points.
fn located(file: &Path) -> PathBuf {
    let dir = match file.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    match file.file_name() {
        Some(name) => dir.join(name),
        None => dir,
    }
}

fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// After `backup` (the copy of `file`) was written: record it, and with
/// `ignore`, make git ignore it. Never fails; a problem is one warning.
pub fn after_backup(file: &Path, backup: &Path, ignore_it: bool) {
    let root = root_for(file);
    if ignore_it && repo_root(&root).is_some() {
        if let Err(e) = ignore(&root) {
            crate::utils::ui::warning(format!(
                "could not update .gitignore for {}: {e:#}",
                backup.display()
            ));
        }
    }
    if let Err(e) = record(&root, file, backup) {
        crate::utils::ui::warning(format!(
            "could not record {} in {REGISTRY}: {e:#}",
            backup.display()
        ));
    }
}

/// `*.bak` and the registry, in the repository's `.gitignore`, unless git
/// already ignores them.
fn ignore(root: &Path) -> Result<()> {
    let wanted: Vec<&str> = [crate::utils::protect::BACKUP_PATTERN, REGISTRY]
        .into_iter()
        .filter(|entry| {
            // A name that stands for the pattern: git's own answer, so the
            // user's rules (`*.bak`, `.evnx/`) count.
            let probe = if *entry == crate::utils::protect::BACKUP_PATTERN {
                ".evnx-probe.bak"
            } else {
                entry
            };
            !crate::core::gitignore::is_protected(root, probe)
        })
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let added = crate::core::gitignore::append_entries(&root.join(".gitignore"), &wanted)?;
    if !added.is_empty() && !crate::utils::ui::is_quiet() {
        eprintln!(
            "  .gitignore: added {} (a backup holds the same secrets as the file)",
            added.join(", ")
        );
    }
    Ok(())
}

fn record(root: &Path, file: &Path, backup: &Path) -> Result<()> {
    let path = root.join(REGISTRY);
    let mut reg: Registry = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Registry::default(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let rel = |p: &Path| name_in(root, p);
    let entry = Entry {
        file: rel(file),
        backup: rel(backup),
        time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        command: command(),
    };
    reg.version = 1;
    reg.backups.retain(|e| e.backup != entry.backup);
    reg.backups.push(entry);
    reg.backups.sort_by(|a, b| a.backup.cmp(&b.backup));
    crate::core::safe_write::create_dirs_private(path.parent().expect("a parent"))?;
    let text = serde_json::to_string_pretty(&reg)? + "\n";
    crate::core::safe_write::write_private(&path, text.as_bytes())?;
    Ok(())
}

/// The registry's root for a command run in `dir`: its repository, or `dir`.
pub fn root_of_dir(dir: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    repo_root(&abs).unwrap_or(abs)
}

/// `file` as the registry names it: relative to `root`, `/`-separated.
pub fn name_in(root: &Path, file: &Path) -> String {
    let abs = located(file);
    abs.strip_prefix(root)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.to_string_lossy().into_owned())
}

/// Drop the entries whose backup is in `backups` (names as in the registry).
pub fn forget(root: &Path, backups: &[String]) -> Result<()> {
    let path = root.join(REGISTRY);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut reg: Registry = serde_json::from_str(&text).unwrap_or_default();
    reg.backups.retain(|e| !backups.contains(&e.backup));
    let text = serde_json::to_string_pretty(&reg)? + "\n";
    crate::core::safe_write::write_private(&path, text.as_bytes())?;
    Ok(())
}

/// The entries in the registry under `root`, for `doctor` and tests.
pub fn list(root: &Path) -> Vec<Entry> {
    std::fs::read_to_string(root.join(REGISTRY))
        .ok()
        .and_then(|t| serde_json::from_str::<Registry>(&t).ok())
        .map(|r| r.backups)
        .unwrap_or_default()
}

/// Whether `name` (a file name) is named like a backup: `*.bak`
/// ([`crate::utils::protect::BACKUP_PATTERN`], what evnx writes), or a
/// `.bak-<anything>` or `.bak.<n>` part, as in `.env.bak-20261120` and
/// `.env.bak.2`.
pub fn is_backup_name(name: &str) -> bool {
    crate::core::glob::glob_match(crate::utils::protect::BACKUP_PATTERN, name)
        || name
            .split('.')
            .skip(1)
            .any(|part| part == "bak" || part.starts_with("bak-"))
}

/// The files in `dir` the registry records as backups, by name.
pub fn recorded_in(dir: &Path) -> Vec<String> {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let root = root_of_dir(dir);
    list(&root)
        .into_iter()
        .filter_map(|e| {
            let path = root.join(&e.backup);
            (path.parent() == Some(abs.as_path()))
                .then(|| path.file_name()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect()
}

/// The plaintext backups of `file` beside it: named like one
/// ([`is_backup_name`], with `file`'s name before the `.bak`), or recorded as
/// one of it in the registry. File names, sorted. `validate` leaves them out
/// of the comparison between environment files and names them once.
pub fn backups_of(file: &Path) -> Vec<String> {
    let Some(own) = file.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let dir = match file.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let root = root_of_dir(dir);
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut out: Vec<String> = list(&root)
        .into_iter()
        .filter(|e| {
            let f = root.join(&e.file);
            f.parent() == Some(abs.as_path()) && f.file_name().and_then(|n| n.to_str()) == Some(own)
        })
        .filter_map(|e| {
            let path = root.join(&e.backup);
            (path.parent() == Some(abs.as_path()) && path.is_file())
                .then(|| path.file_name()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            // `.env.bak…`, not `.env.production.bak`: that one is a backup
            // of `.env.production`.
            let copy_of_own = name
                .strip_prefix(own)
                .is_some_and(|rest| rest.starts_with(".bak"))
                && is_backup_name(&name);
            if copy_of_own && entry.file_type().is_ok_and(|t| t.is_file()) {
                out.push(name);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The running command's name: the first argument that is not a flag.
fn command() -> String {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| "evnx".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_recorded_once_per_file_and_ignored_in_a_repository() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        let env = d.path().join(".env");
        let bak = d.path().join(".env.bak");
        std::fs::write(&env, "A=1\n").unwrap();
        std::fs::write(&bak, "A=0\n").unwrap();
        after_backup(&env, &bak, true);
        after_backup(&env, &bak, true);
        let root = std::fs::canonicalize(d.path()).unwrap();
        let list = list(&root);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].file, ".env");
        assert_eq!(list[0].backup, ".env.bak");
        let gi = std::fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert!(gi.contains("*.bak"), "{gi}");
        assert!(gi.contains(REGISTRY), "{gi}");
    }

    #[test]
    fn outside_a_repository_nothing_is_ignored_but_the_backup_is_recorded() {
        let d = tempfile::TempDir::new().unwrap();
        let env = d.path().join(".env");
        let bak = d.path().join(".env.bak");
        std::fs::write(&env, "A=1\n").unwrap();
        std::fs::write(&bak, "A=0\n").unwrap();
        after_backup(&env, &bak, true);
        assert!(!d.path().join(".gitignore").exists());
        assert_eq!(list(&std::fs::canonicalize(d.path()).unwrap()).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_recorded_where_the_link_is() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::fs::write(d.path().join("real/.env.dev"), "A=1\n").unwrap();
        std::os::unix::fs::symlink("real/.env.dev", d.path().join(".env.dev")).unwrap();
        std::fs::write(d.path().join(".env.dev.bak"), "A=0\n").unwrap();
        after_backup(
            &d.path().join(".env.dev"),
            &d.path().join(".env.dev.bak"),
            true,
        );
        let root = std::fs::canonicalize(d.path()).unwrap();
        let list = list(&root);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].file, ".env.dev");
        assert!(!d.path().join("real/.evnx").exists());
    }

    #[test]
    fn the_users_own_rule_is_enough() {
        let d = tempfile::TempDir::new().unwrap();
        // A real repository: `.evnx/` covering the registry is git's answer.
        let git = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status();
        if !git.is_ok_and(|s| s.success()) {
            return;
        }
        std::fs::write(d.path().join(".gitignore"), "*.bak\n.evnx/\n").unwrap();
        let env = d.path().join(".env");
        let bak = d.path().join(".env.bak");
        std::fs::write(&env, "A=1\n").unwrap();
        std::fs::write(&bak, "A=0\n").unwrap();
        after_backup(&env, &bak, true);
        assert_eq!(
            std::fs::read_to_string(d.path().join(".gitignore")).unwrap(),
            "*.bak\n.evnx/\n"
        );
    }
}
