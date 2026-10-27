//! `evnx diff --format patch --interactive` says what it did (I-073, RM-40).
//!
//! It printed "Applied changes" and "apply with: patch -p1 < changes.patch"
//! while writing nothing and creating no `changes.patch`. It still writes
//! nothing (writing is RM-40, v0.12.0), and now says so. Without a terminal it
//! refuses with the way out, as every other prompt does.
#![cfg(unix)]

mod support;

use support::pty::{Pty, ENTER};
use support::{run, Project};

fn project() -> Project {
    Project::new().example("A=\nB=\n").env("A=1\n")
}

#[test]
fn t_i073_interactive_says_it_wrote_nothing() {
    let p = project();
    let before = p.read(".env");
    let mut s = Pty::spawn(p.path(), &["diff", "--format", "patch", "--interactive"]);
    s.wait_for("Add B=");
    s.press(&["y", ENTER]);
    let (out, code) = s.finish();
    // 1, as without --interactive: nothing was written, so `.env` still differs.
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Chosen (nothing was written)"), "{out}");
    assert!(out.contains("evnx did not change .env"), "{out}");
    assert!(!out.contains("Applied changes"), "{out}");
    assert!(!out.contains("changes.patch"), "{out}");
    assert_eq!(p.read(".env"), before);
    assert!(!p.path().join("changes.patch").exists());
}

#[test]
fn t_i073_interactive_without_a_terminal_names_the_way_out() {
    let p = project();
    let out = run(p.path(), &["diff", "--format", "patch", "--interactive"]);
    out.assert_code(2);
    assert!(
        out.stderr.contains("evnx diff --format patch"),
        "{}",
        out.stderr
    );
}
