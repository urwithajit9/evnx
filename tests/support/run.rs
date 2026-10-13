//! Run the built binary and read what it did.

use std::path::Path;
use std::process::{Command, Stdio};

/// What one evnx run produced, with colour removed.
#[derive(Debug, Clone)]
pub struct Output {
    /// `None` when the process was killed by a signal (e.g. SIGPIPE).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// stdout then stderr, the way a terminal shows them when nothing is
    /// interleaved. Good for "does the user see X anywhere".
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    /// Assert the exit code, printing both streams if it differs.
    #[track_caller]
    pub fn assert_code(&self, expected: i32) -> &Output {
        assert_eq!(
            self.code,
            Some(expected),
            "exit code\n--- stdout\n{}\n--- stderr\n{}",
            self.stdout,
            self.stderr
        );
        self
    }

    #[track_caller]
    pub fn assert_contains(&self, needle: &str) -> &Output {
        assert!(
            self.all().contains(needle),
            "expected {needle:?} in output\n--- stdout\n{}\n--- stderr\n{}",
            self.stdout,
            self.stderr
        );
        self
    }

    #[track_caller]
    pub fn assert_not_contains(&self, needle: &str) -> &Output {
        assert!(
            !self.all().contains(needle),
            "did not expect {needle:?} in output\n--- stdout\n{}\n--- stderr\n{}",
            self.stdout,
            self.stderr
        );
        self
    }
}

/// `evnx <args>` in `dir`, as a script would run it: stdin is not a terminal
/// (it is empty), `NO_COLOR=1`, and any ANSI that still gets through is
/// stripped.
pub fn run(dir: &Path, args: &[&str]) -> Output {
    run_with(dir, args, &[], None)
}

/// [`run`] with extra environment variables and, optionally, bytes on stdin.
///
/// ⚠️ Piping bytes to stdin does **not** make it a terminal. Prompts built on
/// dialoguer refuse it (`not a terminal`); use [`super::pty`] for those.
pub fn run_with(dir: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Output {
    let mut cmd = Command::new(super::evnx_bin());
    cmd.current_dir(dir)
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn evnx");
    if let Some(bytes) = stdin {
        use std::io::Write;
        // A closed pipe here is the child's business (it may not read stdin).
        let _ = child.stdin.take().expect("stdin piped").write_all(bytes);
    }
    let out = child.wait_with_output().expect("wait for evnx");
    Output {
        code: out.status.code(),
        stdout: strip_ansi(&String::from_utf8_lossy(&out.stdout)),
        stderr: strip_ansi(&String::from_utf8_lossy(&out.stderr)),
    }
}

/// Remove ANSI escape sequences (CSI `ESC [ … letter`, OSC `ESC ] … BEL/ST`,
/// and two-byte `ESC x`). `NO_COLOR` covers colour; this covers the cursor
/// codes prompts emit and any colour a library prints regardless.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // Parameters and intermediates, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: up to BEL or ST (ESC \).
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}
