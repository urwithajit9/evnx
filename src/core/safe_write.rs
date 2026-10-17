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
//! written atomically.

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

/// [`create_dirs`], then `dir` itself owner-only (0700) where the platform
/// expresses that, whether it was just made or already there. Parents it
/// creates keep the umask.
///
/// 0700 rather than the umask default: every file inside holds plaintext
/// secrets, and a directory others cannot list is one less way to find them.
pub fn create_dirs_private(dir: &Path) -> Result<Vec<PathBuf>> {
    let created = create_dirs(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dir)
            .with_context(|| format!("reading permissions of {}", dir.display()))?
            .permissions();
        perms.set_mode(permissions::PRIVATE_DIR);
        std::fs::set_permissions(dir, perms)
            .with_context(|| format!("setting permissions on {}", dir.display()))?;
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

        self.tmp
            .persist(&self.target)
            .map_err(|e| e.error)
            .with_context(|| format!("could not replace {}", self.target.display()))?;
        sync_dir(&self.dir);

        let loose_mode = match (self.holds_secrets, &existing, final_mode) {
            (true, Some(_), Some(m)) if permissions::MODES_APPLY && m & 0o077 != 0 => Some(m),
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

/// Print what the person should know about a write: that it went through a
/// symlink (DEC-S6), and that a file holding secrets is readable by others
/// (DEC-G6). Commands print their own success line.
pub fn report(w: &Written) {
    use crate::utils::ui;
    if w.through_link {
        ui::info(format!(
            "wrote through {} -> {}",
            w.path.display(),
            w.target.display()
        ));
    }
    if let Some(m) = w.loose_mode {
        ui::warning(format!(
            "{} is readable by other users (mode {:o}); evnx keeps the mode you set",
            w.path.display(),
            m
        ));
        ui::warning_detail(format!("     →  {}", permissions::chmod_hint(&w.path)));
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
    }
}
