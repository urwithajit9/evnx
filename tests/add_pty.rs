//! `evnx add`'s prompts, driven through a real pseudo-terminal.
//!
//! dialoguer refuses a stdin that is not a terminal, so the old
//! `add_custom_interactive` (keys piped to stdin, `#[ignore]`d "requires TTY")
//! could not pass even when run by hand (T-002). These press the keys a person
//! would, through `tests/support/pty.rs`.
//!
//! On 0.10.1 every yes/no prompt in `add` answers on the first key (`y` or
//! `n`, no Enter). AD-A moves them to init's `yes_no()` (answered on Enter,
//! N-09), and the keys below change in that PR.
//!
//! `known_*` tests pin defects; the package named in each flips it (see
//! `add_integration.rs`).
#![cfg(unix)]

mod support;

use support::pty::{Pty, DOWN, ENTER, ESC};
use support::snapshot::{check_golden, normalise};
use support::{run, Project};

/// A project with one existing variable in each file.
fn project() -> Project {
    Project::new()
        .example("VITE_ENV=development\n")
        .env("VITE_ENV=development\n")
        .file(".gitignore", ".env\n")
}

/// `add custom` up to the first question after the value.
fn custom_name_and_value(p: &Project, name: &str, value: &str) -> Pty {
    let mut s = Pty::spawn(p.path(), &["add", "custom"]);
    s.wait_for("Variable name");
    s.type_line(name);
    s.wait_for("Example/placeholder value");
    s.type_line(value);
    s.wait_for("Add a description comment?");
    s
}

// ─────────────────────────────────────────────────────────────
// Locked behaviour
// ─────────────────────────────────────────────────────────────

/// T-002 / T-ADD-01: replaces the ignored `add_custom_interactive`. One
/// variable with no extras, written to both files under its own section.
#[test]
fn custom_writes_the_variable_and_its_section() {
    let p = project();
    let mut s = custom_name_and_value(&p, "MY_API_KEY", "sk_test_xxx");
    s.press(&["n"]);
    s.wait_for("Assign to a category?");
    s.press(&["n"]);
    s.wait_for("Is this variable required?");
    s.press(&["n"]);
    s.wait_for("Add another variable?");
    s.press(&["n"]);
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");

    for f in [".env.example", ".env"] {
        let text = p.read(f);
        assert!(text.contains("MY_API_KEY=sk_test_xxx"), "{f}:\n{text}");
        assert!(text.contains("# ── Custom Variables ──"), "{f}:\n{text}");
    }
    check_golden(
        "add/custom.env.example",
        &normalise(&p.read(".env.example"), &[p.path()]),
    );
}

