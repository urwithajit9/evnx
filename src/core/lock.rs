//! An advisory lock for one file being rewritten (N14-13, plan E-7).
//!
//! `remove` and `sync --prune` read a file, plan its new text and later swap it
//! in. A compare-and-swap check just before the swap catches a change made in
//! between, but two runs can both pass it and then rename in turn, and the
//! slower one silently undoes the faster. The lock closes that window: while
//! one run holds a file, another run on it stops with a message.
//!
//! # Why a lock file, and not `flock`
//!
//! No locking crate (`fs2`, `fd-lock`, `fs4`) is a dependency, and the standard
//! library's `File::lock` needs Rust 1.89 where this crate builds on 1.87. A
//! `create_new` (`O_EXCL`) file beside the target needs neither, works the same
//! on every platform, and carries the owner's process id so a crash does not
//! lock the file for ever.
//!
//! # Stale locks
//!
//! A lock whose owner is gone is replaced: on Unix when `kill(pid, 0)` says the
//! process does not exist, elsewhere (and when the id cannot be read) when the
//! lock is older than [`STALE_AFTER`]. A run holds its locks for milliseconds,
//! so a lock that old is not a running evnx. The lock never waits: the loser
//! stops at once and says whose lock it is, because a retry loop could stall a
//! CI job on a wedged lock far longer than the edit it protects.
//!
//! The one race left is two runs replacing the same stale lock at the same
//! moment; `create_new` still lets only one of them win, and the other stops.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// A lock with no live owner this old is taken over.
pub const STALE_AFTER: Duration = Duration::from_secs(600);

/// Held while a file is being rewritten; the lock file is removed on drop.
#[derive(Debug)]
pub struct FileLock {
    lock: PathBuf,
}

/// The lock file for `target`: `.evnx-<file name>.lock` in its folder. The name
/// does not start with `.env`, so no evnx command reads it as an environment.
pub fn lock_path(target: &Path) -> PathBuf {
    let real = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    real.with_file_name(format!(".evnx-{name}.lock"))
}

/// Take the lock on `target`, or fail naming who holds it.
pub fn acquire(target: &Path) -> Result<FileLock> {
    let lock = lock_path(target);
    for _ in 0..2 {
        match create(&lock) {
            Ok(()) => return Ok(FileLock { lock }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match owner(&lock) {
                    Owner::Stale => {
                        // Take it over; `create_new` above decides a tie.
                        let _ = std::fs::remove_file(&lock);
                    }
                    Owner::Live(pid) => bail!(held(target, &lock, Some(pid))),
                    Owner::Unknown => bail!(held(target, &lock, None)),
                }
            }
            Err(e) => {
                return Err(e).with_context(|| format!("could not lock {}", target.display()))
            }
        }
    }
    bail!(held(target, &lock, None))
}

fn held(target: &Path, lock: &Path, pid: Option<u32>) -> String {
    format!(
        "another evnx{} is changing {}, so nothing was changed.\n  Wait for it and run this again. \
         If no evnx is running, delete {}.",
        pid.map(|p| format!(" (process {p})")).unwrap_or_default(),
        target.display(),
        lock.display()
    )
}

fn create(lock: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock)?;
    writeln!(f, "{}", std::process::id())?;
    f.sync_all()
}

enum Owner {
    Live(u32),
    Stale,
    /// Held by someone, but not provably dead and not old enough to take.
    Unknown,
}

fn owner(lock: &Path) -> Owner {
    let pid = std::fs::read_to_string(lock)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok());
    if let Some(pid) = pid {
        match alive(pid) {
            Some(true) => return Owner::Live(pid),
            Some(false) => return Owner::Stale,
            None => {}
        }
    }
    let old = std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age >= STALE_AFTER);
    if old {
        Owner::Stale
    } else {
        match pid {
            Some(p) => Owner::Live(p),
            None => Owner::Unknown,
        }
    }
}

/// Whether process `pid` exists; `None` when this platform cannot say.
#[cfg(unix)]
fn alive(pid: u32) -> Option<bool> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: signal 0 only checks that the process can be signalled.
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return Some(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Some(false),
        // EPERM: it exists and belongs to someone else.
        Some(libc::EPERM) => Some(true),
        _ => None,
    }
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> Option<bool> {
    None
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_on_the_same_file_is_refused_until_the_first_drops() {
        let d = tempfile::TempDir::new().unwrap();
        let f = d.path().join(".env");
        std::fs::write(&f, "A=1\n").unwrap();
        let first = acquire(&f).unwrap();
        let err = acquire(&f).unwrap_err().to_string();
        assert!(err.contains("another evnx"), "{err}");
        assert!(err.contains(&std::process::id().to_string()), "{err}");
        drop(first);
        assert!(!lock_path(&f).exists());
        acquire(&f).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_whose_owner_is_gone_is_taken_over() {
        let d = tempfile::TempDir::new().unwrap();
        let f = d.path().join(".env");
        std::fs::write(&f, "A=1\n").unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        std::fs::write(lock_path(&f), format!("{pid}\n")).unwrap();
        acquire(&f).unwrap();
    }

    #[test]
    fn a_lock_with_no_readable_owner_is_held_until_it_is_old() {
        let d = tempfile::TempDir::new().unwrap();
        let f = d.path().join(".env");
        std::fs::write(&f, "A=1\n").unwrap();
        std::fs::write(lock_path(&f), "garbage").unwrap();
        assert!(acquire(&f).is_err());
    }

    #[test]
    fn the_lock_file_is_not_an_environment_file() {
        let name = lock_path(Path::new("/x/.env.staging"));
        let name = name.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            crate::core::env_name::kind(&name),
            crate::core::env_name::FileKind::Other
        );
    }
}
