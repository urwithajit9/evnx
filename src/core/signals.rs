//! Signals for `evnx run` and `evnx cloud run` (WP-11a). Unix only; the
//! `unsafe` of this feature lives here and nowhere else.
//!
//! # Design note
//!
//! **Two ways to run the child, chosen by whether evnx has work left.**
//!
//! 1. *Nothing left to do* (no masking): evnx `exec`s the child after all setup
//!    (`inject::run_with_env`). evnx is gone; signals, the process group, the
//!    terminal and the exit status belong to the child. A signal death reaches
//!    the shell as 128+n with no help from us. This module is not used.
//! 2. *Output to mask*: evnx must stay to copy the child's output, so it forwards
//!    signals ([`Forwarder`]). Everything below is about this path.
//!
//! **Order, which is the point of the module** (`Forwarder::spawn`):
//!
//! 1. Block `SIGTERM`, `SIGHUP`, `SIGQUIT` and `SIGINT` in the calling thread.
//! 2. Install the forwarding handler for each (this replaces the `SIGINT` handler
//!    of `utils::term`, which would re-raise and kill evnx before the child had
//!    cleaned up). The previous actions are saved.
//! 3. `spawn`. std resets the mask in the child, so the child starts unblocked.
//! 4. Store the child's pid in a static `AtomicI32`.
//! 5. Restore the mask. A signal that arrived while blocked is delivered now, with
//!    the pid in place. A signal that another thread received before step 4 finds
//!    pid 0 and is recorded in `PENDING`; step 4 forwards those too, so none is lost.
//! 6. After the output is copied, `waitid(WNOWAIT)` (Linux) waits without reaping,
//!    the pid is cleared, and only then does `wait` reap. A signal can therefore
//!    never be sent to a pid the kernel may already have given to someone else.
//!    The handlers are restored last (also in `Drop`, so an early return or a panic
//!    leaves the process as it was).
//!
//! **The handler** does two things, both async-signal-safe: load the pid and call
//! `kill(pid, sig)` (or set a bit in `PENDING`), saving and restoring `errno`. No
//! allocation, no lock, no printing.
//!
//! **Which signals.** All four are forwarded. A terminal Ctrl-C reaches both evnx
//! and the child (one foreground process group), so the child sees `SIGINT`
//! twice; we accept that, because `kill -INT <evnx>` from a script must reach the
//! child too and the two cases cannot be told apart in a handler. Only the direct
//! child is signalled, not its descendants (it decides what to do with its own).
//!
//! **Exit code.** The child's code passes through. Death by signal n exits
//! 128+n and says so on stderr unless `-q` (`inject::exit_as`). On this path the
//! code goes back through `main`, so destructors (Masker zeroize) run.
//!
//! **One forwarder at a time**: the pid is process-wide, so a mutex is held for
//! the forwarder's life (uncontended in evnx; it serialises unit tests).
//!
//! **Windows**: none of this applies. The child is spawned and waited for; there
//! is no job object, so a child can outlive evnx when the console is closed or
//! evnx is killed, and Ctrl-C reaches both only through the console. Exit codes
//! pass through unchanged. Documented, not engineered (owner decision, 2026-11-17).
//!
//! `SIGKILL` cannot be caught: if evnx is killed that way the child is orphaned.

use std::mem::MaybeUninit;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

/// The signals that are forwarded, in `PENDING` bit order.
const SIGNALS: [libc::c_int; 4] = [libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT, libc::SIGINT];

/// The child's pid; 0 when there is no child to signal.
static CHILD: AtomicI32 = AtomicI32::new(0);
/// Bit i is set when `SIGNALS[i]` arrived while there was no pid yet.
static PENDING: AtomicU32 = AtomicU32::new(0);
/// Held while a forwarder exists: the pid and the handlers are process-wide.
static ACTIVE: Mutex<()> = Mutex::new(());

extern "C" fn forward(sig: libc::c_int) {
    let saved = errno();
    let pid = CHILD.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: `kill` is async-signal-safe and takes only integers. `pid` is
        // the child's, cleared before the child is reaped (`Forwarder::wait`).
        unsafe {
            libc::kill(pid, sig);
        }
    } else if let Some(i) = SIGNALS.iter().position(|&s| s == sig) {
        PENDING.fetch_or(1 << i, Ordering::SeqCst);
    }
    set_errno(saved);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno slot; always valid.
    unsafe { libc::__errno_location() }
}
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno slot; always valid.
    unsafe { libc::__error() }
}
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd"
)))]
fn errno_ptr() -> *mut libc::c_int {
    std::ptr::null_mut()
}

