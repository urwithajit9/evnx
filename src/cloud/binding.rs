//! The per-directory vault binding — `[cloud] vault` in `.evnx.toml`.
//!
//! Without this, every push and pull needs `--vault app/production`, which makes
//! the headline command read `evnx cloud push --vault app/production` instead of
//! `evnx cloud push`. A binding records which vault *this directory* syncs with.
//!
//! # Why `.evnx.toml` is the right home, when credentials are not
//!
//! `.evnx.toml` is project-level and routinely committed — which is exactly wrong
//! for a token and exactly right for a binding. A vault name is not a secret, and
//! committing it is the point: a teammate who clones the repo gets the same
//! binding and their `evnx cloud pull` goes to the same place.
//!
//! # The search deliberately differs from `crate::core::config`
//!
//! That one falls back to `$HOME/.evnx.toml`. A binding must not: a vault named
//! in the home directory would silently apply to *every* project on the machine,
//! so a push from an unrelated directory would go somewhere surprising. This
//! search walks upward from the current directory and **stops at a `.git`
//! boundary**, so a parent repository's binding cannot leak into a nested one.
//!
//! # Writes preserve the file
//!
//! Editing goes through `toml_edit`, not deserialize-mutate-reserialize. A user's
//! `.evnx.toml` holds their comments, their key order, and settings this module
//! knows nothing about; rewriting it from a typed struct would quietly discard
//! all three.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// File that carries the binding.
pub const CONFIG_FILE: &str = ".evnx.toml";

/// Locate the `.evnx.toml` that governs `start`, if any.
///
/// Walks upward, stopping once a directory containing `.git` has been examined —
/// the project boundary. Returns `None` rather than reaching `$HOME`.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Check for the boundary *after* looking in this directory, so a repo
        // root holding the config is still found.
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent();
    }
    None
}

/// The bound vault for `start`, if one is recorded.
///
/// A `.evnx.toml` that cannot be parsed is an error rather than a silent `None`:
/// pushing to the default vault because a binding had a typo is precisely the
/// surprise this feature exists to avoid.
pub fn read(start: &Path) -> Result<Option<Bound>> {
    let Some(path) = find_config(start) else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let Some(vault) = doc
        .get("cloud")
        .and_then(|c| c.get("vault"))
        .and_then(|v| v.as_str())
    else {
        return Ok(None);
    };

    if vault.trim().is_empty() {
        return Err(anyhow!(
            "{} has an empty `[cloud] vault`. Set it with `evnx cloud link <vault>`, \
             or remove the line.",
            path.display()
        ));
    }

    Ok(Some(Bound {
        vault: vault.trim().to_string(),
        path,
    }))
}

/// A binding and the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// Vault spelling as written — `name` or `name/environment`.
    pub vault: String,
    /// File the binding was read from, for messages.
    pub path: PathBuf,
}

