//! Gitignore management utilities for the evnx core library.
//!
//! This module provides pure I/O helpers for locating, reading, and updating
//! `.gitignore` files. It is intentionally decoupled from any specific command
//! (template, init, etc.) and contains no CLI logic, TTY detection, or user
//! prompting — those concerns belong in the calling command module.
//!
//! # Responsibilities
//!
//! - Locating the git root and its `.gitignore` by walking ancestor directories
//! - Checking whether a path entry is already present in a `.gitignore`
//! - Appending new entries under a labelled `# ── evnx generated` section
//! - Computing output-relative paths suitable for `.gitignore` patterns
//!
//! # What this module does NOT do
//!
//! - TTY detection or interactive prompting — handle in the calling command
//! - Glob pattern matching (e.g. detecting that `*.toml` covers `config.toml`)
//!   — exact-line matching only; glob support is deferred to a future version
//! - Per-subdirectory `.gitignore` file scanning — only the root `.gitignore`
//!   is considered in the current implementation
//!
//! # Usage
//!
//! ```no_run
//! use std::path::Path;
//! use evnx::core::gitignore::{
//!     find_gitignore_path, relative_output_path, check_ignored,
//!     append_entry, GitignoreStatus,
//! };
//!
//! let start = Path::new("/project/services/api");
//!
//! if let Some(gitignore_path) = find_gitignore_path(start) {
//!     let git_root = gitignore_path.parent().unwrap();
//!     let output   = Path::new("/project/services/api/config.toml");
//!
//!     if let Some(entry) = relative_output_path(git_root, output) {
//!         let entry_str = entry.to_string_lossy();
//!         match check_ignored(&gitignore_path, &entry_str).expect("could not read .gitignore") {
//!             GitignoreStatus::AlreadyIgnored => { /* nothing to do */ }
//!             _ => {
//!                 append_entry(&gitignore_path, &entry_str).expect("write failed");
//!             }
//!         }
//!     }
//! }
//! ```

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// The section header written by evnx when it adds entries to `.gitignore`.
///
/// This exact string is used both when writing the section and when detecting
/// whether the section already exists. It must not be changed without a
/// migration path, as existing `.gitignore` files will contain this literal.
pub const SECTION_HEADER: &str = "# ── evnx generated ─────────────────────────────────────────";

/// The explanatory comment written immediately below [`SECTION_HEADER`].
const SECTION_COMMENT: &str = "# Written by evnx. These files may contain secrets — do not commit.";

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// The result of checking whether a path entry is present in a `.gitignore`.
///
/// Returned by [`check_ignored`]. The caller should use this to decide whether
/// to call [`append_entry`] — `append_entry` is not idempotent on its own.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreStatus {
    /// The entry was found as an exact line match (after trimming whitespace).
    /// No action needed.
    AlreadyIgnored,

    /// The `.gitignore` file exists but does not contain a matching entry.
    /// [`append_entry`] should be called to add the entry.
    NotIgnored,

    /// The `.gitignore` file does not exist yet.
    /// [`append_entry`] will create it.
    FileNotFound,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Locate the `.gitignore` file for the git repository that contains `start`.
