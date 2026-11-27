//! `evnx scan --history [RANGE]` (RM-69): every blob a range of commits
//! introduced, scanned once.
//!
//! A diff is text, and a name rule needs the key beside the value, so the
//! blobs are scanned whole, as `--staged` scans an index blob. GitHub scans
//! history the same way: by blob id, not by commit. A blob that many commits
//! carry is read once and attributed to the **oldest** commit that has it.
//!
//! ⚠️ The range comes from the command line (and from a hook's stdin), so it
//! is never allowed to look like an option.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Which commits `--history` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Range {
    /// `--history` alone: everything reachable from `HEAD`.
    Head,
    /// `--history all`: every ref.
    All,
    /// Any revision range git understands: `origin/main..HEAD`, `v0.15.0..`.
    Spec(String),
}

impl Range {
    /// The value of `--history`: empty for the bare flag.
    pub fn parse(arg: &str) -> Result<Range> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Ok(Range::Head);
        }
        if arg == "all" {
            return Ok(Range::All);
        }
        // Revisions, with `--not` and `--remotes` allowed between them: what a
        // new branch needs ("everything no remote has"). Nothing else that
        // looks like an option is passed to git.
        for token in arg.split_whitespace() {
            if token.starts_with('-') && !matches!(token, "--not" | "--remotes") {
                bail!("--history: {arg:?} is not a revision range");
            }
        }
        Ok(Range::Spec(arg.to_string()))
    }

    pub fn label(&self) -> String {
        match self {
            Range::Head => "HEAD".to_string(),
            Range::All => "all refs".to_string(),
            Range::Spec(s) => s.clone(),
        }
    }
}

/// One blob to scan, with where history first had it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub blob: String,
    /// From the repository root, as git names it.
    pub path: String,
    /// The oldest commit in the range that has this blob.
    pub commit: String,
    /// That commit's time, seconds since the epoch.
    pub time: i64,
}

/// The unique blobs `range` adds or changes, oldest commit first. A deleted
/// file, a submodule and a symlink are not content, so they are left out.
/// `pathspecs` (relative to `cwd`) limit the files; `.` means no limit.
pub fn blobs(cwd: &Path, range: &Range, pathspecs: &[String]) -> Result<Vec<Entry>> {
    let mut log = Command::new("git");
    log.args([
        "log",
        "--raw",
        "-z",
        "--no-abbrev",
        "--no-renames",
        "-m",
        "--reverse",
        "--diff-filter=ACMT",
        "--format=%x01%H%x09%ct",
    ]);
    match range {
        Range::Head => log.arg("HEAD"),
        Range::All => log.arg("--all"),
        Range::Spec(s) => log.args(s.split_whitespace()),
    };
    log.arg("--")
        .args(pathspecs.iter().filter(|p| p.as_str() != "."))
        .current_dir(cwd);
    let out = log.output().context("running git log")?;
    if !out.status.success() {
        bail!(
            "git log {} failed: {}",
            range.label(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_log(&out.stdout))
}

/// `git log --raw -z --format=%x01%H%x09%ct` output, oldest first, as unique
/// blobs. Each commit is `\x01<sha>\t<time>\0`, then `\n:modes shas S\0path\0`
/// for each file.
fn parse_log(raw: &[u8]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut commit = (String::new(), 0i64);
    let mut fields = raw.split(|&b| b == 0);
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        let text = text.trim_start_matches('\n');
        if let Some(header) = text.strip_prefix('\u{1}') {
            let (sha, time) = header.split_once('\t').unwrap_or((header, "0"));
            commit = (sha.to_string(), time.trim().parse().unwrap_or(0));
        } else if text.starts_with(':') {
            let Some(path) = fields.next() else { break };
            // Reuse the one raw-line reader: it wants `meta\0path\0`.
            let mut line = text.as_bytes().to_vec();
            line.push(0);
            line.extend_from_slice(path);
            line.push(0);
            for (blob, path) in super::git_blobs::raw_files(&line) {
                if seen.insert(blob.clone()) {
                    out.push(Entry {
                        blob,
                        path,
                        commit: commit.0.clone(),
                        time: commit.1,
                    });
                }
            }
        }
    }
    out
}

/// Of `paths` (from the repository root), the blobs some commit a remote
/// already has holds there: one `git log --remotes` for all of them. `None`
/// when the repository has no remote (nothing says whether a commit was
/// pushed, so the advice stays the one for shared history) or git cannot
/// answer. A remote with no refs yet (a first push) has none.
///
/// ⚠️ The pre-push hook scans only the commits being pushed, and its advice
/// said to force push after coordinating with everyone who has a clone, for
/// commits no one else had. A blob no remote has is in local commits only:
/// amending or rebasing them is the whole fix.
pub fn on_remotes(cwd: &Path, paths: &[String]) -> Option<HashSet<String>> {
    let remotes = Command::new("git")
        .args(["remote"])
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    if String::from_utf8_lossy(&remotes.stdout).trim().is_empty() {
        return None;
    }
    let mut found = HashSet::new();
    // Chunked so a long list stays under the argument limit.
    for chunk in paths.chunks(256) {
        let out = Command::new("git")
            .args([
                "log",
                "--remotes",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-renames",
                "-m",
                "--diff-filter=ACMT",
                "--format=%x01%H%x09%ct",
                "--",
            ])
            .args(chunk.iter().map(|p| format!(":(top,literal){p}")))
            .current_dir(cwd)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        found.extend(parse_log(&out.stdout).into_iter().map(|e| e.blob));
    }
    Some(found)
}

/// What rewrites local commits from `commit` on: `git commit --amend` when it
/// is `HEAD`, else `git rebase -i <commit>^`, or `--root` for a first commit.
pub fn local_rewrite(cwd: &Path, commit: &str) -> String {
    let short: String = commit.chars().take(7).collect();
    if head(cwd).is_ok_and(|h| h == commit) {
        return "git commit --amend".to_string();
    }
    let has_parent = Command::new("git")
        .args(["rev-parse", "--verify", "-q", &format!("{commit}^")])
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if has_parent {
        format!("git rebase -i {short}^")
    } else {
        "git rebase -i --root".to_string()
    }
}

/// The commit `HEAD` names, in full.
pub fn head(cwd: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .context("running git rev-parse")?;
    if !out.status.success() {
        bail!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_head_all_or_a_spec_and_never_an_option() {
        assert_eq!(Range::parse("").unwrap(), Range::Head);
        assert_eq!(Range::parse("all").unwrap(), Range::All);
        assert_eq!(
            Range::parse("v1..").unwrap(),
            Range::Spec("v1..".to_string())
        );
        assert!(Range::parse("--output=x").is_err());
        assert!(Range::parse("HEAD --output=x").is_err());
        assert_eq!(
            Range::parse("abc123 --not --remotes").unwrap(),
            Range::Spec("abc123 --not --remotes".to_string())
        );
    }

    #[test]
    fn the_oldest_commit_keeps_a_blob_that_later_ones_carry() {
        let z = "0".repeat(40);
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let raw = format!(
            "\u{1}c1\t10\0\n:000000 100644 {z} {a} A\0x.env\0\
             \u{1}c2\t20\0\n:100644 100644 {a} {a} M\0y.env\0:100644 100644 {a} {b} M\0z.env\0\
             \u{1}c3\t30\0\n:100644 000000 {b} {z} D\0z.env\0"
        );
        let got = parse_log(raw.as_bytes());
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].commit.as_str(), got[0].time), ("c1", 10));
        assert_eq!(got[1].path, "z.env");
        assert_eq!(got[1].commit, "c2");
    }
}
