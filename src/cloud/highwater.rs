//! The highest version of each vault this machine has seen (RM-89, a
//! client-only slice of P6).
//!
//! The server says what "latest" is, and a client that believes it cannot tell
//! a vault that was rolled back, restored from an older backup or served a stale
//! replica from one that simply has fewer versions. This file is the memory
//! that lets it: when the server's latest is lower than a version this client
//! has already seen, `push`, `pull`, `run`, `diff` and `export` say so on stderr.
//!
//! Only **vault id → version number** is stored, in `high-water.json` in the
//! config directory. No name, no key, no value, no hash. It is a hint, not a
//! guard: it is per machine, so a new machine starts blank, and a failure to
//! read or write it never fails a command.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::config::config_dir;

/// The file in the config directory.
const FILE: &str = "high-water.json";

fn path() -> Option<PathBuf> {
    config_dir().ok().map(|d| d.join(FILE))
}

fn key(vault_id: &str) -> String {
    vault_id.to_ascii_lowercase()
}

fn load(path: &Path) -> BTreeMap<String, i32> {
    let raw: BTreeMap<String, i32> = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // Ids are compared in lower case (C10); a hand-edited file may not be.
    let mut marks = BTreeMap::new();
    for (id, n) in raw {
        let slot = marks.entry(key(&id)).or_insert(n);
        *slot = (*slot).max(n);
    }
    marks
}

/// Write through a temporary file and a rename, so a crash cannot leave half a
/// file. Owner-only on Unix, as the rest of the config directory is.
fn store(path: &Path, marks: &BTreeMap<String, i32>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let body = serde_json::to_string_pretty(marks).map_err(std::io::Error::other)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// What the server's `latest` means against the mark: the warning to print, if
/// any. Pure, so it is tested without a file.
#[must_use]
pub fn regression(label: &str, seen: i32, latest: i32) -> Option<String> {
    (latest < seen).then(|| {
        format!(
            "the server says {label} is at version {latest}, but this machine has already \
             seen version {seen}.\n\
             \x20 A newer version may have been removed, or the server may be serving older \
             data than it did (a rollback or a restored backup). Check `evnx cloud history \
             --vault {label}` before you rely on this."
        )
    })
}

/// Record that the server reports `latest` for `vault_id`: warn when that is
/// below the mark (the mark is kept, so the warning repeats until the server
/// catches up), otherwise raise the mark.
pub fn observe(vault_id: &str, label: &str, latest: i32) {
    let Some(path) = path() else { return };
    let mut marks = load(&path);
    let seen = marks.get(&key(vault_id)).copied().unwrap_or(0);
    if let Some(msg) = regression(label, seen, latest) {
        crate::utils::ui::warning(msg);
    } else if latest > seen && store_one(&path, &mut marks, vault_id, latest).is_err() {
        // Best effort: a read-only config directory must not fail a push.
    }
}

/// Raise the mark to `version`, which this client has just pushed or fetched.
pub fn note(vault_id: &str, version: i32) {
    let Some(path) = path() else { return };
    let mut marks = load(&path);
    if version > marks.get(&key(vault_id)).copied().unwrap_or(0) {
        let _ = store_one(&path, &mut marks, vault_id, version);
    }
}

/// Drop a vault that was deleted.
pub fn forget(vault_id: &str) {
    let Some(path) = path() else { return };
    let mut marks = load(&path);
    if marks.remove(&key(vault_id)).is_some() {
        let _ = store(&path, &marks);
    }
}

fn store_one(
    path: &Path,
    marks: &mut BTreeMap<String, i32>,
    vault_id: &str,
    version: i32,
) -> std::io::Result<()> {
    marks.insert(key(vault_id), version);
    store(path, marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lower_latest_than_seen_warns_and_an_equal_or_higher_one_does_not() {
        let msg = regression("app/production", 7, 5).expect("regression");
        assert!(
            msg.contains("version 5") && msg.contains("version 7"),
            "{msg}"
        );
        assert!(msg.contains("evnx cloud history --vault app/production"));
        assert!(regression("app/production", 7, 7).is_none());
        assert!(regression("app/production", 7, 9).is_none());
        // Never seen: nothing to compare with, even for an empty vault.
        assert!(regression("app/production", 0, 0).is_none());
    }

    #[test]
    fn the_file_holds_ids_and_numbers_and_survives_a_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join(FILE);
        let mut marks = load(&p);
        assert!(marks.is_empty());
        store_one(&p, &mut marks, "0B8A1F8E-3C2D", 4).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // The id is lowercased, the version is a number, and that is all.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({ "0b8a1f8e-3c2d": 4 })
        );
        assert_eq!(load(&p).get("0b8a1f8e-3c2d"), Some(&4));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_damaged_file_reads_as_empty_rather_than_failing() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join(FILE);
        std::fs::write(&p, "{ not json").unwrap();
        assert!(load(&p).is_empty());
    }
}