///
/// Walks the directory tree upward from `start`, looking for an entry named
/// `.git` at each level. Both `.git/` directories (standard repositories) and
/// `.git` files (git worktrees) are recognised. The `.gitignore` is expected
/// to reside in the same directory as the `.git` entry, even if it does not
/// exist yet — the returned path may refer to a file that has not been created.
///
/// Returns `None` if no git root is found before reaching the filesystem root.
///
/// # Arguments
///
/// * `start` - The directory from which to begin the upward walk. Typically
///   `std::env::current_dir()`. Does not need to exist; if it does not,
///   the walk begins at the nearest existing ancestor.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use evnx::core::gitignore::find_gitignore_path;
///
/// // Running from /project/services/api in a repo rooted at /project:
/// let path = find_gitignore_path(Path::new("/project/services/api"));
/// assert_eq!(path, Some(std::path::PathBuf::from("/project/.gitignore")));
/// ```
#[must_use]
pub fn find_gitignore_path(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();

    loop {
        if current.join(".git").exists() {
            return Some(current.join(".gitignore"));
        }

        // Move to the parent; stop if we have reached the filesystem root
        // (i.e. parent == current, which happens at "/" on Unix and "C:\" on Windows).
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// Compute the path of `output` relative to `git_root`, suitable for use as
/// a `.gitignore` pattern.
///
/// Both `git_root` and `output` should be **absolute** paths. If `output` is
/// relative, the caller is responsible for making it absolute (e.g. by joining
/// it with `std::env::current_dir()`) before passing it here.
///
/// Returns `None` if `output` is not under `git_root` — in that case the
/// `.gitignore` file cannot protect it, and the caller should warn the user.
///
/// # Arguments
///
/// * `git_root` - Absolute path to the root of the git repository (the
///   directory that contains `.git`).
/// * `output`   - Absolute path to the generated file to be ignored.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use evnx::core::gitignore::relative_output_path;
///
/// let git_root = Path::new("/project");
/// let output   = Path::new("/project/k8s/configmap.yaml");
/// let entry    = relative_output_path(git_root, output).unwrap();
///
/// assert_eq!(entry.to_string_lossy(), "k8s/configmap.yaml");
/// ```
///
/// ⚠️ `.` and `..` are resolved first (I-252). `cwd.join("../x.conf")` starts
/// with the root as text, so without this it came back as `../x.conf`, a
/// `.gitignore` line that matches nothing, and the caller said "Added". The
/// resolution is lexical, as git's own path matching is.
///
/// ⚠️ The result is joined with `/` on every platform: it becomes a
/// `.gitignore` line, and git reads `\` there as an escape, not a separator.
/// Rebuilding the path for the `..` step joined it with Windows' `\`
/// (`k8s\configmap.yaml`, caught by the Windows CI job).
#[must_use]
pub fn relative_output_path(git_root: &Path, output: &Path) -> Option<PathBuf> {
    let resolved = resolve_dots(output);
    let rel = resolved.strip_prefix(resolve_dots(git_root)).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(PathBuf::from(parts.join("/")))
}

/// `path` with `.` dropped and each `..` taking off the component before it.
/// A `..` with nothing left to take off stays, so the result is never inside
/// a root it climbed out of.
fn resolve_dots(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(c);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Check whether `entry` is already present in the `.gitignore` at the given path.
///
/// Matching is **exact** (after trimming leading and trailing whitespace) and
/// **case-sensitive**, consistent with git's behaviour on Linux. Comment lines
/// (starting with `#`) and blank lines are ignored.
///
/// Note: this function performs an exact string comparison only. It does not
/// evaluate gitignore glob patterns — a `.gitignore` containing `*.toml` will
/// not be detected as covering `config.toml` by this function. Glob-aware
/// coverage detection is deferred to a future version.
///
/// # Arguments
///
/// * `gitignore_path` - Path to the `.gitignore` file to check. May not exist.
/// * `entry`          - The path pattern to search for (e.g. `"config.toml"`,
///   `"k8s/configmap.yaml"`).
///
/// # Errors
///
/// Returns an error only if the file exists but cannot be read.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use evnx::core::gitignore::{check_ignored, GitignoreStatus};
///
/// let status = check_ignored(Path::new(".gitignore"), "config.toml")
///     .expect("could not read .gitignore");
///
/// match status {
///     GitignoreStatus::AlreadyIgnored => println!("already protected"),
///     GitignoreStatus::NotIgnored     => println!("needs to be added"),
///     GitignoreStatus::FileNotFound   => println!("no .gitignore yet"),
/// }
/// ```
pub fn check_ignored(gitignore_path: &Path, entry: &str) -> Result<GitignoreStatus> {
    if !gitignore_path.exists() {
        return Ok(GitignoreStatus::FileNotFound);
    }

    let content = fs::read_to_string(gitignore_path).with_context(|| {
        format!(
            "Failed to read .gitignore at '{}'",
            gitignore_path.display()
        )
    })?;

    let matched = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .any(|line| line.trim() == entry.trim());

    if matched {
        Ok(GitignoreStatus::AlreadyIgnored)
    } else {
        Ok(GitignoreStatus::NotIgnored)
    }
}

/// Add `entry` to the `.gitignore` at the given path, creating it if necessary.
///
/// Entries are grouped under a labelled `# ── evnx generated` section so they
/// are easy to identify and maintain. On the first call, the section header and
/// an explanatory comment are written; subsequent calls append inside the
/// existing section.
///
/// **This function is not idempotent.** Always call [`check_ignored`] first and
/// only call `append_entry` when the status is [`GitignoreStatus::NotIgnored`]
/// or [`GitignoreStatus::FileNotFound`].
///
/// # Arguments
///
/// * `gitignore_path` - Path to the `.gitignore` file. Created if it does not
///   exist; its parent directory must already exist.
/// * `entry`          - The path pattern to append (e.g. `"config.toml"`).
///
/// # Errors
///
/// Returns an error if the file cannot be read or written.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use evnx::core::gitignore::{check_ignored, append_entry, GitignoreStatus};
///
/// let path  = Path::new(".gitignore");
/// let entry = "config.toml";
///
/// if !matches!(check_ignored(path, entry).unwrap(), GitignoreStatus::AlreadyIgnored) {
///     append_entry(path, entry).expect("failed to update .gitignore");
/// }
/// ```
pub fn append_entry(gitignore_path: &Path, entry: &str) -> Result<()> {
    let content = if gitignore_path.exists() {
        fs::read_to_string(gitignore_path).with_context(|| {
            format!(
                "Failed to read .gitignore at '{}'",
                gitignore_path.display()
            )
        })?
    } else {
        String::new()
    };

    // ⚠️ A `.gitignore` with CRLF endings keeps them: the entry is added in the
    // file's own line endings, not as a lone LF line among CRLF ones (A-28).
    let crlf = content.contains("\r\n");
    let content = if crlf {
        content.replace("\r\n", "\n")
    } else {
        content
    };
    let mut updated = if content.contains(SECTION_HEADER) {
        insert_into_section(&content, entry)
    } else {
        append_new_section(&content, entry)
    };
    if crlf {
        updated = updated.replace('\n', "\r\n");
    }

    // Shared, so a new `.gitignore` is 0644; atomic, and a symlinked
    // `.gitignore` is written through (core::safe_write).
    let written = crate::core::safe_write::write_shared(gitignore_path, updated.as_bytes())
        .with_context(|| {
            format!(
                "Failed to write .gitignore at '{}'",
                gitignore_path.display()
            )
        })?;
    crate::core::safe_write::report(&written);

    Ok(())
}

/// Add each of `entries` that `.gitignore` does not already hold as a line,
/// under the evnx section. Returns the entries that were added, in order.
///
/// The one `.gitignore` writer (A-28): `init`, `add` and `doctor --fix` all come
/// through here. "Holds as a line" is deliberately literal — whether the result
/// actually protects a file is [`is_protected`]'s question, asked afterwards.
pub fn append_entries(gitignore_path: &Path, entries: &[&str]) -> Result<Vec<String>> {
    let mut added = Vec::new();
    for entry in entries {
        match check_ignored(gitignore_path, entry)? {
            GitignoreStatus::AlreadyIgnored => {}
            GitignoreStatus::NotIgnored | GitignoreStatus::FileNotFound => {
                append_entry(gitignore_path, entry)?;
                added.push((*entry).to_string());
            }
        }
    }
    Ok(added)
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Insert `entry` inside the existing evnx section in `content`.
///
/// The section ends at the first blank line after the header, or at EOF.
/// The new entry is inserted at that boundary so it stays within the section.
fn insert_into_section(content: &str, entry: &str) -> String {
    let trailing_newline = content.ends_with('\n');
    let lines: Vec<&str> = content.lines().collect();

    // SAFETY: called only after confirming SECTION_HEADER is present in content.
    let header_idx = lines
        .iter()
        .position(|l| *l == SECTION_HEADER)
        .expect("SECTION_HEADER confirmed present before calling insert_into_section");

    // The section ends at the first blank line after the header, or at EOF.
    let section_end = lines[header_idx + 1..]
        .iter()
        .position(|l| l.trim().is_empty())
        .map(|offset| header_idx + 1 + offset)
        .unwrap_or(lines.len());

    let mut updated = lines.to_vec();
    updated.insert(section_end, entry);

    let mut result = updated.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result
}

/// Append a new evnx section containing `entry` to the end of `content`.
///
/// Ensures exactly one blank line separates the new section from any existing
/// content, and that the file ends with a trailing newline.
fn append_new_section(content: &str, entry: &str) -> String {
    // Determine the separator needed before the new section.
    let prefix = if content.is_empty() {
        // New file — no separator needed.
        ""
    } else if content.ends_with("\n\n") {
        // Already has a blank line — no additional separator.
        ""
    } else if content.ends_with('\n') {
        // Has a trailing newline but no blank line — add one blank line.
        "\n"
    } else {
        // No trailing newline at all — add newline + blank line.
        "\n\n"
    };

    format!("{content}{prefix}{SECTION_HEADER}\n{SECTION_COMMENT}\n{entry}\n")
}

// ─────────────────────────────────────────────────────────────
// Is a file ignored?
// ─────────────────────────────────────────────────────────────

/// The `.gitignore` lines evnx writes to protect env files.
///
/// ⚠️ One definition. This list lived in `init::shared` and again, inline, in
/// `doctor`'s auto-fix, and `add` had neither — which is how `evnx add` came to
/// append to a `.env` in a repository that would commit it, without a word.
///
/// The negations must follow `.env*`, because gitignore resolves last-match-wins
/// and a negation placed above the pattern it carves out of does nothing.
pub const ENV_ENTRIES: &[&str] = &[".env*", "!.env.example", "!.env.sample", "!.env.template"];

/// Whether git would ignore `filename` under `project_root`.
///
/// ⚠️ Not [`check_ignored`], which compares `.gitignore` lines for exact string
/// equality. That is right for "did *we* write this entry" and wrong for "is
/// this file protected": against the `.env*` that `evnx init` writes, asking it
/// about `.env` returns `NotIgnored`, so a caller would warn about every project
/// evnx itself had set up correctly.
///
/// Asks git, which is the authority, and falls back to reading `.gitignore` when
/// git cannot answer — no repository, or git not installed.
pub fn env_file_is_protected(project_root: &Path, filename: &str) -> bool {
    is_protected(project_root, filename)
}

/// Whether git would leave `filename` out of a commit: ignored by the rules
/// that apply to it. `git check-ignore` is the authority (every `.gitignore`
/// level, negation, `.git/info/exclude`, global excludes); the fallback reads
/// the project's `.gitignore` when git cannot answer (A-24, DEC-T11).
///
/// ⚠️ A file that is already **tracked** is not protected whatever the rules
/// say, and `git check-ignore` reports it as not ignored. Use [`exposure`] to
/// tell the two apart.
pub fn is_protected(project_root: &Path, filename: &str) -> bool {
    is_gitignored(project_root, filename)
        .unwrap_or_else(|_| fallback_gitignore_check(project_root, filename))
}

/// How exposed `filename` is to git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Ignored: a commit will not include it.
    Protected,
    /// Not ignored and not tracked: the next `git add .` takes it.
    Unignored,
    /// Already in the index: its contents are in the repository, and
    /// `.gitignore` no longer applies to it (I-157). Only `git rm --cached`
    /// and rotating the secrets undo that.
    Tracked,
}

/// [`Exposure`] of `filename` under `project_root`. Without git, or outside a
/// repository, nothing can be tracked and the answer comes from the rules.
pub fn exposure(project_root: &Path, filename: &str) -> Exposure {
    if is_tracked(project_root, filename) {
        Exposure::Tracked
    } else if is_protected(project_root, filename) {
        Exposure::Protected
    } else {
        Exposure::Unignored
    }
}

/// What git says about one file a command is about to read: its [`Exposure`],
/// and its path from the repository root with `/` separators (what
/// `git filter-repo --path` takes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitView {
    pub exposure: Exposure,
    pub repo_path: String,
}

/// [`exposure`] for many files at once, in the order given (I-285).
///
/// `None` for a file outside a work tree, or wherever git cannot answer — no
/// git, a path that cannot be resolved, a command that fails. A caller treats
/// `None` as "no information", never as "ignored".
///
/// ⚠️ Two git processes per **repository**, not two per file. [`exposure`]
/// runs `ls-files` and `check-ignore` for its one file, which is right for
/// `validate` and `doctor` and would be thousands of processes for a walk.
/// Here every file is grouped under the nearest folder holding a `.git` (a
/// directory, or a file for a submodule or worktree), and that repository is
/// asked once: `ls-files -z` for what is tracked, `check-ignore -z --stdin` for
/// what is ignored. `check-ignore` does not report a tracked file, which is the
/// same order [`exposure`] decides in.
pub fn exposures(files: &[PathBuf]) -> Vec<Option<GitView>> {
    use std::collections::HashMap;

    let mut out: Vec<Option<GitView>> = vec![None; files.len()];
    // Repository root → (index into `files`, path from that root).
    let mut groups: HashMap<PathBuf, Vec<(usize, String)>> = HashMap::new();
    let mut root_of: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();

    for (i, file) in files.iter().enumerate() {
        let Ok(abs) = fs::canonicalize(file) else {
            continue;
        };
        let Some(dir) = abs.parent() else { continue };
        let root = root_of
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                dir.ancestors()
                    .find(|a| a.join(".git").exists())
                    .map(Path::to_path_buf)
            })
            .clone();
        let Some(root) = root else { continue };
        let Ok(rel) = abs.strip_prefix(&root) else {
            continue;
        };
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        groups.entry(root).or_default().push((i, rel));
    }

    for (root, members) in groups {
        let Some((tracked, ignored)) = ask_repository(&root, &members) else {
            continue;
        };
        for (i, rel) in members {
            let exposure = if tracked.contains(&rel) {
                Exposure::Tracked
            } else if ignored.contains(&rel) {
                Exposure::Protected
            } else {
                Exposure::Unignored
            };
            out[i] = Some(GitView {
                exposure,
                repo_path: rel,
            });
        }
    }
    out
}

