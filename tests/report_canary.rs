//! The reports of the five commands built on `core::output` (init, add,
//! validate, sync, scan) show no part of a value (WP-17, RUST-8).
//!
//! A canary is generated at run time (random, never a literal in the
//! repository), planted under secret-looking names in `.env`, and given to
//! `add custom` as a value. Each command runs in every output form it has:
//! text, `-q`, `--json`/`--format json`, SARIF and GitHub annotations, and
//! `--exit-only`. Neither stream may hold the canary or four characters of
//! either end of it. The unit tests next to each report type check the same
//! for the report struct's `Debug` and JSON; this one checks what reaches the
//! terminal.

mod support;

use support::{Output, Project, run};

fn canary() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes
        .iter()
        .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
        .collect()
}

fn project(canary: &str) -> Project {
    Project::new()
        .env(&format!(
            "SECRET_KEY={canary}\nDB_PASSWORD=changeme\nAPI_TOKEN={canary}\n\
             DATABASE_URL=postgres://app:{canary}@db.internal:5432/app\n"
        ))
        .example("SECRET_KEY=\nDB_PASSWORD=\nAPI_TOKEN=\nDATABASE_URL=\nNEW_SECRET_KEY=\n")
}

#[track_caller]
fn assert_clean(canary: &str, name: &str, out: &Output) {
    assert!(
        matches!(out.code, Some(0) | Some(1)),
        "{name}: did not run (exit {:?})\n{}",
        out.code,
        out.all().replace(canary, "<canary>")
    );
    let text = out.all();
    for part in [canary, &canary[..4], &canary[canary.len() - 4..]] {
        assert!(
            !text.contains(part),
            "{name}: shows {} characters of the canary",
            part.len()
        );
    }
}

#[test]
fn every_report_form_of_the_five_commands_is_clean() {
    let canary = canary();
    let add_value = format!("NEW_SECRET_KEY={canary}");
    let cases: Vec<(&str, Vec<&str>, bool)> = vec![
        // (name, args, prints something)
        ("validate", vec!["validate"], true),
        ("validate -q", vec!["-q", "validate"], true),
        (
            "validate --fix --dry-run",
            vec!["validate", "--fix", "--dry-run"],
            true,
        ),
        (
            "validate --fix --rotate-weak-secrets",
            vec!["validate", "--fix", "--rotate-weak-secrets"],
            true,
        ),
        (
            "validate --format json",
            vec!["validate", "--format", "json"],
            true,
        ),
        (
            "validate --format github",
            vec!["validate", "--format", "github"],
            true,
        ),
        ("sync --dry-run", vec!["sync", "--dry-run"], true),
        ("sync --check", vec!["sync", "--check"], true),
        (
            "sync --format json",
            vec!["sync", "--format", "json", "--dry-run"],
            true,
        ),
        (
            "sync reverse --dry-run",
            vec!["sync", "--direction", "reverse", "--dry-run"],
            true,
        ),
        ("scan", vec!["scan", "."], true),
        ("scan -q", vec!["-q", "scan", "."], false),
        (
            "scan --format json",
            vec!["scan", ".", "--format", "json"],
            true,
        ),
        (
            "scan --format sarif",
            vec!["scan", ".", "--format", "sarif"],
            true,
        ),
        (
            "scan --format github",
            vec!["scan", ".", "--format", "github"],
            true,
        ),
        ("scan --exit-only", vec!["scan", ".", "--exit-only"], false),
        (
            "add custom --dry-run",
            vec!["add", "custom", &add_value, "--dry-run"],
            true,
        ),
        (
            "add custom --json",
            vec!["add", "custom", &add_value, "--json", "--dry-run"],
            true,
        ),
        (
            "add custom --yes",
            vec!["add", "custom", &add_value, "--yes"],
            true,
        ),
        (
            "add custom --yes --json",
            vec!["add", "custom", &add_value, "--yes", "--json"],
            true,
        ),
        ("init --dry-run", vec!["init", "--dry-run", "--yes"], true),
        (
            "init --json",
            vec!["init", "--json", "--dry-run", "--yes"],
            true,
        ),
    ];
    for (name, args, prints) in cases {
        let p = project(&canary);
        let out = run(p.path(), &args);
        assert_clean(&canary, name, &out);
        if prints {
            assert!(
                !out.all().trim().is_empty(),
                "{name}: printed nothing, so there is nothing to check"
            );
        }
    }
}
