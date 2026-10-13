//! Interactive prompts, driven through a real pseudo-terminal (Unix only).
//!
//! # Why a pty
//!
//! evnx's prompts are dialoguer widgets, and dialoguer refuses a stdin that is
//! not a terminal (`IO error: not a terminal`). Keys written to a pipe never
//! reach a prompt, which is why the tests that tried it sat `#[ignore]`d and
//! could not pass even when run by hand (T-001 for `init`, T-002 for `add`).
//! This presses the same keys a person would. Moved here from
//! `tests/init_pty.rs` so every command's interactive tests share one driver.
//!
//! The Windows console is a different mechanism; `expectrl` is a `cfg(unix)`
//! dev-dependency and so is this module.

use expectrl::process::unix::WaitStatus;
use expectrl::{Eof, Expect, Session};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub const ENTER: &str = "\r";
pub const DOWN: &str = "\x1b[B";
pub const UP: &str = "\x1b[A";
pub const SPACE: &str = " ";
pub const ESC: &str = "\x1b";
pub const CTRL_C: &str = "\x03";

/// One interactive `evnx <args>` in `dir`, on a 110×40 terminal, without colour.
pub struct Pty {
    session: expectrl::session::OsSession,
}

impl Pty {
    /// Start `evnx <args>`. `args` is the whole command line, subcommand
    /// included (`&["add", "custom"]`).
    pub fn spawn(dir: &Path, args: &[&str]) -> Pty {
        let mut cmd = Command::new(super::evnx_bin());
        cmd.current_dir(dir)
            .args(args)
            .env("NO_COLOR", "1")
            .env("TERM", "xterm");
        let mut session = Session::spawn(cmd).expect("spawn evnx in a pty");
        session.set_expect_timeout(Some(Duration::from_secs(15)));
        session
            .get_process_mut()
            .set_window_size(110, 40)
            .expect("set pty size");
        Pty { session }
    }

    /// Wait until `text` has been printed; returns everything printed before it.
    #[track_caller]
    pub fn wait_for(&mut self, text: &str) -> String {
        let found = self
            .session
            .expect(text)
            .unwrap_or_else(|e| panic!("never saw {text:?}: {e}"));
        String::from_utf8_lossy(found.before()).to_string()
    }

    /// Like [`Pty::wait_for`], but give up after `timeout` and return `None`
    /// instead of failing the test. For keys that may or may not have been
    /// consumed (see [`Pty::cancel_yes_no`] and `add_pty.rs`'s Esc test).
    pub fn try_wait_for(&mut self, text: &str, timeout: Duration) -> Option<String> {
        self.session.set_expect_timeout(Some(timeout));
        let found = self
            .session
            .expect(text)
            .ok()
            .map(|f| String::from_utf8_lossy(f.before()).to_string());
        self.session
            .set_expect_timeout(Some(Duration::from_secs(15)));
        found
    }

    /// Wait until the program has switched terminal echo off: a hidden-input
    /// prompt is reading.
    ///
    /// ⚠️ Seeing the prompt text is not enough. `console` prints the prompt,
    /// *then* turns echo off with `TCSAFLUSH`, which discards input already
    /// waiting. Text typed between the two was thrown away and the prompt
    /// read only the Enter after it, an empty value (CI, 2026-10-14). The
    /// pty's line settings are shared by both ends, so the echo flag is read
    /// here. Gives up quietly after 5 s, on a platform where it cannot tell.
    pub fn wait_for_echo_off(&mut self) {
        use std::os::fd::AsRawFd;
        let fd = self.session.as_raw_fd();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
            // SAFETY: `fd` is the open pty master; tcgetattr fills `t` on 0.
            if unsafe { libc::tcgetattr(fd, t.as_mut_ptr()) } == 0 {
                let t = unsafe { t.assume_init() };
                if t.c_lflag & libc::ECHO == 0 {
                    // Settled: the read is next, and the flush is done.
                    std::thread::sleep(Duration::from_millis(50));
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Press keys one at a time, the way a person would. dialoguer reads one key
    /// per event, and a burst of escape sequences is not what it is built for.
    pub fn press(&mut self, keys: &[&str]) {
        for k in keys {
            self.session.send(k).expect("write to pty");
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Type `text` and press Enter, for a text input.
    pub fn type_line(&mut self, text: &str) {
        self.session.send(text).expect("write to pty");
        std::thread::sleep(Duration::from_millis(40));
        self.press(&[ENTER]);
    }

    /// Cancel a yes/no question: Esc, then Enter once Esc has been read.
    ///
    /// ⚠️ Not `press(&[ESC, ENTER])`. On Unix the `console` crate, after an
    /// ESC byte, checks with a zero timeout whether more bytes are waiting; if
    /// the Enter is already queued, the two are read as one unknown escape
    /// sequence, the Enter is swallowed, and the question never returns. That
    /// passed locally and hung on a slower CI runner (#156). After Esc the
    /// question is redrawn with no default, `[y/n]` (it showed `[Y/n]` or
    /// `[y/N]` before), so seeing that redraw proves the Esc was read on its own.
    pub fn cancel_yes_no(&mut self) {
        self.press(&[ESC]);
        self.wait_for("[y/n]");
        self.press(&[ENTER]);
    }

    /// Read to the end and return (remaining output, exit code). Panics if the
    /// process ended by a signal; use [`Pty::finish_raw`] for that case.
    pub fn finish(self) -> (String, i32) {
        let (rest, status) = self.finish_raw();
        let code = match status {
            WaitStatus::Exited(_, code) => code,
            other => panic!("evnx did not exit normally: {other:?}"),
        };
        (String::from_utf8_lossy(&rest).to_string(), code)
    }

    /// Read to the end and return the raw remaining bytes (escape codes
    /// included) and the wait status.
    pub fn finish_raw(mut self) -> (Vec<u8>, WaitStatus) {
        // ⚠️ `as_bytes`, not `before`: the `Eof` needle matches the whole
        // remaining buffer, so `before()` is always empty here.
        let rest = self.session.expect(Eof).expect("evnx did not exit");
        let rest = rest.as_bytes().to_vec();
        let status = self.session.get_process().wait().expect("wait for evnx");
        (rest, status)
    }
}