/// The tracked and the ignored paths among `members`, from git itself, or
/// `None` when git cannot answer for `root`.
fn ask_repository(
    root: &Path,
    members: &[(usize, String)],
) -> Option<(
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
)> {
    use std::io::Write;
    use std::process::Stdio;

    // ⚠️ The folder must be the work tree's top, not merely contain a `.git`:
    // a stray file named `.git` would otherwise make git answer for whatever
    // repository sits above it, with paths relative to the wrong root.
    let top = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    if fs::canonicalize(&top).ok()? != root {
        return None;
    }

    let split = |bytes: &[u8]| -> std::collections::HashSet<String> {
        bytes
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect()
    };

    let listed = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let tracked = split(&listed.stdout);

    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-z", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        for (_, rel) in members {
            stdin.write_all(rel.as_bytes()).ok()?;
            stdin.write_all(&[0]).ok()?;
        }
    }
    let answered = child.wait_with_output().ok()?;
    // 0: some paths are ignored, 1: none are. Anything else is no answer — the
    // same reading as `is_gitignored`, where collapsing 128 onto "not ignored"
    // once reported every env file in a plain folder as exposed.
    if !matches!(answered.status.code(), Some(0 | 1)) {
        return None;
    }
    Some((tracked, split(&answered.stdout)))
}

