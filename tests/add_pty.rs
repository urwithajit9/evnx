//! `evnx add`'s prompts, driven through a real pseudo-terminal.
//!
//! dialoguer refuses a stdin that is not a terminal, so the old
//! `add_custom_interactive` (keys piped to stdin, `#[ignore]`d "requires TTY")
//! could not pass even when run by hand (T-002). These press the keys a person
//! would, through `tests/support/pty.rs`.
//!
//! Since 0.10.2 (AD-A) every yes/no prompt in `add` is answered on Enter, as
//! in `init` (N-09): `n⏎`, `y⏎`, or Enter for the default. Esc, then Enter,
//! cancels (N-13).
//!
//! `known_*` tests pin defects; the package named in each flips it (see
//! `add_integration.rs`).
#![cfg(unix)]

mod support;

use support::pty::{Pty, DOWN, ENTER};
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
    s.press(&["n", ENTER]);
    s.wait_for("Assign to a category?");
    s.press(&["n", ENTER]);
    s.wait_for("Is this variable required?");
    s.press(&["n", ENTER]);
    s.wait_for("Add another variable?");
    s.press(&["n", ENTER]);
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y", ENTER]);
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
    s.press(&["n", ENTER]);
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
    s.press(&["y", ENTER]);
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

/// N-09 (fixed in 0.10.2, AD-A; was `known_n09_*`): `y⏎` answers one
/// question. On 0.10.1 the `y` answered "Is this variable required?" and the
/// Enter silently answered "Add another variable?" with its default.
#[test]
fn y_enter_answers_exactly_one_question() {
    let p = project();
    let mut s = custom_name_and_value(&p, "N09_VAR", "v");
    s.press(&["n", ENTER]);
    s.wait_for("Assign to a category?");
    s.press(&["n", ENTER]);
    s.wait_for("Is this variable required?");
    s.press(&["y", ENTER]);
    let shown = s.wait_for("Add another variable?");
    assert!(!shown.contains("Add another variable? yes"), "{shown}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    s.press(&["n", ENTER]); // the user's own answer
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    let example = p.read(".env.example");
    assert!(example.contains("# (required)\nN09_VAR=v"), "{example}");
}

/// I-016 (AD-S): `add custom` accepts a name that is already there. (I-022,
/// the plural in "1 variables to add", was fixed in AD-A and is asserted
/// here too.)
#[test]
fn known_i016f_custom_re_adds_an_existing_name() {
    let p = project();
    let mut s = custom_name_and_value(&p, "VITE_ENV", "your_value");
    s.press(&["n", ENTER]);
    s.wait_for("Assign to a category?");
    s.press(&["n", ENTER]);
    s.wait_for("Is this variable required?");
    s.press(&["n", ENTER]);
    s.wait_for("Add another variable?");
    s.press(&["n", ENTER]);
    let shown = s.wait_for("Append these variables to .env.example?");
    assert!(shown.contains("1 variable to add"), "{shown}");
    s.press(&["y", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Added 1 custom variable"), "{rest}");
    assert!(!rest.contains("custom variables"), "{rest}");
    assert_eq!(p.count_lines(".env.example", "VITE_ENV="), 2);
    assert_eq!(p.count_lines(".env", "VITE_ENV="), 2);
}

/// N-12 (fixed in 0.10.2, AD-A; was `known_n12_*`): an abort prints no next
/// steps. On 0.10.1 "Next steps: 1. Edit .env…" followed "Aborted.".
#[test]
fn an_abort_prints_no_next_steps() {
    let p = project();
    let mut s = Pty::spawn(p.path(), &["add", "blueprint", "mern_v2"]);
    s.wait_for("Add these 18 variables to .env.example?");
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Aborted. Nothing was written."), "{rest}");
    assert!(!rest.contains("Next steps"), "{rest}");
}

/// N-13 (fixed in 0.10.2, AD-A; was `known_n13_*`): Esc cancels a
/// confirmation, writes nothing and exits 0, as in `init`. A yes/no question
/// takes its answer on Enter, so Esc selects "cancel" and Enter confirms it;
/// `cancel_yes_no` waits for the redraw between the two (see its comment for
/// the race this avoids).
#[test]
fn escape_cancels_every_confirmation() {
    for args in [
        &["add", "service", "redis"][..],
        &["add", "framework", "-l", "python", "django"],
        &["add", "blueprint", "mern_v2"],
    ] {
        let p = project();
        let before = (p.read(".env.example"), p.read(".env"));
        let mut s = Pty::spawn(p.path(), args);
        s.wait_for("to .env.example?");
        s.cancel_yes_no();
        let (rest, code) = s.finish();
        assert_eq!(code, 0, "{args:?}: {rest}");
        assert!(
            rest.contains("Aborted. Nothing was written."),
            "{args:?}: {rest}"
        );
        assert_eq!((p.read(".env.example"), p.read(".env")), before, "{args:?}");
    }
}

/// N-13: Esc at one of `add custom`'s questions cancels the whole run.
#[test]
fn escape_cancels_add_custom() {
    let p = project();
    let before = p.read(".env.example");
    let mut s = custom_name_and_value(&p, "NEVER_WRITTEN", "v");
    s.cancel_yes_no();
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    assert!(rest.contains("Aborted. Nothing was written."), "{rest}");
    assert_eq!(p.read(".env.example"), before);
}

/// N-10 (fixed in 0.10.2, AD-A; was `known_n10_*` in `add_integration.rs`):
/// the preview lists framework, service and infra in that order on every run.
/// On 0.10.1 the order came from a `HashMap` and changed between runs.
#[test]
fn preview_groups_come_in_a_fixed_order() {
    for _ in 0..3 {
        let p = Project::new();
        let mut s = Pty::spawn(p.path(), &["add", "blueprint", "t3_modern"]);
        let shown = s.wait_for("to .env.example?");
        let order: Vec<&str> = shown
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("• "))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert_eq!(order, ["framework", "service", "infra"], "{shown}");
        s.press(&["n", ENTER]);
        s.finish();
    }
}

/// `add custom` with a category and no description, to the end. `keys` pick
/// the category. Returns what was printed between the list and the next
/// question.
fn custom_with_category_only(p: &Project, keys: &[&str]) -> String {
    let mut s = custom_name_and_value(p, "CAT_ONLY", "v1");
    s.press(&["n", ENTER]); // no description
    s.wait_for("Assign to a category?");
    s.press(&["y", ENTER]);
    s.wait_for("Other");
    s.press(keys);
    let picked = s.wait_for("Is this variable required?");
    s.press(&["n", ENTER]);
    s.wait_for("Add another variable?");
    s.press(&["n", ENTER]);
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    picked
}

/// N-17 (fixed in 0.10.2, AD-B; was `known_n17_*`): a category with no
/// description goes on a comment line of its own, and the variable stays
/// active. On 0.10.1 it was written as `# [Application] CAT_ONLY=v1`: commented
/// out, while the run said "Added".
#[test]
fn a_category_never_comments_the_variable_out() {
    let p = project();
    custom_with_category_only(&p, &[ENTER]);
    let example = p.read(".env.example");
    assert!(
        example.contains("# [Application]\nCAT_ONLY=v1\n"),
        "{example}"
    );
    assert_eq!(p.count_lines(".env.example", "CAT_ONLY="), 1);
    assert_eq!(p.count_lines(".env", "CAT_ONLY="), 1);
}

/// N-18 (fixed in 0.10.2, AD-B; was `known_n18_*`): the category list starts
/// on its first item, so Enter picks it and Down moves to the second. On
/// 0.10.1 nothing was selected, Enter did nothing, and Down landed on the first.
#[test]
fn the_category_list_starts_on_the_first_item() {
    let p = project();
    let picked = custom_with_category_only(&p, &[ENTER]);
    assert!(picked.contains("Select category: Application"), "{picked}");
    let q = project();
    let picked = custom_with_category_only(&q, &[DOWN, ENTER]);
    assert!(picked.contains("Select category: Security"), "{picked}");
}
