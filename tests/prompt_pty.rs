//! Yes/no questions show the key that was pressed, and answer one question
//! each (I-264, I-265).
//!
//! I-264 (owner report, 2026-10-19, `evnx init`): at "Generate .env files with
//! these variables? [Y/n]" pressing `n` printed nothing — dialoguer's
//! `Confirm` in Enter-to-answer mode hides the cursor, never echoes the key,
//! and only re-renders the hint with the pending answer as its default
//! (`[Y/n]` ↔ `[y/N]`; under a No default, `n` changed nothing at all). The
//! answer appeared only after Enter, so a keypress looked ignored.
//!
//! I-265: twelve other questions were plain `Confirm::new()`, answered on the
//! first key, so the Enter a person types after `y` answered the *next*
//! question too — the N-09 defect `utils::prompt::yes_no` had fixed for init
//! and add, still live in sync, restore and the cloud commands.
#![cfg(unix)]

mod support;

use std::time::Duration;
use support::pty::{Pty, DOWN, ENTER};
use support::Project;

const BACKSPACE: &str = "\x7f";

/// `init` up to "Generate .env files with these variables? [Y/n]".
fn at_generate_question(p: &Project) -> Pty {
    let mut run = Pty::spawn(p.path(), &["init"]);
    run.wait_for("How do you want to start?");
    run.press(&[DOWN, ENTER]);
    run.wait_for("Choose a stack blueprint");
    run.press(&[ENTER]); // T3 Turbo
    run.wait_for("Generate .env files with these variables?");
    run.wait_for("[Y/n]");
    run.wait_for_echo_off();
    run
}

/// The reported case: the pressed key is on screen before Enter.
#[test]
fn t_i264_the_answer_shows_before_enter() {
    let p = Project::new();
    let mut run = at_generate_question(&p);
    run.press(&["n"]);
    assert!(
        run.try_wait_for("[Y/n] n", Duration::from_secs(3))
            .is_some(),
        "`n` was not shown after the hint before Enter"
    );
    run.press(&[ENTER]);
    run.wait_for("Generate .env files with these variables? no");
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(!p.exists(".env.example"), "answered no: nothing written");
}

/// `y` shows too, and Backspace takes it back: Enter then gives the default.
#[test]
fn t_i264_backspace_takes_the_answer_back() {
    let p = Project::new();
    let mut run = at_generate_question(&p);
    run.press(&["n"]);
    assert!(
        run.try_wait_for("[Y/n] n", Duration::from_secs(3))
            .is_some(),
        "`n` was not shown"
    );
    run.press(&[BACKSPACE]);
    run.press(&["y"]);
    assert!(
        run.try_wait_for("[Y/n] y", Duration::from_secs(3))
            .is_some(),
        "`y` was not shown"
    );
    run.press(&[BACKSPACE, ENTER]);
    // Enter with nothing typed takes the default, Yes.
    run.wait_for("Generate .env files with these variables? yes");
    let (rest, code) = run.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(p.exists(".env.example"), "the default (yes) was taken");
}

/// I-265: `sync`'s "Are you absolutely sure…?" answered on `y` alone, and the
/// Enter after it answered "Add a comment explaining these variables?" with
/// its default (yes). Now each question takes its own Enter.
#[test]
fn t_i265_enter_after_y_answers_only_one_question() {
    let p = Project::new()
        .env("APP_NAME=demo\nLOG_LEVEL=info\n")
        .example("APP_NAME=\n");
    let mut s = Pty::spawn(p.path(), &["sync"]);
    s.wait_for("Add these to .env.example?");
    s.press(&[DOWN, ENTER]); // "Yes, add with actual values"
    s.wait_for("Are you absolutely sure");
    s.wait_for("[y/N]");
    s.wait_for_echo_off();
    s.press(&["y", ENTER]);
    s.wait_for("Add a comment explaining these variables?");
    assert!(
        s.try_wait_for("variables? yes", Duration::from_secs(2))
            .is_none(),
        "the Enter after `y` answered the next question as well"
    );
    s.wait_for_echo_off();
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(p.read(".env.example").contains("LOG_LEVEL=info"));
    assert!(
        !p.read(".env.example").contains("Additional configuration"),
        "no comment was added"
    );
}
