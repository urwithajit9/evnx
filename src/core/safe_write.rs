//! Writing a file evnx produces: one way, for every command (C-04, U2).
//!
//! Before this module, seven commands wrote files seven ways: `fs::write` at the
//! umask's mode (convert `-o`, template, init — secrets left 0644, I-081, I-130,
//! I-237), a truncate-in-place that refused symlinks (backup, restore — a crash
//! left a half-written `.env`, I-151), and a temp-file rename that replaced a
//! symlink with a plain file and set modes by accident (sync, I-122, I-123).
//!
//! # The contract
//!
//! | Rule | Why |
//! |---|---|
//! | **Atomic**: written to a temp file in the same directory, flushed, renamed over the target | a crash or full disk leaves the old file or the new one, never half of either |
//! | **New file**: [`write_private`] 0600, [`write_shared`] 0644 | secrets are not world-readable; templates stay shareable (G9) |
//! | **Existing file keeps its mode** (DEC-G6, DEC-S7) | the user chose it; a private file that is readable by others gets a warning naming `chmod`, not a silent change |
//! | **A symlink is written through** to its target, and the link stays (DEC-S6, DEC-B7) | monorepos and stow/Nix setups link `.env.example`; replacing the link broke them |
//! | the temp file is created exclusively (`O_EXCL`), 0600 from the start | nobody can pre-create it or read it while it fills |
//!
//! ⚠️ Not kept across the rename: the old file's owner, ACLs, extended attributes
//! and other hard links (a hard link keeps the old content). Same as editors that
//! save atomically.
//!
//! ⚠️ Windows: modes do not apply (see [`super::permissions`]); the file is still
//! written atomically, and a private one gets an owner-only ACL (a warning if
//! `icacls` fails).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::permissions;

/// What a write did, for the caller's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The path the caller asked for.
    pub path: PathBuf,
    /// The file that was created or replaced: `path`, or a symlink's target.
    pub target: PathBuf,
    /// `path` was a symlink and the write went to its target.
    pub through_link: bool,
    /// A file already existed at `target` and was replaced.
    pub replaced: bool,
    /// The file holds secrets, already existed, and kept a mode that others can
    /// read (DEC-G6). `None` when there is nothing to warn about.
    pub loose_mode: Option<u32>,
}

/// Write a file that holds secrets: new files 0600.
pub fn write_private(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Written> {
    write(
        path.as_ref(),
        bytes,
        Mode::NewOnly(permissions::PRIVATE),
        true,
    )
}

/// Write a file meant to be shared (`.env.example`): new files 0644.
pub fn write_shared(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Written> {
    write(
        path.as_ref(),
        bytes,
        Mode::NewOnly(permissions::SHARED),
        false,
    )
}

/// Write with a mode the user asked for (`template --mode`). Unlike the two
/// above, this sets the mode on an existing file too: it was requested.
pub fn write_with_mode(path: impl AsRef<Path>, bytes: &[u8], mode: u32) -> Result<Written> {
    write(path.as_ref(), bytes, Mode::Always(mode), false)
}

/// Keep `content` — what `path` holds before an in-place edit — beside it as
/// `<path>.bak`. Returns the backup's path.
///
/// ⚠️ **One backup per file.** `<path>.bak` is replaced on every call; it holds
/// the file as it was before the latest edit. Earlier versions of this helper
/// piled up `.bak.2` … `.bak.100`, one per write, and a `.gitignore` question
/// for each. Stale numbered copies of the same file (`<path>.bak.<digits>`,
/// exactly that and nothing else) are removed once the new copy is on disk, and
/// one line on stderr says how many.
///
/// ⚠️ **Always 0600** (VAL-2, REM-6): the copy holds the same secrets as the
/// original, and a 644 original gave a 644 backup. The mode is set on the temp
/// file before it is renamed, so it is never readable by others for a moment
/// either. A symlink where the backup goes is replaced, not written through.
///
/// Then [`crate::core::backups::after_backup`] records it in
/// `.evnx/backups.json` (FLD-9) and makes git ignore it. A command with its own
/// `--gitignore` choice (`cloud pull`, `platform pull`, `restore`) protects the
/// copy with that choice instead: [`backup_unignored`].
///
/// `validate --fix`, `restore`, `diff --apply`, `remove`, `cloud pull` and
/// `platform pull` are the callers; each says where the copy went with
/// [`Backup::describe`] and [`Backup::undo`].
pub fn backup(path: impl AsRef<Path>, content: &[u8]) -> Result<Backup> {
    keep(path.as_ref(), content, true)
}

/// [`backup`], leaving `.gitignore` to the caller's own `--gitignore` choice
/// (`protect::protect_backup`). Still private and recorded.
pub fn backup_unignored(path: impl AsRef<Path>, content: &[u8]) -> Result<Backup> {
    keep(path.as_ref(), content, false)
}

fn keep(path: &Path, content: &[u8], ignore: bool) -> Result<Backup> {
    let base = path.as_os_str().to_string_lossy();
    let candidate = backup_path(path);
    if let Ok(meta) = std::fs::symlink_metadata(&candidate) {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&candidate)
                .with_context(|| format!("could not replace the link {}", candidate.display()))?;
        } else if meta.is_dir() {
            bail!(
                "refusing to change {base}: {} is a directory",
                candidate.display()
            );
        }
    }
    let replaced = candidate.exists();
    write_with_mode(&candidate, content, permissions::PRIVATE)
        .with_context(|| format!("could not keep a backup at {}", candidate.display()))?;
    crate::core::backups::after_backup(path, &candidate, ignore);
    let removed = remove_stale_backups(path);
    if removed > 0 && !crate::utils::ui::is_quiet() {
        eprintln!(
            "  removed {removed} old backup{} of {base} (only {base}.bak is kept)",
            if removed == 1 { "" } else { "s" }
        );
    }
    Ok(Backup {
        path: candidate,
        original: path.to_path_buf(),
        replaced,
    })
}