fn errno() -> libc::c_int {
    let p = errno_ptr();
    if p.is_null() {
        return 0;
    }
    // SAFETY: `p` is the thread's errno slot (non-null checked above).
    unsafe { *p }
}

fn set_errno(value: libc::c_int) {
    let p = errno_ptr();
    if !p.is_null() {
        // SAFETY: `p` is the thread's errno slot (non-null checked above).
        unsafe { *p = value }
    }
}

fn signal_set() -> libc::sigset_t {
    let mut set = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `sigemptyset` initialises the set it is given; `sigaddset` is
    // then called on that initialised set with valid signal numbers.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for sig in SIGNALS {
            libc::sigaddset(set.as_mut_ptr(), sig);
        }
        set.assume_init()
    }
}

/// Forwards signals sent to evnx to the child while it runs. See the module doc.
pub struct Forwarder {
    _only_one: MutexGuard<'static, ()>,
    old_actions: Vec<libc::sigaction>,
    old_mask: libc::sigset_t,
    restored: bool,
}

impl Forwarder {
    /// Spawn `cmd` with signal forwarding in place before the child exists and
    /// no signal lost between spawn and the pid being stored.
    pub fn spawn(cmd: &mut Command) -> std::io::Result<(Child, Forwarder)> {
        let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        CHILD.store(0, Ordering::SeqCst);
        PENDING.store(0, Ordering::SeqCst);

        let block = signal_set();
        let mut old_mask = MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: both pointers are valid for the call; `old_mask` is written by it.
        // `pthread_sigmask` returns an error number (it does not set errno).
        let rc = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &block, old_mask.as_mut_ptr()) };
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc));
        }
        // SAFETY: initialised by the successful call above.
        let old_mask = unsafe { old_mask.assume_init() };

        // From here, `Drop` puts everything back, whatever happens.
        let mut me = Forwarder {
            _only_one: guard,
            old_actions: Vec::with_capacity(SIGNALS.len()),
            old_mask,
            restored: false,
        };

        for sig in SIGNALS {
            // SAFETY: a zeroed `sigaction` is a valid starting value (empty mask,
            // no flags); the fields we need are set below.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = forward as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_mask = signal_set();
            action.sa_flags = libc::SA_RESTART;
            let mut old = MaybeUninit::<libc::sigaction>::uninit();
            // SAFETY: `action` is fully initialised, `old` is written by the call,
            // and `forward` only calls async-signal-safe functions.
            let rc = unsafe { libc::sigaction(sig, &action, old.as_mut_ptr()) };
            if rc != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: initialised by the successful call above.
            let old = unsafe { old.assume_init() };
            if old.sa_sigaction == libc::SIG_IGN {
                // `nohup` left this ignored; the child must inherit that, and a
                // handler would be reset to the default by `exec`.
                // SAFETY: puts back the action `sigaction` just returned.
                unsafe {
                    libc::sigaction(sig, &old, std::ptr::null_mut());
                }
            }
            me.old_actions.push(old);
        }

        let child = cmd.spawn()?;
        CHILD.store(child.id() as libc::pid_t, Ordering::SeqCst);

        // Signals that another thread took before the pid existed.
        let pending = PENDING.swap(0, Ordering::SeqCst);
        for (i, sig) in SIGNALS.iter().enumerate() {
            if pending & (1 << i) != 0 {
                // SAFETY: an ordinary `kill` to our own child, which is not reaped.
                unsafe {
                    libc::kill(child.id() as libc::pid_t, *sig);
                }
            }
        }
        me.unblock();
        Ok((child, me))
    }

    fn unblock(&self) {
        // SAFETY: restores the mask saved by `spawn`; the old-set pointer may be null.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.old_mask, std::ptr::null_mut());
        }
    }

    /// Wait for the child. The pid is cleared before the child is reaped, then
    /// the handlers go back to what they were.
    pub fn wait(mut self, child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
        wait_without_reaping(child);
        CHILD.store(0, Ordering::SeqCst);
        self.restore();
        child.wait()
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        CHILD.store(0, Ordering::SeqCst);
        for (sig, old) in SIGNALS.iter().zip(&self.old_actions) {
            // SAFETY: puts back the action `sigaction` returned for this signal.
            unsafe {
                libc::sigaction(*sig, old, std::ptr::null_mut());
            }
        }
        self.unblock();
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Block until the child has exited, leaving it unreaped (so its pid cannot be
/// reused before the caller clears it). Linux only; elsewhere this does nothing
/// and `Child::wait` does both, with a very small window.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn wait_without_reaping(child: &Child) {
    let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
    loop {
        // SAFETY: `info` is valid writable memory for a `siginfo_t`; the pid is
        // our unreaped child's. `WNOWAIT` leaves the child waitable.
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn wait_without_reaping(_child: &Child) {}
