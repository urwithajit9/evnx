//! Process-wide terminal behaviour, installed once at start-up from `main`.
//!
//! The first two pieces of the planned `core::term` (C-02): what evnx does when
//! its stdout goes away, and what it leaves on the terminal when interrupted.
//! Both apply to every command.

/// Exit status for "the reader of stdout went away": 128 + SIGPIPE, the value a
/// shell reports for `yes | head -1`, so scripts and CI read it as they would
/// for any other well-behaved tool.
pub const BROKEN_PIPE_EXIT: i32 = 141;

/// Exit quietly when stdout or stderr is a closed pipe.
///
/// `println!` panics when its write fails, so `evnx … | head -n 3` used to end
/// with `thread 'main' panicked … failed printing to stdout: Broken pipe` and
/// exit 101 (I-008, I-185). The reader has already got what it wanted; there is
/// nothing to report, so this exits with [`BROKEN_PIPE_EXIT`] and no message.
///
/// ⚠️ A panic hook scoped to this one failure, **not** `signal(SIGPIPE,
/// SIG_DFL)`. SIGPIPE is also raised by a write to a network socket the server
/// has closed, and resetting it would turn such a write, in any dependency that
/// does not guard its sockets, into a silent kill instead of an error. Every
/// other panic, and every other print failure (a full disk, say), goes to the
/// default hook unchanged.
pub fn exit_quietly_on_broken_pipe() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        if is_broken_pipe_panic(message) {
            std::process::exit(BROKEN_PIPE_EXIT);
        }
        default_hook(info);
    }));
}

/// Whether a panic message is std's "failed printing to stdout/stderr" for a
/// closed pipe: `Broken pipe (os error 32)` on Unix, `os error 232` ("The pipe
/// is being closed") on Windows.
fn is_broken_pipe_panic(message: &str) -> bool {
    message.starts_with("failed printing to std")
        && (message.contains("Broken pipe") || message.contains("os error 232"))
}

/// Give the cursor back if the user presses Ctrl-C at a prompt.
///
/// dialoguer hides the cursor while a menu is open and shows it again when the
/// menu returns. Ctrl-C ends the process before that, so the shell was left with
/// no visible cursor (N-07). This handler writes the "show cursor" sequence and
/// then re-raises SIGINT with the default action, so the process still ends the
/// way an interrupted process always has (shell status 130, nothing written).
///
/// Installed only when stderr, where dialoguer draws, is a terminal: in CI or a
/// pipe there is no cursor to restore, and the escape sequence would only add
/// noise to a log. Unix only; the Windows console restores its own cursor.
#[cfg(unix)]
pub fn restore_cursor_on_interrupt() {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }

    extern "C" fn on_interrupt(signal: libc::c_int) {
        const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
        // SAFETY: `write`, `signal` and `raise` are async-signal-safe, and
        // nothing else happens in this handler.
        unsafe {
            libc::write(2, SHOW_CURSOR.as_ptr().cast(), SHOW_CURSOR.len());
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }

    // SAFETY: installs a handler that only calls async-signal-safe functions.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
    }
}

/// No-op off Unix.
#[cfg(not(unix))]
pub fn restore_cursor_on_interrupt() {}

/// Whether evnx may ask the person a question: stdin is a terminal.
///
/// One definition for every command (C-02). Ten call sites each asked
/// `std::io::stdin().is_terminal()` themselves; a rule that changes (a
/// `--no-input` flag, `CI=true`) now changes in one place.
///
/// `std::io::IsTerminal`, stable since Rust 1.70, rather than the `atty` crate,
/// which is unmaintained and carries RUSTSEC-2021-0145.
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_closed_pipe_is_treated_as_quiet() {
        assert!(is_broken_pipe_panic(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_broken_pipe_panic(
            "failed printing to stderr: Broken pipe (os error 32)"
        ));
        assert!(is_broken_pipe_panic(
            "failed printing to stdout: The pipe is being closed. (os error 232)"
        ));
        // A full disk is a real error and must still be reported.
        assert!(!is_broken_pipe_panic(
            "failed printing to stdout: No space left on device (os error 28)"
        ));
        assert!(!is_broken_pipe_panic("index out of bounds: Broken pipe"));
    }
}