/// A backup [`backup`] kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// `<file>.bak`.
    pub path: PathBuf,
    /// The file it is a copy of.
    pub original: PathBuf,
    /// An older backup of the same file was there, and is gone now (REM-4:
    /// one slot per file, shared by every command).
    pub replaced: bool,
}

impl Backup {
    /// `.env.bak`, or `.env.bak (replaced the previous backup)`.
    pub fn describe(&self) -> String {
        if self.replaced {
            format!("{} (replaced the previous backup)", self.path.display())
        } else {
            self.path.display().to_string()
        }
    }

    /// `evnx undo .env`: how to put the file back as it was (`evnx undo`
    /// swaps the file and this copy; owner, 2026-11-16).
    pub fn undo(&self) -> String {
        format!("evnx undo {}", sh_quote(&self.file_shown()))
    }

    /// The original as a person types it: without a leading `./` (`.` and a
    /// backslash on Windows), so a hint reads the same on every platform.
    pub fn file_shown(&self) -> String {
        match self.original.strip_prefix(".") {
            Ok(rest) if !rest.as_os_str().is_empty() => rest.to_string_lossy().into_owned(),
            _ => self.original.to_string_lossy().into_owned(),
        }
    }
}

/// A path as one shell word: bare when it is only safe characters, else in
/// single quotes.
fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-~:\\".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

