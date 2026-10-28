//! `evnx diff --format patch --interactive` writes what was chosen (RM-40).
//!
//! It printed "Applied changes" and "apply with: patch -p1 < changes.patch"
//! while writing nothing (I-073). It now writes the chosen changes and keeps
//! the previous file as `.env.bak`. Without a terminal it refuses with the way
//! out, as every other prompt does.
#![cfg(unix)]

mod support;

use support::pty::{Pty, ENTER};
use support::{run, Project};

fn project() -> Project {
    Project::new().example("A=\nB=\n").env("A=1\n")
}

#[test]
fn rm40_interactive_writes_the_chosen_change_and_keeps_a_bak() {
    let p = project();
    let before = p.read(".env");
    let mut s = Pty::spawn(p.path(), &["diff", "--format", "patch", "--interactive"]);
    s.wait_for("Add B=");
    s.press(&["y", ENTER]);
    let (out, code) = s.finish();
    // 1: the verdict is the comparison that was made, before the write.
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("wrote 1 change to .env"), "{out}");
    assert!(!out.contains("Applied changes"), "{out}");
    assert_eq!(p.read(".env"), "A=1\nB=\n");
    assert_eq!(p.read(".env.bak"), before);
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