/// T-ADD-35: the preview names the count, and `n` writes nothing.
#[test]
fn answering_no_writes_nothing() {
    let p = project();
    let before = (p.read(".env.example"), p.read(".env"));
    let mut s = Pty::spawn(p.path(), &["add", "service", "redis"]);
    s.wait_for("Preview:");
    s.wait_for("Add these 5 variables to .env.example?");
    s.press(&["n"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Aborted."), "{rest}");
    assert_eq!((p.read(".env.example"), p.read(".env")), before);
}

/// T-ADD-31: answering yes at the prompt writes exactly what `--yes` writes.
#[test]
fn yes_at_the_prompt_writes_what_dash_dash_yes_writes() {
    let asked = project();
    let mut s = Pty::spawn(asked.path(), &["add", "service", "redis"]);
    s.wait_for("Add these 5 variables to .env.example?");
    s.press(&["y"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");

    let flagged = project();
    run(flagged.path(), &["add", "service", "redis", "--yes"]).assert_code(0);

    for f in [".env.example", ".env"] {
        assert_eq!(
            normalise(&asked.read(f), &[asked.path()]),
            normalise(&flagged.read(f), &[flagged.path()]),
            "{f}"
        );
    }
}

// ─────────────────────────────────────────────────────────────
// KNOWN IN 0.10.1
// ─────────────────────────────────────────────────────────────

/// N-09 (AD-A): `y⏎` at "Is this variable required?" also answers "Add
/// another variable?" (default yes): the `y` answers the first, the Enter the
/// second, and the user never sees the second question asked.
#[test]
fn known_n09_y_enter_answers_two_questions() {
    let p = project();
    let mut s = custom_name_and_value(&p, "N09_VAR", "v");
    s.press(&["n"]);
    s.wait_for("Assign to a category?");
    s.press(&["n"]);
    s.wait_for("Is this variable required?");
    s.press(&["y", ENTER]);
    let shown = s.wait_for("Variable name");
    assert!(shown.contains("Add another variable? yes"), "{shown}");
    s.press(&[ENTER]); // empty name: finish
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
}

/// I-016 (AD-S) and I-022 plurals (AD-A): `add custom` accepts a name that is
/// already there, and talks about "1 variables".
#[test]
fn known_i016f_i022_custom_re_adds_an_existing_name_in_the_plural() {
    let p = project();
    let mut s = custom_name_and_value(&p, "VITE_ENV", "your_value");
    s.press(&["n"]);
    s.wait_for("Assign to a category?");
    s.press(&["n"]);
    s.wait_for("Is this variable required?");
    s.press(&["n"]);
    s.wait_for("Add another variable?");
    s.press(&["n"]);
    let shown = s.wait_for("Append these variables to .env.example?");
    assert!(shown.contains("1 variables to add"), "{shown}");
    s.press(&["y"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Added 1 custom variables"), "{rest}");
    assert_eq!(p.count_lines(".env.example", "VITE_ENV="), 2);
    assert_eq!(p.count_lines(".env", "VITE_ENV="), 2);
}

/// N-12 (AD-A): "Next steps" is printed after "Aborted.".
#[test]
fn known_n12_next_steps_after_an_abort() {
    let p = project();
    let mut s = Pty::spawn(p.path(), &["add", "blueprint", "mern_v2"]);
    s.wait_for("Add these 18 variables to .env.example?");
    s.press(&["n"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    let aborted = rest.find("Aborted.").expect("aborted");
    let next = rest.find("Next steps").expect("next steps");
    assert!(aborted < next, "{rest}");
}

/// N-13 (AD-A): Esc at a confirmation is ignored; the question keeps waiting
/// and takes a later key. (init cancels on Esc since 0.10.1.)
///
/// ⚠️ The key after Esc may be swallowed. The `console` crate, having read an
/// ESC byte, takes the next byte at once if one is already waiting and reads
/// the pair as one unknown escape sequence. Prompt text on screen does not
/// mean dialoguer is already reading, so on a slow runner Esc and the `n`
/// after it can both be waiting when it starts: `ESC n` is dropped and the
/// question never returns (CI, 2026-10-14; same family as #156). Whichever
/// way the Esc is read, it does not end the question, so this presses `n`
/// until the question takes it.
#[test]
fn known_n13_escape_is_ignored_at_the_confirmation() {
    use std::time::Duration;
    let p = project();
    let mut s = Pty::spawn(p.path(), &["add", "service", "redis"]);
    s.wait_for("Add these 5 variables to .env.example?");
    s.press(&[ESC]);
    let mut answered = None;
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(300));
        s.press(&["n"]);
        answered = s.try_wait_for("? no", Duration::from_secs(2));
        if answered.is_some() {
            break;
        }
    }
    let shown = answered.expect("the question never took an answer after Esc");
    assert!(
        !shown.contains("Nothing was written"),
        "Esc ended the prompt: {shown}"
    );
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Aborted."), "{rest}");
    assert!(!rest.contains("Nothing was written"), "{rest}");
}

/// `add custom` with a category and no description, to the end.
/// Returns what was printed between the category list and the next question.
fn custom_with_category_only(p: &Project) -> String {
    let mut s = custom_name_and_value(p, "CAT_ONLY", "v1");
    s.press(&["n"]); // no description
    s.wait_for("Assign to a category?");
    s.press(&["y"]);
    s.wait_for("Other");
    s.press(&[DOWN, ENTER]);
    let picked = s.wait_for("Is this variable required?");
    s.press(&["n"]);
    s.wait_for("Add another variable?");
    s.press(&["n"]);
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y"]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    picked
}

/// N-17 (high; AD-B): a category with no description is written on the
/// assignment's own line, which comments the variable out. The run still says
/// it added it.
#[test]
fn known_n17_a_category_comments_the_variable_out() {
    let p = project();
    custom_with_category_only(&p);
    let example = p.read(".env.example");
    assert!(example.contains("# [Application] CAT_ONLY=v1"), "{example}");
    assert_eq!(p.count_lines(".env.example", "CAT_ONLY="), 0);
    assert_eq!(p.count_lines(".env", "CAT_ONLY="), 0);
}

/// N-18 (AD-B): the category list starts with nothing selected, so the first
/// Down lands on the first item and Enter alone selects nothing.
#[test]
fn known_n18_the_category_list_has_no_default() {
    let p = project();
    let picked = custom_with_category_only(&p);
    assert!(picked.contains("Select category: Application"), "{picked}");
}