impl std::ops::Deref for Backup {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Backup {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Where [`backup`] keeps `path`'s copy: `<path>.bak`.
///
/// Public so a command can ask, before it writes anything, whether git would
/// see that copy: a `.gitignore` that covers `.env` need not cover `.env.bak`.
pub fn backup_path(path: impl AsRef<Path>) -> PathBuf {
    let mut name = path.as_ref().as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

/// Remove `<path>.bak.<digits>` beside `path` (the numbered copies older
/// versions made) and return how many went. Matches that exact shape only:
/// `.bak.old`, `notes.bak.txt`, `.bak.2x`, `.bak.` and directories stay.
/// Best effort: a copy that cannot be removed is left and not counted.
fn remove_stale_backups(path: &Path) -> usize {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return 0;
    };
    let prefix = format!("{name}.bak.");
    let dir = target_dir(path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = file_name.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let p = entry.path();
        match std::fs::symlink_metadata(&p) {
            Ok(m) if !m.is_dir() && std::fs::remove_file(&p).is_ok() => removed += 1,
            _ => {}
        }
    }
    removed
}

/// One file a [`replace_all`] changes.
#[derive(Debug, Clone, Copy)]
pub struct Replacement<'a> {
    pub path: &'a Path,
    /// How errors name it.
    pub label: &'a str,
    /// What it held when it was read: compared just before it is replaced, and
    /// written back if a later file fails.
    pub original: &'a [u8],
    pub content: &'a [u8],
    /// Holds secrets: written as [`write_private`] writes, else as
    /// [`write_shared`] does.
    pub private: bool,
    /// Keep a [`backup`] of `original` first.
    pub backup: bool,
}

/// Replace several files as nearly together as files allow. Returns each
/// file's backup, in order.
///
/// 1. Every file is locked ([`crate::core::lock`]) and checked to still hold
///    `original`; then every backup is taken and every new content staged. A
///    failure in any of these changes nothing visible.
/// 2. Each file is committed only if it still holds `original`, so an edit made
///    since it was read is never overwritten (compare and swap).
/// 3. When a commit fails, the files already committed are written back as
///    they were, and the error says which were put back, which could not be
///    (their backups hold the originals), and which were not reached.
///
/// ⚠️ Not a transaction: another process can see the first file changed before
/// the last one is, and a crash between commits leaves some changed. Each file
/// on its own is replaced atomically.
///
/// `before_commit(i, path)` runs just before file `i` is committed. Tests use it
/// to change a file under the run or to make a write fail; callers pass a
/// closure that does nothing.
///
/// `remove` (S-5, S-6) and `align` share this, so a fix to the rollback is made
/// once.
pub fn replace_all(
    files: &[Replacement<'_>],
    before_commit: &mut dyn FnMut(usize, &Path),
) -> Result<Vec<Option<Backup>>> {
    // E-7: one run at a time per file. Held until this function returns, so the
    // check below, the backups and the swaps all happen under it. Sorted, and
    // never waited for, so two runs cannot deadlock: the second stops.
    let mut targets: Vec<&Path> = files.iter().map(|f| f.path).collect();
    targets.sort();
    targets.dedup();
    let mut _locks = Vec::with_capacity(targets.len());
    for t in targets {
        _locks.push(crate::core::lock::acquire(t)?);
    }
    // A run that held the lock before us may have finished since we read: stop
    // before taking a backup of a file that is no longer what we planned from.
    for f in files {
        if !matches!(std::fs::read(f.path), Ok(now) if now == f.original) {
            bail!("{} changed while evnx was running", f.label);
        }
    }
    let mut backups = Vec::with_capacity(files.len());
    for f in files {
        backups.push(if f.backup {
            Some(backup(f.path, f.original)?)
        } else {
            None
        });
    }
    let mut staged = Vec::with_capacity(files.len());
    for f in files {
        staged.push(if f.private {
            stage_private(f.path, f.content)?
        } else {
            stage_shared(f.path, f.content)?
        });
    }
    let mut done: Vec<usize> = Vec::new();
    for (i, (f, s)) in files.iter().zip(staged).enumerate() {
        before_commit(i, f.path);
        let result = match std::fs::read(f.path) {
            Ok(now) if now == f.original => s.commit().map(|w| report(&w)),
            Ok(_) => Err(anyhow::anyhow!(
                "{} changed while evnx was running",
                f.label
            )),
            Err(e) => Err(anyhow::Error::from(e).context(format!("reading {}", f.label))),
        };
        if let Err(e) = result {
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            for &j in &done {
                let q = &files[j];
                let put_back = if q.private {
                    write_private(q.path, q.original)
                } else {
                    write_shared(q.path, q.original)
                };
                match put_back {
                    Ok(_) => restored.push(q.label),
                    Err(_) => failed.push(q.label),
                }
            }
            let mut msg = format!("{e:#}\n  {} was not changed.", f.label);
            if !restored.is_empty() {
                msg.push_str(&format!(
                    "\n  Put back as they were: {}.",
                    restored.join(", ")
                ));
            }
            if !failed.is_empty() {
                msg.push_str(&format!(
                    "\n  ⚠ Could not put back: {}. Their .bak copies hold the originals.",
                    failed.join(", ")
                ));
            }
            let untouched: Vec<&str> = files[i + 1..].iter().map(|f| f.label).collect();
            if !untouched.is_empty() {
                msg.push_str(&format!("\n  Not reached: {}.", untouched.join(", ")));
            }
            bail!(msg);
        }
        done.push(i);
    }
    Ok(backups)
}

/// Stage a private write: the bytes are in a temp file beside the target, and
/// nothing visible has changed until [`Staged::commit`]. For callers that must
/// do something between the two — `backup` rotates old copies only once the new
/// one is safely on disk (DEC-B7, I-150).
pub fn stage_private(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Staged> {
    stage(
        path.as_ref(),
        bytes,
        Mode::NewOnly(permissions::PRIVATE),
        true,
    )
}

/// [`stage_private`] for a file meant to be shared (`.env.example`,
/// `.evnx.toml`): new files 0644, and an existing one's mode is not reported.
pub fn stage_shared(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Staged> {
    stage(
        path.as_ref(),
        bytes,
        Mode::NewOnly(permissions::SHARED),
        false,
    )
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    NewOnly(u32),
    Always(u32),
}

/// A write that has been prepared but not made visible. Dropping it without
/// [`commit`](Staged::commit) deletes the temp file and leaves the target as it
/// was.
#[derive(Debug)]
pub struct Staged {
    tmp: tempfile::NamedTempFile,
    path: PathBuf,
    target: PathBuf,
    dir: PathBuf,
    through_link: bool,
    mode: Mode,
    holds_secrets: bool,
}

fn write(path: &Path, bytes: &[u8], mode: Mode, holds_secrets: bool) -> Result<Written> {
    stage(path, bytes, mode, holds_secrets)?.commit()
}

fn stage(path: &Path, bytes: &[u8], mode: Mode, holds_secrets: bool) -> Result<Staged> {
    let (target, through_link) = resolve(path)?;
    existing_file(&target)?;

    let dir = target_dir(&target);
    if !dir.is_dir() {
        bail!("the directory {} does not exist", dir.display());
    }
    // `tempfile` opens with O_EXCL and mode 0600: nothing else can have created
    // it, and nothing else can read it while it fills.
    let mut tmp = tempfile::Builder::new()
        .prefix(".evnx-")
        .suffix(".tmp")
        .tempfile_in(&dir)
        .with_context(|| format!("could not create a temporary file in {}", dir.display()))?;
    tmp.write_all(bytes)
        .and_then(|_| tmp.as_file().sync_all())
        .with_context(|| format!("could not write {}", target.display()))?;

    Ok(Staged {
        tmp,
        path: path.to_path_buf(),
        target,
        dir,
        through_link,
        mode,
        holds_secrets,
    })
}

/// The directory a write to `target` goes through (its temp file lives there).
fn target_dir(target: &Path) -> PathBuf {
    match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// What a write to a path would do, without writing (WP-C6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The path the caller asked for.
    pub path: PathBuf,
    /// The file that would be created or replaced: `path`, or a symlink's target.
    pub target: PathBuf,
    /// `path` is a symlink and the write would go to its target.
    pub through_link: bool,
    /// A file already exists at `target` and would be replaced.
    pub replaces: bool,
    /// The directory the file would go in, when it does not exist: the write
    /// would fail unless it is created first.
    pub missing_dir: Option<PathBuf>,
}

/// What [`write_private`] (or its siblings) would do with `path`, worked out
/// the way the write works it out — the same symlink resolution, the same
/// directory rule — so a `--dry-run` cannot promise something the write would
/// not do. An error is one the write would also hit (a directory, or a chain
/// of links, where the file should be).
pub fn preview(path: impl AsRef<Path>) -> Result<Preview> {
    let path = path.as_ref();
    let (target, through_link) = resolve(path)?;
    let replaces = existing_file(&target)?.is_some();
    let dir = target_dir(&target);
    Ok(Preview {
        path: path.to_path_buf(),
        target,
        through_link,
        replaces,
        missing_dir: (!dir.is_dir()).then_some(dir),
    })
}

/// Whether an output path means stdout: exactly `-` (`template --output -`,
/// DEC-T8; `cloud pull --file -`, I-244). One rule, so `./-` stays a file
/// named `-` everywhere.
pub fn is_stdout(path: impl AsRef<Path>) -> bool {
    path.as_ref() == Path::new("-")
}

/// Create `dir` and every missing parent, as `mkdir -p` does. Returns the
/// directories that did not exist, outermost first, so a command can say what
/// it made. One helper for `template --mkdir`, `init` and `cloud export`.
pub fn create_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|a| !a.as_os_str().is_empty() && !a.exists())
        .map(Path::to_path_buf)
        .collect();
    missing.reverse();
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    Ok(missing)
}

/// [`create_dirs`], with every directory it creates owner-only (0700) where the
/// platform expresses that, and `dir` itself too if it already existed (an
/// empty `cloud export` target). Directories above that already existed keep
/// their mode.
///
/// 0700 rather than the umask default: every file inside holds plaintext
/// secrets, and a directory others cannot list is one less way to find them.
///
/// ⚠️ Every created one, not only `dir` (I-144): `template --mkdir
/// deploy/k8s/app.yml` left `deploy` at the umask's mode while `--help`
/// promised 0700.
pub fn create_dirs_private(dir: &Path) -> Result<Vec<PathBuf>> {
    let created = create_dirs(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let leaf = (!created.iter().any(|c| c == dir)).then_some(dir);
        for d in created.iter().map(PathBuf::as_path).chain(leaf) {
            let mut perms = std::fs::metadata(d)
                .with_context(|| format!("reading permissions of {}", d.display()))?
                .permissions();
            perms.set_mode(permissions::PRIVATE_DIR);
            std::fs::set_permissions(d, perms)
                .with_context(|| format!("setting permissions on {}", d.display()))?;
        }
    }
    Ok(created)
}

/// The file at `target`, if one exists; an error if something else is there.
fn existing_file(target: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::metadata(target) {
        Ok(m) if m.is_file() => Ok(Some(m)),
        Ok(_) => bail!("{} exists and is not a regular file", target.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", target.display())),
    }
}

impl Staged {
    /// Make the write visible: rename the temp file over the target. The mode
    /// is decided here, against what is at the target **now** — after a
    /// rotation moved the old file away, the new one is a new file.
    pub fn commit(self) -> Result<Written> {
        let existing = existing_file(&self.target)?;
        let final_mode = match (self.mode, &existing) {
            (Mode::Always(m), _) => Some(m),
            (Mode::NewOnly(_), Some(meta)) => existing_mode(meta),
            (Mode::NewOnly(m), None) => Some(m),
        };
        #[cfg(unix)]
        if let Some(m) = final_mode {
            use std::os::unix::fs::PermissionsExt;
            self.tmp
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(m))
                .with_context(|| format!("could not set the mode of {}", self.target.display()))?;
        }

        // Windows has no mode to set: a file meant to be private gets an ACL
        // that leaves it to this account (a warning when that fails; the
        // write is not undone). Before the rename, so the file is never
        // visible with the folder's ACL.
        let requested = match self.mode {
            Mode::Always(m) | Mode::NewOnly(m) => m,
        };
        if !permissions::MODES_APPLY && permissions::is_private_mode(requested) {
            permissions::owner_only(self.tmp.path(), permissions::AclFailure::Warn)?;
        }

        self.tmp
            .persist(&self.target)
            .map_err(|e| e.error)
            .with_context(|| format!("could not replace {}", self.target.display()))?;
        sync_dir(&self.dir);

        let loose_mode = match (self.holds_secrets, &existing, final_mode) {
            (true, Some(_), Some(m))
                if permissions::MODES_APPLY && !permissions::is_private_mode(m) =>
            {
                Some(m)
            }
            _ => None,
        };
        Ok(Written {
            path: self.path,
            target: self.target,
            through_link: self.through_link,
            replaced: existing.is_some(),
            loose_mode,
        })
    }
}

/// Follow `path` while it is a symlink. A dangling link resolves to the file it
/// names, which the write then creates.
fn resolve(path: &Path) -> Result<(PathBuf, bool)> {
    let mut current = path.to_path_buf();
    let mut through = false;
    for _ in 0..40 {
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = std::fs::read_link(&current)
                    .with_context(|| format!("reading the link {}", current.display()))?;
                current = if link.is_absolute() {
                    link
                } else {
                    current.parent().unwrap_or(Path::new(".")).join(link)
                };
                through = true;
            }
            _ => return Ok((current, through)),
        }
    }
    bail!("{} is a chain of more than 40 symlinks", path.display())
}

