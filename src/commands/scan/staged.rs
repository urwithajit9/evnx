//! `evnx scan --staged` (C9): what `git commit` would record, read from the
//! index, never from disk.
//!
//! A file staged and then edited on disk is committed as staged, so the index
//! blob is what is scanned. Two git processes in all: `git diff --cached --raw`
//! lists the files, and one `git cat-file --batch` reads every blob.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One staged file.
pub struct Staged {
    /// From the repository root, `/`-separated, as git names it.
    pub path: String,
    /// `None` when the blob is larger than the cap: it was not read.
    pub bytes: Option<Vec<u8>>,
    pub size: u64,
}

/// The repository's top folder, or the `--staged` error outside one.
pub fn toplevel(dir: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "--staged needs a git repository, and {} is not in one",
            dir.display()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim_end(),
    ))
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
    // `:old_mode new_mode old_sha new_sha status\0path\0`, repeated.
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut fields = out.stdout.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta = String::from_utf8_lossy(meta);
        let parts: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(mode), Some(sha)) = (parts.get(1), parts.get(3)) else {
            continue;
        };
        // 100644 / 100755 only: 160000 is a submodule, 120000 a symlink.
        if mode.starts_with("100") {
            wanted.push((sha.to_string(), String::from_utf8_lossy(path).into_owned()));
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("running git cat-file")?;
    // ⚠️ Written from a thread: git answers while it reads, and a full stdout
    // pipe with an unwritten stdin is a deadlock.
    let mut stdin = child.stdin.take().expect("piped");
    let input: String = wanted.iter().map(|(sha, _)| format!("{sha}\n")).collect();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let mut reader = BufReader::new(child.stdout.take().expect("piped"));

    let mut out = Vec::with_capacity(wanted.len());
    for (sha, path) in wanted {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .context("reading git cat-file")?;
        // `<sha> blob <size>`, or `<sha> missing`.
        let size: u64 = match header.split_whitespace().collect::<Vec<_>>()[..] {
            [_, "blob", size] => size.parse().context("git cat-file size")?,
            _ => bail!("git cat-file could not read {path} ({sha})"),
        };
        let bytes = if size > max_bytes {
            std::io::copy(&mut (&mut reader).take(size), &mut std::io::sink())?;
            None
        } else {
            let mut buf = vec![0; size as usize];
            reader.read_exact(&mut buf)?;
            Some(buf)
        };
        let mut newline = [0u8; 1];
        reader.read_exact(&mut newline)?;
        out.push(Staged { path, bytes, size });
    }
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("git cat-file writer panicked"))??;
    child.wait()?;
    Ok(out)
}
