//! The CLI runs on a thread with an 8 MB stack (RUST-16). Whatever that thread
//! does, the process must behave as it did on the main one: the exit code comes
//! out, and the commands that only print (`--version`, `--help`) work from an
//! empty directory with nothing configured. On Windows this is the check that a
//! debug build starts at all; CI also runs it there with no linker flag.

mod support;

use support::run;

#[test]
fn version_and_help_exit_zero() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["--version"][..],
        &["--help"][..],
        &["scan", "--help"][..],
        &["validate", "--help"][..],
    ] {
        let out = run(dir.path(), args);
        out.assert_code(0);
    }
}

#[test]
fn a_usage_error_still_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["--no-such-flag"]).assert_code(2);
}

/// A panic on the worker thread is re-raised on the main one: still exit 101,
/// still reported as `thread 'main'`, and printed once. `/dev/full` makes
/// `println!` fail with something other than a closed pipe (which exits 141).
#[cfg(target_os = "linux")]
#[test]
fn a_panic_still_exits_101() {
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let out = Command::new(support::evnx_bin())
        .current_dir(dir.path())
        .arg("scan")
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("panicked at").count(), 1, "{stderr}");
    assert!(stderr.contains("thread 'main'"), "{stderr}");
}