#[cfg(unix)]
fn existing_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn existing_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Make the rename durable. Best effort: some filesystems refuse to open a
/// directory, and the write itself has already succeeded.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Whether `a` and `b` name the same file: the same path, a symlink to it, or
/// a hard link (same device and inode). A missing path is never the same file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

/// Refuse an `--output` that names `input` — by path, symlink or hard link —
/// which writing would replace with its own result. `what` names the input
/// ("template", "env file").
///
/// One wording for `template` (I-131) and `convert` (I-081), so a person who
/// meets it twice reads the same rule twice.
pub fn refuse_same_file(output: &str, what: &str, input: &str) -> Result<()> {
    if same_file(Path::new(input), Path::new(output)) {
        bail!(
            "--output '{output}' is the same file as the {what} '{input}'. \
             Nothing was written; choose another --output."
        );
    }
    Ok(())
}

/// Print what the person should know about a write: that it went through a
/// symlink (DEC-S6), and that a file holding secrets is readable by others
/// (DEC-G6). Commands print their own success line.
pub fn report(w: &Written) {
    report_link(w);
    if let Some(m) = w.loose_mode {
        permissions::readable_warning(&w.path, m, Some("evnx keeps the mode you set"));
    }
}

/// The symlink half of [`report`], for a command that has already warned about
/// the file's mode before writing it (`sync`, I-276).
pub fn report_link(w: &Written) {
    if w.through_link {
        crate::utils::ui::info(format!(
            "wrote through {} -> {}",
            w.path.display(),
            w.target.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_and_replaces_content() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("out.env");
        let w = write_private(&p, b"A=1\n").unwrap();
        assert!(!w.replaced && !w.through_link);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "A=1\n");
        let w = write_private(&p, b"A=2\n").unwrap();
        assert!(w.replaced);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "A=2\n");
    }

    #[test]
    fn a_second_backup_replaces_the_first() {
        let d = TempDir::new().unwrap();
        let p = d.path().join(".env");
        std::fs::write(&p, "A=1\n").unwrap();
        let first = backup(&p, b"A=1\n").unwrap();
        let second = backup(&p, b"A=2\n").unwrap();
        assert_eq!(first.path, d.path().join(".env.bak"));
        assert!(!first.replaced);
        assert_eq!(second.path, first.path);
        assert!(second.replaced, "REM-4: says it replaced the first");
        assert_eq!(second.undo(), format!("evnx undo {}", p.display()));
        assert_eq!(std::fs::read_to_string(&*second).unwrap(), "A=2\n");
        assert!(!d.path().join(".env.bak.2").exists());
    }

    #[test]
    fn legacy_numbered_backups_are_removed_and_nothing_else() {
        let d = TempDir::new().unwrap();
        let p = d.path().join(".env");
        std::fs::write(&p, "A=1\n").unwrap();
        for n in [".env.bak", ".env.bak.2", ".env.bak.3", ".env.bak.100"] {
            std::fs::write(d.path().join(n), "old\n").unwrap();
        }
        let mut keep = vec![
            ".env.bak.old",
            ".env.bak.2x",
            ".env.production.bak.2",
            "notes.bak.txt",
            ".envx.bak.2",
            ".env.bak.2.txt",
        ];
        // Windows drops a trailing dot from a file name, so `.env.bak.` is
        // `.env.bak` there: the backup itself, not a file to leave alone.
        if !cfg!(windows) {
            keep.push(".env.bak.");
        }
        for n in &keep {
            std::fs::write(d.path().join(n), "keep\n").unwrap();
        }
        std::fs::create_dir(d.path().join(".env.bak.7")).unwrap();
        backup(&p, b"A=1\n").unwrap();
        for n in [".env.bak.2", ".env.bak.3", ".env.bak.100"] {
            assert!(!d.path().join(n).exists(), "{n} should be gone");
        }
        for n in &keep {
            assert_eq!(
                std::fs::read_to_string(d.path().join(n)).unwrap(),
                "keep\n",
                "{n}"
            );
        }
        assert!(d.path().join(".env.bak.7").is_dir());
        assert_eq!(
            std::fs::read_to_string(d.path().join(".env.bak")).unwrap(),
            "A=1\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn vals2_a_backup_is_private_whatever_the_originals_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new().unwrap();
        let p = d.path().join(".env");
        std::fs::write(&p, "A=1\n").unwrap();
        for original in [0o644, 0o640, 0o600] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(original)).unwrap();
            let b = backup(&p, b"A=1\n").unwrap();
            let mode = std::fs::metadata(&*b).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "original {original:o}");
        }
    }

    #[test]
    fn leaves_no_temporary_file_behind() {
        let d = TempDir::new().unwrap();
        write_private(d.path().join("a"), b"x").unwrap();
        write_shared(d.path().join("a"), b"y").unwrap();
        let names: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a")]);
    }

    #[test]
    fn a_missing_directory_is_an_error_naming_it() {
        let d = TempDir::new().unwrap();
        let err = write_private(d.path().join("nodir/x"), b"x").unwrap_err();
        assert!(format!("{err:#}").contains("nodir"), "{err:#}");
    }

    #[test]
    fn a_directory_at_the_path_is_refused() {
        let d = TempDir::new().unwrap();
        std::fs::create_dir(d.path().join("out")).unwrap();
        let err = write_private(d.path().join("out"), b"x").unwrap_err();
        assert!(format!("{err:#}").contains("not a regular file"), "{err:#}");
    }

    #[test]
    fn a_staged_write_changes_nothing_until_commit() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("out");
        std::fs::write(&p, "old").unwrap();
        let staged = stage_private(&p, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
        staged.commit().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");

        // Dropped without commit: target untouched, no temp file left.
        drop(stage_private(&p, b"never").unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn is_stdout_is_exactly_a_dash() {
        assert!(is_stdout("-"));
        for p in ["./-", "--", "-x", "", "out"] {
            assert!(!is_stdout(p), "{p}");
        }
    }

    #[test]
    fn preview_says_new_replace_and_missing_directory_and_writes_nothing() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("out.yml");
        let v = preview(&p).unwrap();
        assert!(!v.replaces && !v.through_link && v.missing_dir.is_none());
        assert!(!p.exists(), "a preview writes nothing");
        std::fs::write(&p, "x").unwrap();
        assert!(preview(&p).unwrap().replaces);
        let deep = d.path().join("a/b/out.yml");
        assert_eq!(
            preview(&deep).unwrap().missing_dir,
            Some(d.path().join("a/b"))
        );
        assert!(!d.path().join("a").exists());
        assert!(
            preview(d.path()).is_err(),
            "a directory where the file goes"
        );
    }

    #[test]
    fn create_dirs_names_what_it_made_outermost_first() {
        let d = TempDir::new().unwrap();
        let deep = d.path().join("a/b/c");
        assert_eq!(
            create_dirs(&deep).unwrap(),
            vec![d.path().join("a"), d.path().join("a/b"), deep.clone()]
        );
        assert!(deep.is_dir());
        assert!(
            create_dirs(&deep).unwrap().is_empty(),
            "nothing new the second time"
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn mode(p: &Path) -> u32 {
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn new_files_get_0600_or_0644() {
            let d = TempDir::new().unwrap();
            write_private(d.path().join("secret"), b"x").unwrap();
            write_shared(d.path().join("example"), b"x").unwrap();
            assert_eq!(mode(&d.path().join("secret")), 0o600);
            assert_eq!(mode(&d.path().join("example")), 0o644);
        }

        #[test]
        fn an_existing_file_keeps_its_mode_and_a_loose_one_is_reported() {
            let d = TempDir::new().unwrap();
            let p = d.path().join(".env");
            std::fs::write(&p, "old").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
            let w = write_private(&p, b"new").unwrap();
            assert_eq!(mode(&p), 0o640, "DEC-G6: never change an existing mode");
            assert_eq!(w.loose_mode, Some(0o640));

            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(write_private(&p, b"x").unwrap().loose_mode, None);
        }

        #[test]
        fn a_shared_file_is_never_reported_as_loose() {
            let d = TempDir::new().unwrap();
            let p = d.path().join(".env.example");
            std::fs::write(&p, "A=").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(write_shared(&p, b"A=\n").unwrap().loose_mode, None);
        }

        #[test]
        fn write_with_mode_sets_it_on_an_existing_file() {
            let d = TempDir::new().unwrap();
            let p = d.path().join("out.yml");
            std::fs::write(&p, "old").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            write_with_mode(&p, b"new", 0o640).unwrap();
            assert_eq!(mode(&p), 0o640);
        }

        #[test]
        fn a_symlink_is_written_through_and_stays_a_link() {
            let d = TempDir::new().unwrap();
            std::fs::create_dir(d.path().join("shared")).unwrap();
            let real = d.path().join("shared/real.example");
            std::fs::write(&real, "OLD=1\n").unwrap();
            let link = d.path().join(".env.example");
            std::os::unix::fs::symlink("shared/real.example", &link).unwrap();

            let w = write_shared(&link, b"NEW=1\n").unwrap();
            assert!(w.through_link);
            assert_eq!(w.target, d.path().join("shared/real.example"));
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read_to_string(&real).unwrap(), "NEW=1\n");
        }

        #[test]
        fn a_dangling_link_creates_its_target() {
            let d = TempDir::new().unwrap();
            let link = d.path().join("out");
            std::os::unix::fs::symlink("made.txt", &link).unwrap();
            write_private(&link, b"x").unwrap();
            assert_eq!(
                std::fs::read_to_string(d.path().join("made.txt")).unwrap(),
                "x"
            );
            assert_eq!(mode(&d.path().join("made.txt")), 0o600);
        }

        #[test]
        fn same_file_sees_paths_links_and_hard_links() {
            let d = TempDir::new().unwrap();
            let a = d.path().join("a");
            std::fs::write(&a, "x").unwrap();
            std::os::unix::fs::symlink("a", d.path().join("sym")).unwrap();
            std::fs::hard_link(&a, d.path().join("hard")).unwrap();
            std::fs::write(d.path().join("other"), "x").unwrap();
            assert!(same_file(&a, &d.path().join("./a")));
            assert!(same_file(&a, &d.path().join("sym")));
            assert!(same_file(&a, &d.path().join("hard")));
            assert!(!same_file(&a, &d.path().join("other")));
            assert!(!same_file(&a, &d.path().join("missing")));
        }
        #[test]
        fn preview_follows_a_link_like_the_write() {
            let d = TempDir::new().unwrap();
            std::fs::write(d.path().join("real.yml"), "x").unwrap();
            let link = d.path().join("link.yml");
            std::os::unix::fs::symlink("real.yml", &link).unwrap();
            let v = preview(&link).unwrap();
            assert!(v.through_link && v.replaces);
            assert_eq!(v.target, d.path().join("real.yml"));
        }

        #[test]
        fn create_dirs_private_makes_the_leaf_owner_only() {
            let d = TempDir::new().unwrap();
            let deep = d.path().join("a/secrets");
            create_dirs_private(&deep).unwrap();
            assert_eq!(mode(&deep), 0o700);
        }

        /// I-144: every directory it creates is 0700, not only the last one;
        /// one that already existed keeps its mode.
        #[test]
        fn create_dirs_private_makes_every_created_directory_owner_only() {
            use std::os::unix::fs::PermissionsExt;
            let d = TempDir::new().unwrap();
            let existing = d.path().join("existing");
            std::fs::create_dir(&existing).unwrap();
            std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
            create_dirs_private(&existing.join("a/b")).unwrap();
            assert_eq!(mode(&existing), 0o755);
            assert_eq!(mode(&existing.join("a")), 0o700);
            assert_eq!(mode(&existing.join("a/b")), 0o700);
        }
    }
}
