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
    // Enter answers, since 0.10.9: a bare `y` used to answer this question and
    // let the Enter after it answer the next one (I-265).
    s.press(&["y", ENTER]);
    // The warning and audit line print before the next prompt.
    let before = s.wait_for("Add a comment explaining these variables?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(&["n", ENTER]);
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

/// T-SYNC-43, I-126 (O-4): the comment describes the variables, so it goes
/// above them — it was appended after the block, in a second write.
#[test]
fn t_i126_comment_is_written_before_its_variables() {
    let p = Project::new().env("A=1\nNEW_ONE=1\n").example("A=\n");
    let mut s = Pty::spawn(p.path(), &["sync"]);
    s.wait_for("Add these to .env.example?");
    s.press(&[ENTER]);
    s.wait_for("Add a comment explaining these variables?");
    s.wait_for("[Y/n]");
    s.wait_for_echo_off();
    s.press(&["y", ENTER]);
    s.wait_for("Comment");
    s.type_line("Team vars");
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    let example = p.read(".env.example");
    let comment = example.find("# Team vars").expect(&example);
    let var = example.find("NEW_ONE=").expect(&example);
    assert!(comment < var, "comment above its variables:\n{example}");
}

/// DEC-S9, DEC-S18: with `CI` set, sync does not ask even at a terminal.
#[test]
fn t_i124_ci_set_skips_the_prompt_at_a_terminal() {
    let p = Project::new().env("A=1\nNEW_ONE=1\n").example("A=\n");
    let s = Pty::spawn_with(p.path(), &["sync"], &[("CI", "1")]);
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("Add these to"), "no prompt:\n{out}");
    assert!(p.read(".env.example").contains("NEW_ONE=YOUR_VALUE_HERE"));
}

/// I-277: the new-template "actual values" choice rewrote `.env` line by line,
/// so a refused key's continuation lines (a PEM block) and a line written
/// `export<TAB>KEY=…` reached `.env.example` — while the run said both were
/// kept out. The template is now built from the parsed values.
#[test]
fn t_i277_new_template_actual_values_never_copies_a_refused_value() {
    let env = "APP_NAME=demo\n\
               PRIVATE_KEY=\"-----BEGIN RSA PRIVATE KEY-----\n\
               MIIEpAIBAAKCAQEA7ZQ9uX0Fh3sH2LkP8mRk\n\
               -----END RSA PRIVATE KEY-----\"\n\
               export\tSTRIPE_KEY=sk_live_51H1234567890abcdefghijk\n";
    let p = Project::new().env(env);
    let mut s = Pty::spawn(p.path(), &["sync"]);
    s.wait_for("Create .env.example with");
    s.press(&[DOWN, ENTER]);
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");

    let example = p.read(".env.example");
    for leaked in ["MIIEpAIBAAKCAQEA", "END RSA PRIVATE KEY", "sk_live_51H"] {
        assert!(!example.contains(leaked), "{leaked} written:\n{example}");
    }
    assert!(example.lines().any(|l| l == "APP_NAME=demo"), "{example}");
    assert!(example.contains("PRIVATE_KEY="), "{example}");
    assert!(example.contains("STRIPE_KEY="), "{example}");
    // I-278: the advice is a command that does what it says.
    assert!(
        !out.contains("re-create it with `evnx sync --force`"),
        "{out}"
    );
    assert!(out.contains("delete .env.example"), "{out}");
}

/// I-280: dialoguer draws on stderr. With stdin a terminal and stderr sent to
/// a file, the prompt failed "IO error: not a terminal" (I-124's message).
#[test]
fn t_i280_no_prompt_when_stderr_is_not_a_terminal() {
    let p = Project::new().env("A=1\nNEW_ONE=1\n").example("A=\n");
    let s = Pty::spawn_shell(p.path(), "\"$EVNX\" sync 2>err.txt; echo \"rc=$?\"");
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("rc=0"), "{out}");
    let err = p.read("err.txt");
    assert!(!err.contains("not a terminal"), "{err}");
    assert!(p.read(".env.example").contains("NEW_ONE=YOUR_VALUE_HERE"));
}

/// Reverse, "Let me choose individually", nothing chosen: nothing is
/// written — it appended an empty "# Added by evnx sync" heading.
#[test]
fn t_i283_reverse_empty_selection_writes_nothing() {
    let p = Project::new().env("A=1\n").example("A=\nB=\n");
    let before = p.read(".env");
    let mut s = Pty::spawn(p.path(), &["sync", "--direction", "reverse"]);
    s.wait_for("Add to .env?");
    s.press(&[DOWN, DOWN, ENTER]);
    s.wait_for("Select variables to add");
    s.press(&[ENTER]);
    let (out, code) = s.finish();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No changes made"), "{out}");
    assert_eq!(p.read(".env"), before);
}