/// Whether `dir` is inside a git work tree. `false` when git cannot answer.
pub fn in_work_tree(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Whether git's index holds `filename`. `false` when git cannot answer.
pub fn is_tracked(project_root: &Path, filename: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["ls-files", "--error-unmatch", "--", filename])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ask git whether `filename` is ignored.
///
/// ⚠️ `Err` means **git could not answer**, not "not ignored" — the caller falls
/// back to reading `.gitignore` directly, and collapsing the two makes that
/// fallback unreachable.
///
/// `git check-ignore` exits `0` when the path is ignored, `1` when it is not, and
/// `128` when there is no repository. This used to return
/// `Ok(status.success())`, so `128` became `Ok(false)` — "not ignored" — and any
/// project that was not a git repo had every one of its env files reported as a
/// security risk. A check that cries wolf on a directory with nothing wrong with
/// it is one people learn to skip.
pub fn is_gitignored(project_root: &Path, filename: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "-C",
            project_root.to_string_lossy().as_ref(),
            "check-ignore",
            filename,
        ])
        .output()
        .context("Failed to execute git check-ignore")?;

    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        other => anyhow::bail!(
            "git check-ignore could not answer for {filename} (exit {})",
            other.map_or_else(|| "signal".to_string(), |c| c.to_string())
        ),
    }
}

