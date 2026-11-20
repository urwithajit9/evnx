//! Reading git blobs for the scans that read git, not the disk: `--staged`
//! (the index) and `--history` (every commit in a range).
//!
//! One `git cat-file --batch` reads any number of blobs.

use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One blob, as read.
pub struct Blob {
    /// `None` when the blob is larger than the cap: it was not read.
    pub bytes: Option<Vec<u8>>,
    pub size: u64,
}

/// The repository's top folder, or an error saying `flag` needs one.
pub fn toplevel(dir: &Path, flag: &str) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "{flag} needs a git repository, and {} is not in one",
            dir.display()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim_end(),
    ))
}

/// Read each of `shas` (full object names) with one `git cat-file --batch`,
/// in order. A blob over `max_bytes` is skipped, not read.
pub fn read_blobs(cwd: &Path, shas: &[String], max_bytes: u64) -> Result<Vec<Blob>> {
    if shas.is_empty() {
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
    let input: String = shas.iter().map(|sha| format!("{sha}\n")).collect();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let mut reader = BufReader::new(child.stdout.take().expect("piped"));

    let mut out = Vec::with_capacity(shas.len());
    for sha in shas {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .context("reading git cat-file")?;
        // `<sha> blob <size>`, or `<sha> missing`.
        let size: u64 = match header.split_whitespace().collect::<Vec<_>>()[..] {
            [_, "blob", size] => size.parse().context("git cat-file size")?,
            _ => bail!("git cat-file could not read {sha}"),
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
        out.push(Blob { bytes, size });
    }
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("git cat-file writer panicked"))??;
    child.wait()?;
    Ok(out)
}

/// The regular files (`100644`, `100755`) in `git ... --raw -z --no-abbrev`
/// output, as `(blob id, path)`. A deletion (all-zero id), a submodule
/// (`160000`) and a symlink (`120000`) are not content a commit adds.
pub fn raw_files(raw: &[u8]) -> Vec<(String, String)> {
    // `:old_mode new_mode old_sha new_sha status\0path\0`, repeated.
    let mut out = Vec::new();
    let mut fields = raw.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(meta) = fields.next() {
        let meta = String::from_utf8_lossy(meta);
        if !meta.starts_with(':') {
            continue;
        }
        let Some(path) = fields.next() else { break };
        let parts: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(mode), Some(sha)) = (parts.get(1), parts.get(3)) else {
            continue;
        };
        if mode.starts_with("100") && sha.bytes().any(|b| b != b'0') {
            out.push((sha.to_string(), String::from_utf8_lossy(path).into_owned()));
        }
    }
    out
}
