//! `evnx sync`'s interactive "actual values" choices (I-113, DEC-S1).
//!
//! `allow_actual` in a template config was refused when a key held a
//! credential, but the two interactive menu choices that write **actual
//! values** into `.env.example` had no check at all: one `y` committed
//! `sk_live_…`. Now both run the same detectors as `allow_actual` (the ones
//! `evnx scan` uses): a credential key is named and gets a placeholder, the
//! other keys keep their real values, and an audit line goes to stderr.
#![cfg(unix)]

mod support;

use support::pty::{Pty, DOWN, ENTER};
use support::Project;

const TOKEN: &str = "sk_live_51H1234567890abcdefghijk";
const DB_URL: &str = "postgres://admin:s3cr3t@db.internal:5432/app";

fn env() -> String {
    format!("APP_NAME=demo\nLOG_LEVEL=info\nSTRIPE_KEY={TOKEN}\nDATABASE_URL={DB_URL}\n")
}

/// Existing template, menu choice 2 "Yes, add with actual values".
#[test]
fn t_i113_existing_template_actual_values_refuses_credentials() {
    let p = Project::new().env(&env()).example("APP_NAME=\n");
    let mut s = Pty::spawn(p.path(), &["sync"]);
    s.wait_for("Add these to .env.example?");
    s.press(&[DOWN, ENTER]);
    // ⚠️ "[y/N]", not the question: dialoguer prints the question, then
    // switches the terminal to raw mode (which discards keys already typed),
    // then prints the hint. A key sent on the question alone was lost.
    s.wait_for("Are you absolutely sure");
    s.wait_for("[y/N]");
    s.wait_for_echo_off();
    s.press(&["y"]);
    // The warning and audit line print before the next prompt.
    let before = s.wait_for("Add a comment explaining these variables?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(&["n"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    let out = format!("{before}{rest}");

    let example = p.read(".env.example");
    assert!(
        example.contains("LOG_LEVEL=info"),
        "non-secret keeps its value:\n{example}"
    );
    for secret in [TOKEN, "s3cr3t"] {
        assert!(!example.contains(secret), "{secret} written:\n{example}");
    }
    assert!(example.contains("STRIPE_KEY="), "{example}");
    assert!(example.contains("DATABASE_URL="), "{example}");
    assert!(
        out.contains("STRIPE_KEY"),
        "the refused key is named:\n{out}"
    );
    assert!(
        out.contains("[AUDIT]"),
        "audit line without --verbose:\n{out}"
    );
}

/// No template yet, menu choice 2 "Actual values copied from .env".
#[test]
fn t_i113_new_template_actual_values_refuses_credentials() {
    let p = Project::new().env(&env());
    let mut s = Pty::spawn(p.path(), &["sync"]);
    s.wait_for("Create .env.example with");
    s.press(&[DOWN, ENTER]);
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");

    let example = p.read(".env.example");
    assert!(example.contains("APP_NAME=demo"), "{example}");
    for secret in [TOKEN, "s3cr3t"] {
        assert!(!example.contains(secret), "{secret} written:\n{example}");
    }
    assert!(
        example.contains("DATABASE_URL="),
        "the key stays, as a placeholder:\n{example}"
    );
    assert!(
        out.contains("DATABASE_URL"),
        "the refused key is named:\n{out}"
    );
    assert!(out.contains("[AUDIT]"), "{out}");
}