/// Whether `.gitignore` covers `filename`, without shelling out to git.
///
/// Used when `git check-ignore` cannot answer — no repository, or git is not
/// installed. It is a deliberate approximation, but it has to understand the
/// patterns evnx itself writes, and evnx now writes `.env*` with the template
/// family negated rather than a literal `.env`. Exact line matching alone would
/// report a correctly-ignored `.env` as exposed in any non-git directory.
///
/// Handles, in gitignore's own precedence order (last match wins):
/// exact names, a leading `/`, a trailing `*` glob, and `!` negations.
/// Anything more (`**`, character classes, mid-pattern globs) is left to git.
pub fn fallback_gitignore_check(project_root: &Path, filename: &str) -> bool {
    let gitignore_path = project_root.join(".gitignore");
    if !gitignore_path.exists() {
        return false;
    }

    let Ok(content) = fs::read_to_string(&gitignore_path) else {
        return false;
    };

    let mut ignored = false;
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (negated, pattern) = match line.strip_prefix('!') {
            Some(rest) => (true, rest.trim()),
            None => (false, line),
        };
        let pattern = pattern.strip_prefix('/').unwrap_or(pattern);

        let matches = match pattern.strip_suffix('*') {
            Some(prefix) => filename.starts_with(prefix),
            None => filename == pattern,
        };

        // Later rules override earlier ones, which is what makes
        // `.env*` followed by `!.env.example` work.
        if matches {
            ignored = !negated;
        }
    }
    ignored
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {

    // ── exposure / append_entries (U2 part 3) ─────────────────────────────────

    fn git_repo() -> Option<tempfile::TempDir> {
        let d = tempfile::TempDir::new().unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then_some(d)
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// I-156: `.env*` then `!.env` re-includes `.env`. Every line evnx writes
    /// is present, so a line-by-line check says protected; git says not.
    #[test]
    fn t_i156_a_later_negation_leaves_env_unignored() {
        let Some(d) = git_repo() else { return };
        fs::write(d.path().join(".gitignore"), ".env*\n!.env\n").unwrap();
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        assert_eq!(exposure(d.path(), ".env"), Exposure::Unignored);
        assert!(!is_protected(d.path(), ".env"));
    }

    /// I-157: a committed `.env` is tracked, whatever `.gitignore` says.
    #[test]
    fn t_i157_a_committed_env_is_tracked_not_unignored() {
        let Some(d) = git_repo() else { return };
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        git(d.path(), &["add", ".env"]);
        git(d.path(), &["commit", "-qm", "x"]);
        fs::write(d.path().join(".gitignore"), ".env\n").unwrap();
        assert_eq!(exposure(d.path(), ".env"), Exposure::Tracked);
    }

    #[test]
    fn exposure_protected_and_without_a_repository() {
        let Some(d) = git_repo() else { return };
        fs::write(d.path().join(".gitignore"), ".env*\n!.env.example\n").unwrap();
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        assert_eq!(exposure(d.path(), ".env"), Exposure::Protected);
        assert_eq!(exposure(d.path(), ".env.example"), Exposure::Unignored);

        let plain = tempfile::TempDir::new().unwrap();
        fs::write(plain.path().join(".gitignore"), ".env*\n").unwrap();
        assert_eq!(exposure(plain.path(), ".env"), Exposure::Protected);
    }

    // ── exposures: the batched form, for a walk (I-285) ──────────────────────

    fn exposure_of(views: &[Option<GitView>], i: usize) -> Option<Exposure> {
        views[i].as_ref().map(|v| v.exposure)
    }

    /// One answer per file, each the same as [`exposure`] gives for it alone.
    #[test]
    fn t_i285_exposures_agree_with_exposure_for_each_file() {
        let Some(d) = git_repo() else { return };
        let root = d.path();
        fs::write(root.join(".gitignore"), ".env*\n!.env.example\nsecrets/\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("secrets")).unwrap();
        for f in [
            ".env",
            ".env.example",
            "src/app.py",
            "secrets/a.txt",
            "done.txt",
        ] {
            fs::write(root.join(f), "A=1\n").unwrap();
        }
        git(root, &["add", "done.txt"]);
        git(root, &["commit", "-qm", "x"]);

        let files: Vec<PathBuf> = [
            ".env",
            ".env.example",
            "src/app.py",
            "secrets/a.txt",
            "done.txt",
        ]
        .iter()
        .map(|f| root.join(f))
        .collect();
        let views = exposures(&files);
        assert_eq!(exposure_of(&views, 0), Some(Exposure::Protected));
        assert_eq!(exposure_of(&views, 1), Some(Exposure::Unignored));
        assert_eq!(exposure_of(&views, 2), Some(Exposure::Unignored));
        // A file inside an ignored directory is ignored too.
        assert_eq!(exposure_of(&views, 3), Some(Exposure::Protected));
        assert_eq!(exposure_of(&views, 4), Some(Exposure::Tracked));
        assert_eq!(views[2].as_ref().unwrap().repo_path, "src/app.py");
    }

    /// I-157 through the batch: committed first and ignored later is tracked.
    #[test]
    fn t_i285_exposures_a_committed_then_ignored_file_is_tracked() {
        let Some(d) = git_repo() else { return };
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        git(d.path(), &["add", ".env"]);
        git(d.path(), &["commit", "-qm", "x"]);
        fs::write(d.path().join(".gitignore"), ".env\n").unwrap();
        let views = exposures(&[d.path().join(".env")]);
        assert_eq!(exposure_of(&views, 0), Some(Exposure::Tracked));
    }

    /// Outside a work tree there is no answer — not "ignored", whatever a
    /// `.gitignore` lying in the folder says.
    #[test]
    fn t_i285_exposures_outside_a_repository_is_no_answer() {
        let plain = tempfile::TempDir::new().unwrap();
        fs::write(plain.path().join(".gitignore"), ".env*\n").unwrap();
        fs::write(plain.path().join(".env"), "A=1\n").unwrap();
        assert_eq!(exposures(&[plain.path().join(".env")]), vec![None]);
        // A path that does not resolve is no answer too.
        assert_eq!(exposures(&[plain.path().join("missing")]), vec![None]);
    }

    /// A path spelled with `.` and `..`, as a walk or an argument gives it, gets
    /// the same answer and the same repository path.
    #[test]
    fn t_i285_exposures_accepts_dotted_spellings() {
        let Some(d) = git_repo() else { return };
        fs::create_dir_all(d.path().join("sub")).unwrap();
        fs::write(d.path().join(".gitignore"), ".env\n").unwrap();
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        let views = exposures(&[d.path().join("./.env"), d.path().join("sub/../.env")]);
        for i in 0..2 {
            assert_eq!(exposure_of(&views, i), Some(Exposure::Protected));
            assert_eq!(views[i].as_ref().unwrap().repo_path, ".env");
        }
    }

    /// Two repositories in one call — a nested one is its own repository.
    #[test]
    fn t_i285_exposures_a_nested_repository_answers_for_itself() {
        let Some(d) = git_repo() else { return };
        let inner = d.path().join("inner");
        fs::create_dir_all(&inner).unwrap();
        git(&inner, &["init", "-q"]);
        // The outer repository ignores nothing; the inner one ignores `.env`.
        fs::write(inner.join(".gitignore"), ".env\n").unwrap();
        fs::write(inner.join(".env"), "A=1\n").unwrap();
        fs::write(d.path().join(".env"), "A=1\n").unwrap();
        let views = exposures(&[d.path().join(".env"), inner.join(".env")]);
        assert_eq!(exposure_of(&views, 0), Some(Exposure::Unignored));
        assert_eq!(exposure_of(&views, 1), Some(Exposure::Protected));
        assert_eq!(views[1].as_ref().unwrap().repo_path, ".env");
    }

    #[test]
    fn append_entries_adds_only_what_is_missing() {
        let d = tempfile::TempDir::new().unwrap();
        let g = d.path().join(".gitignore");
        fs::write(&g, "node_modules/\n.env*\n").unwrap();
        let added = append_entries(&g, &[".env*", "!.env.example"]).unwrap();
        assert_eq!(added, vec!["!.env.example".to_string()]);
        assert!(append_entries(&g, &[".env*", "!.env.example"])
            .unwrap()
            .is_empty());
    }

    /// A-28: a CRLF `.gitignore` stays CRLF.
    #[test]
    fn a_crlf_gitignore_keeps_its_line_endings() {
        let d = tempfile::TempDir::new().unwrap();
        let g = d.path().join(".gitignore");
        fs::write(&g, "node_modules/\r\ndist/\r\n").unwrap();
        append_entry(&g, ".env*").unwrap();
        let out = fs::read_to_string(&g).unwrap();
        assert!(out.contains(".env*\r\n"), "{out:?}");
        assert!(
            !out.replace("\r\n", "").contains('\n'),
            "a lone LF: {out:?}"
        );
    }

    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// The negations carve out exactly the template family `core::env_name`
    /// names, so the two lists cannot drift apart.
    #[test]
    fn env_entries_negate_the_template_family() {
        let negated: Vec<&str> = ENV_ENTRIES
            .iter()
            .filter_map(|e| e.strip_prefix("!.env."))
            .collect();
        assert_eq!(negated, crate::core::env_name::TEMPLATE_SUFFIXES);
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Create a temporary directory tree:
    ///   <root>/
    ///   <root>/.git/          (marks the git root)
    ///   <root>/sub/sub2/      (nested working directory)
    fn make_repo(tmp: &TempDir) -> (PathBuf, PathBuf) {
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("sub").join("sub2");
        fs::create_dir_all(&nested).unwrap();
        (root, nested)
    }

    // ── find_gitignore_path ───────────────────────────────────────────────────

    #[test]
    fn test_find_from_repo_root() {
        let tmp = TempDir::new().unwrap();
        let (root, _) = make_repo(&tmp);

        let result = find_gitignore_path(&root);
        assert_eq!(result, Some(root.join(".gitignore")));
    }

    #[test]
    fn test_find_from_nested_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let (root, nested) = make_repo(&tmp);

        let result = find_gitignore_path(&nested);
        assert_eq!(result, Some(root.join(".gitignore")));
    }

    #[test]
    fn test_find_no_git_repo() {
        // A temp dir with no .git anywhere.
        let tmp = TempDir::new().unwrap();
        let result = find_gitignore_path(tmp.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_git_as_file_worktree() {
        // git worktrees use a .git FILE instead of a directory.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        fs::write(root.join(".git"), "gitdir: /some/other/.git/worktrees/wt").unwrap();

        let result = find_gitignore_path(&root);
        assert_eq!(result, Some(root.join(".gitignore")));
    }

    #[test]
    fn test_find_gitignore_not_required_to_exist() {
        // find_gitignore_path returns the expected path even if the file
        // itself does not exist yet (it will be created by append_entry).
        let tmp = TempDir::new().unwrap();
        let (root, nested) = make_repo(&tmp);

        let result = find_gitignore_path(&nested).unwrap();
        assert_eq!(result, root.join(".gitignore"));
        assert!(!result.exists(), "file should not exist yet");
    }

    // ── relative_output_path ─────────────────────────────────────────────────

    #[test]
    fn test_relative_path_simple() {
        let root = Path::new("/project");
        let output = Path::new("/project/config.toml");
        let result = relative_output_path(root, output).unwrap();
        assert_eq!(result, PathBuf::from("config.toml"));
    }

    #[test]
    fn test_relative_path_subdirectory() {
        let root = Path::new("/project");
        let output = Path::new("/project/k8s/configmap.yaml");
        let result = relative_output_path(root, output).unwrap();
        assert_eq!(result, PathBuf::from("k8s/configmap.yaml"));
    }

    #[test]
    fn test_relative_path_outside_root_returns_none() {
        let root = Path::new("/project");
        let output = Path::new("/other/config.toml");
        let result = relative_output_path(root, output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_relative_path_deeply_nested() {
        let root = Path::new("/project");
        let output = Path::new("/project/a/b/c/config.toml");
        let result = relative_output_path(root, output).unwrap();
        assert_eq!(result, PathBuf::from("a/b/c/config.toml"));
    }

    /// I-252: `..` and `.` are resolved before the root is stripped, so
    /// `sub/../out.yml` is `out.yml`, and `../x.conf` is outside.
    #[test]
    fn t_i252_dot_dot_is_resolved_before_the_root_is_stripped() {
        let root = Path::new("/project");
        assert_eq!(
            relative_output_path(root, Path::new("/project/sub/../out.yml")),
            Some(PathBuf::from("out.yml"))
        );
        assert_eq!(
            relative_output_path(root, Path::new("/project/./a/./b.yml")),
            Some(PathBuf::from("a/b.yml"))
        );
        assert_eq!(
            relative_output_path(root, Path::new("/project/../x.conf")),
            None
        );
        assert_eq!(
            relative_output_path(root, Path::new("/project/a/../../x")),
            None
        );
    }

    /// The entry is written with `/`, whatever the platform joins paths with.
    #[test]
    fn t_i252_an_entry_is_joined_with_forward_slashes() {
        let entry = relative_output_path(
            Path::new("/project"),
            Path::new("/project/k8s/../k8s/./configmap.yaml"),
        )
        .unwrap();
        assert_eq!(entry.to_string_lossy(), "k8s/configmap.yaml");
    }

    /// On Windows a path typed with `\` still gives a `/` entry.
    #[cfg(windows)]
    #[test]
    fn t_i252_a_windows_path_gives_a_forward_slash_entry() {
        let entry = relative_output_path(
            Path::new(r"C:\project"),
            Path::new(r"C:\project\k8s\sub\..\configmap.yaml"),
        )
        .unwrap();
        assert_eq!(entry.to_string_lossy(), "k8s/configmap.yaml");
        assert_eq!(
            relative_output_path(Path::new(r"C:\project"), Path::new(r"C:\project\..\x.conf")),
            None
        );
    }

    // ── check_ignored ─────────────────────────────────────────────────────────

    #[test]
    fn test_check_file_not_found() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        // File does not exist
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::FileNotFound);
    }

    #[test]
    fn test_check_empty_file_is_not_ignored() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, "").unwrap();
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::NotIgnored);
    }

    #[test]
    fn test_check_exact_match() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\nconfig.toml\nnode_modules/\n").unwrap();
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::AlreadyIgnored);
    }

    #[test]
    fn test_check_no_match() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\nnode_modules/\n").unwrap();
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::NotIgnored);
    }

    #[test]
    fn test_check_comment_line_is_not_a_match() {
        // A line like "# config.toml" must not be treated as a match.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, "# config.toml\n").unwrap();
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::NotIgnored);
    }

    #[test]
    fn test_check_whitespace_padded_entry_matches() {
        // "  config.toml  " in the file should match "config.toml".
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, "  config.toml  \n").unwrap();
        let status = check_ignored(&path, "config.toml").unwrap();
        assert_eq!(status, GitignoreStatus::AlreadyIgnored);
    }

    #[test]
    fn test_check_blank_lines_not_false_positive() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, "\n\n\n").unwrap();
        let status = check_ignored(&path, "").unwrap();
        // Even an empty entry should not match blank lines
        // (blank lines are filtered out before comparison)
        assert_eq!(status, GitignoreStatus::NotIgnored);
    }

    #[test]
    fn test_check_subdirectory_entry() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, "k8s/configmap.yaml\n").unwrap();
        let status = check_ignored(&path, "k8s/configmap.yaml").unwrap();
        assert_eq!(status, GitignoreStatus::AlreadyIgnored);
    }

    // ── append_entry ─────────────────────────────────────────────────────────

    #[test]
    fn test_append_creates_file_when_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        assert!(!path.exists());

        append_entry(&path, "config.toml").unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(SECTION_HEADER));
        assert!(content.contains(SECTION_COMMENT));
        assert!(content.contains("config.toml"));
        assert!(content.ends_with('\n'), "file should end with newline");
    }

    #[test]
    fn test_append_to_existing_file_without_section() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\nnode_modules/\n").unwrap();

        append_entry(&path, "config.toml").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // Original content preserved
        assert!(content.contains(".env"));
        assert!(content.contains("node_modules/"));
        // New section appended
        assert!(content.contains(SECTION_HEADER));
        assert!(content.contains("config.toml"));
        // Section appears after original content
        let header_pos = content.find(SECTION_HEADER).unwrap();
        let env_pos = content.find(".env").unwrap();
        assert!(header_pos > env_pos);
    }

    #[test]
    fn test_append_separator_when_file_ends_with_newline() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\n").unwrap();

        append_entry(&path, "config.toml").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // There should be a blank line between existing content and the section.
        assert!(
            content.contains(".env\n\n"),
            "expected blank line before section; got:\n{content}"
        );
    }

    #[test]
    fn test_append_no_double_blank_when_file_ends_with_double_newline() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\n\n").unwrap();

        append_entry(&path, "config.toml").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // Should not introduce a triple newline.
        assert!(
            !content.contains("\n\n\n"),
            "unexpected triple newline; got:\n{content}"
        );
    }

    #[test]
    fn test_append_second_entry_inside_section() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");

        append_entry(&path, "config.toml").unwrap();
        append_entry(&path, "k8s/configmap.yaml").unwrap();

        let content = fs::read_to_string(&path).unwrap();

        // Both entries present
        assert!(content.contains("config.toml"));
        assert!(content.contains("k8s/configmap.yaml"));

        // Only one section header
        assert_eq!(
            content.matches(SECTION_HEADER).count(),
            1,
            "section header should appear exactly once"
        );

        // Second entry is inside the section (appears after header, not before)
        let header_pos = content.find(SECTION_HEADER).unwrap();
        let second_pos = content.find("k8s/configmap.yaml").unwrap();
        assert!(second_pos > header_pos);
    }

    #[test]
    fn test_append_section_mid_file_inserts_before_blank_line() {
        // Existing structure:
        //   .env
        //   # ── evnx generated ──
        //   config.toml
        //
        //   node_modules/
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        let initial =
            format!(".env\n{SECTION_HEADER}\n{SECTION_COMMENT}\nconfig.toml\n\nnode_modules/\n");
        fs::write(&path, &initial).unwrap();

        append_entry(&path, "deploy.yaml").unwrap();

        let content = fs::read_to_string(&path).unwrap();

        // node_modules/ must still be present and after a blank line
        assert!(content.contains("\n\nnode_modules/"));
        // deploy.yaml must be inside the section, before node_modules/
        let deploy_pos = content.find("deploy.yaml").unwrap();
        let modules_pos = content.find("node_modules/").unwrap();
        assert!(
            deploy_pos < modules_pos,
            "deploy.yaml should precede node_modules/"
        );
        // Only one section header
        assert_eq!(content.matches(SECTION_HEADER).count(), 1);
    }

    #[test]
    fn test_append_preserves_trailing_newline() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");
        fs::write(&path, ".env\n").unwrap();

        append_entry(&path, "config.toml").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'), "file should end with newline");
    }

    #[test]
    fn test_append_empty_file_no_leading_blank() {
        // Creating a .gitignore from scratch should not start with a blank line.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gitignore");

        append_entry(&path, "config.toml").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(
            !content.starts_with('\n'),
            "file created from empty should not start with blank line"
        );
    }

    // ── integration: full workflow ────────────────────────────────────────────

    #[test]
    fn test_full_workflow_new_repo() {
        let tmp = TempDir::new().unwrap();
        let (root, nested) = make_repo(&tmp);
        let output = root.join("config.toml");

        // Simulate a generated output file
        fs::write(&output, "secret = true\n").unwrap();

        // Step 1: find the gitignore path
        let gitignore = find_gitignore_path(&nested).unwrap();
        assert_eq!(gitignore, root.join(".gitignore"));

        // Step 2: compute the relative entry
        let entry = relative_output_path(&root, &output).unwrap();
        let entry_str = entry.to_string_lossy().to_string();
        assert_eq!(entry_str, "config.toml");

        // Step 3: check current status
        let status = check_ignored(&gitignore, &entry_str).unwrap();
        assert_eq!(status, GitignoreStatus::FileNotFound);

        // Step 4: append the entry
        append_entry(&gitignore, &entry_str).unwrap();

        // Step 5: verify it is now detected as ignored
        let status = check_ignored(&gitignore, &entry_str).unwrap();
        assert_eq!(status, GitignoreStatus::AlreadyIgnored);
    }

    #[test]
    fn test_full_workflow_idempotent_via_check() {
        let tmp = TempDir::new().unwrap();
        let (root, _) = make_repo(&tmp);
        let gitignore = root.join(".gitignore");

        // First run
        append_entry(&gitignore, "config.toml").unwrap();
        // Second run — check first, then conditionally append
        if !matches!(
            check_ignored(&gitignore, "config.toml").unwrap(),
            GitignoreStatus::AlreadyIgnored
        ) {
            append_entry(&gitignore, "config.toml").unwrap();
        }

        let content = fs::read_to_string(&gitignore).unwrap();
        assert_eq!(
            content.matches("config.toml").count(),
            1,
            "entry should appear exactly once when check is used correctly"
        );
    }
}