/// Record a binding, creating `.evnx.toml` in `dir` if there is none to edit.
///
/// Everything already in the file survives: comments, ordering, and any keys this
/// module does not understand.
pub fn write(dir: &Path, vault: &str) -> Result<PathBuf> {
    let path = find_config(dir).unwrap_or_else(|| dir.join(CONFIG_FILE));

    let mut doc: toml_edit::DocumentMut = match std::fs::read_to_string(&path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    if !doc.contains_key("cloud") {
        let mut table = toml_edit::Table::new();
        table.decor_mut().set_prefix(
            "\n# Which evnx cloud vault this directory syncs with.\n\
             # Safe to commit: a vault name is not a secret, and sharing it means\n\
             # a teammate's `evnx cloud pull` lands in the same place.\n",
        );
        doc["cloud"] = toml_edit::Item::Table(table);
    }
    doc["cloud"]["vault"] = toml_edit::value(vault);

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Remove a binding. Returns the file it was removed from, if there was one.
///
/// Only the `vault` key is removed, and the `[cloud]` table only if it is then
/// empty — the file may hold settings that have nothing to do with this.
pub fn clear(dir: &Path) -> Result<Option<PathBuf>> {
    let Some(path) = find_config(dir) else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let had = doc
        .get_mut("cloud")
        .and_then(|c| c.as_table_mut())
        .map(|t| t.remove("vault").is_some())
        .unwrap_or(false);

    if !had {
        return Ok(None);
    }

    if doc
        .get("cloud")
        .and_then(|c| c.as_table())
        .is_some_and(|t| t.is_empty())
    {
        doc.remove("cloud");
    }

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_binding_round_trips() {
        let d = tmp();
        let p = write(d.path(), "app/production").unwrap();
        assert_eq!(p, d.path().join(CONFIG_FILE));
        let bound = read(d.path()).unwrap().unwrap();
        assert_eq!(bound.vault, "app/production");
    }

    #[test]
    fn writing_preserves_comments_and_unrelated_settings() {
        // The user's file is theirs. Deserialize-mutate-reserialize would drop
        // every comment and any key this module does not model.
        let d = tmp();
        let path = d.path().join(CONFIG_FILE);
        std::fs::write(
            &path,
            "# my project settings\n\
             [defaults]\n\
             env_file = \".env.local\"  # not the default\n\n\
             [scan]\n\
             ignore_placeholders = true\n",
        )
        .unwrap();

        write(d.path(), "app/staging").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        assert!(after.contains("# my project settings"), "{after}");
        assert!(after.contains("# not the default"), "{after}");
        assert!(after.contains("env_file = \".env.local\""), "{after}");
        assert!(after.contains("ignore_placeholders = true"), "{after}");
        assert!(after.contains("vault = \"app/staging\""), "{after}");
    }

    #[test]
    fn rebinding_replaces_rather_than_duplicates() {
        let d = tmp();
        write(d.path(), "app/production").unwrap();
        write(d.path(), "app/staging").unwrap();
        let text = std::fs::read_to_string(d.path().join(CONFIG_FILE)).unwrap();
        assert_eq!(text.matches("vault =").count(), 1, "{text}");
        assert_eq!(read(d.path()).unwrap().unwrap().vault, "app/staging");
    }

    #[test]
    fn clearing_leaves_other_settings_alone() {
        let d = tmp();
        let path = d.path().join(CONFIG_FILE);
        std::fs::write(&path, "[defaults]\nenv_file = \".env\"\n").unwrap();
        write(d.path(), "app/production").unwrap();

        assert!(clear(d.path()).unwrap().is_some());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("env_file"), "{after}");
        assert!(!after.contains("vault"), "{after}");
        assert!(!after.contains("[cloud]"), "empty table should go: {after}");
        assert!(read(d.path()).unwrap().is_none());
    }

    #[test]
    fn clearing_keeps_a_cloud_table_that_still_has_content() {
        let d = tmp();
        let path = d.path().join(CONFIG_FILE);
        std::fs::write(&path, "[cloud]\nvault = \"a/b\"\nsomething_else = 1\n").unwrap();
        clear(d.path()).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("something_else"), "{after}");
        assert!(!after.contains("vault"), "{after}");
    }

    #[test]
    fn clearing_when_there_is_nothing_to_clear_reports_none() {
        let d = tmp();
        assert!(clear(d.path()).unwrap().is_none());
        std::fs::write(d.path().join(CONFIG_FILE), "[defaults]\n").unwrap();
        assert!(clear(d.path()).unwrap().is_none());
    }

    #[test]
    fn the_search_walks_up_to_the_project_root() {
        let d = tmp();
        let nested = d.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        write(d.path(), "app/production").unwrap();
        assert_eq!(read(&nested).unwrap().unwrap().vault, "app/production");
    }

    #[test]
    fn the_search_stops_at_a_git_boundary() {
        // A parent repository's binding must not govern a nested project, or a
        // push from inside it would go somewhere the directory never declared.
        let d = tmp();
        write(d.path(), "outer/production").unwrap();

        let inner = d.path().join("vendor/inner");
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        assert!(
            read(&inner).unwrap().is_none(),
            "the outer binding leaked past a .git boundary"
        );
    }

    #[test]
    fn a_config_at_the_repo_root_is_still_found() {
        // The boundary is checked after looking in the directory, so the repo
        // root's own .evnx.toml counts.
        let d = tmp();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        write(d.path(), "app/production").unwrap();
        let nested = d.path().join("src");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(read(&nested).unwrap().unwrap().vault, "app/production");
    }

    #[test]
    fn an_empty_binding_is_an_error_not_a_silent_fallback() {
        let d = tmp();
        std::fs::write(d.path().join(CONFIG_FILE), "[cloud]\nvault = \"  \"\n").unwrap();
        let err = read(d.path()).unwrap_err().to_string();
        assert!(err.contains("evnx cloud link"), "{err}");
    }

    #[test]
    fn a_file_with_no_cloud_section_simply_has_no_binding() {
        let d = tmp();
        std::fs::write(
            d.path().join(CONFIG_FILE),
            "[defaults]\nenv_file = \".env\"\n",
        )
        .unwrap();
        assert!(read(d.path()).unwrap().is_none());
    }

    #[test]
    fn a_malformed_file_is_reported_rather_than_ignored() {
        let d = tmp();
        std::fs::write(d.path().join(CONFIG_FILE), "[cloud\nvault =").unwrap();
        assert!(read(d.path()).is_err());
    }
}
