//! `evnx scan --staged` (C9): what `git commit` would record, read from the
//! index, never from disk.
//!
//! A file staged and then edited on disk is committed as staged, so the index
//! blob is what is scanned. Two git processes in all: `git diff --cached --raw`
//! lists the files, and one `git cat-file --batch` reads every blob.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// One staged file.
pub struct Staged {
    /// From the repository root, `/`-separated, as git names it.
    pub path: String,
    /// `None` when the blob is larger than the cap: it was not read.
    pub bytes: Option<Vec<u8>>,
    pub size: u64,
}

/// Every added, copied, modified or retyped regular file in the index, limited
/// to `pathspecs` (relative to `cwd`) when any are given. Deleted files,
/// submodules and symlinks are not content a commit adds, so they are left out.
/// Blobs over `max_bytes` are listed unread.
pub fn staged(cwd: &Path, pathspecs: &[String], max_bytes: u64) -> Result<Vec<Staged>> {
    // `--no-renames`: a rename is then an add, whose path is the new one.
    let mut diff = Command::new("git");
    diff.args([
        "diff",
        "--cached",
        "--raw",
        "-z",
        "--no-renames",
        "--no-abbrev",
        "--diff-filter=ACMT",
        "--",
    ])
    .args(pathspecs)
    .current_dir(cwd);
    let out = diff.output().context("running git diff --cached")?;
    if !out.status.success() {
        bail!(
            "git diff --cached failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let wanted = super::git_blobs::raw_files(&out.stdout);
    let shas: Vec<String> = wanted.iter().map(|(sha, _)| sha.clone()).collect();
    let blobs = super::git_blobs::read_blobs(cwd, &shas, max_bytes)?;
    Ok(wanted
        .into_iter()
        .zip(blobs)
        .map(|((_, path), blob)| Staged {
            path,
            bytes: blob.bytes,
            size: blob.size,
        })
        .collect())
}
