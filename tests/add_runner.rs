//! The `evnx add` training demo, as tests (P0-3, `add` slice).
//!
//! # Why this exists
//!
//! The review package teaches `add` with a shell runner,
//! `tutorial/run-add-demo.sh`, scene by scene (A0 to A11). Nothing ran it in
//! CI, so a scene could go stale the day the product changed. These are the
//! same scenes, asserting 0.10.2's behaviour; the deck and runbook are
//! regenerated from real runs of them. Lines the runner marked `KNOWN` (a
//! defect shown as it was) are fixed in 0.10.2 and asserted fixed here, with
//! the issue id beside each.
//!
//! Keep scene names and order in step with the runner and the runbook. Each
//! scene starts in its own empty folder, as the runner's do.

mod support;

use support::{run, Project};

/// A0: four subcommands; bare `evnx add` prints help and exits 2 (DA-10).
#[test]
fn a0_bare_add_prints_help() {
    let p = Project::new();
    let out = run(p.path(), &["add"]);
    out.assert_code(2);
    for sub in ["service", "framework", "blueprint", "custom"] {
        out.assert_contains(sub);
    }
    out.assert_contains("--list").assert_contains("--dry-run");
}

/// A1: a service into an empty folder. `.env.example`, then `.env` from it,
/// `.gitignore` first (I-021, DQ-A4).
#[test]
fn a1_service_into_an_empty_folder() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("Created .env from .env.example")
        .assert_contains("Added 5 variables for Redis");
    assert_eq!(p.read(".env"), p.read(".env.example"));
    assert!(p.read(".gitignore").contains(".env*"));
}

/// A2: an existing `.env` that git would commit gets a warning, once it is
/// ignored the warning stops.
#[test]
fn a2_existing_env_and_the_gitignore_warning() {
    let p = Project::new().env("KEEP=1\n").git();
    run(p.path(), &["add", "service", "stripe", "--yes"])
        .assert_code(0)
        .assert_contains(".env is not covered by .gitignore");
    assert!(p.read(".env").starts_with("KEEP=1\n"));
    std::fs::write(p.join(".gitignore"), ".env\n").unwrap();
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_not_contains("not covered by .gitignore");
}

/// A3: frameworks. `--language` is optional and checked when given (I-017).
#[test]
fn a3_frameworks() {
    let p = Project::new();
    run(
        p.path(),
        &[
            "add",
            "framework",
            "--language",
            "python",
            "django",
            "--yes",
        ],
    )
    .assert_code(0);
    run(p.path(), &["add", "framework", "nextjs", "--yes"])
        .assert_code(0)
        .assert_contains("Added 4 variables for Next.js");
    run(
        p.path(),
        &[
            "add",
            "framework",
            "--language",
            "typescript",
            "nextjs",
            "--yes",
        ],
    )
    .assert_code(2)
    .assert_contains("Unknown language: 'typescript'");
}

/// A4: blueprints. A wrong id lists the real ones; a different value is kept.
#[test]
fn a4_blueprints() {
    let p = Project::new();
    run(p.path(), &["add", "blueprint", "fastapi_ai", "--yes"]).assert_code(0);
    run(p.path(), &["add", "blueprint", "fastapi_full", "--yes"])
        .assert_code(2)
        .assert_contains("fastapi_ai");

    let q = Project::new().example("DATABASE_URL=postgres://real@host/db\n");
    run(q.path(), &["add", "blueprint", "rust_high_perf", "--yes"])
        .assert_code(0)
        .assert_contains("DATABASE_URL (existing: \"postgres://real@host/db\"");
    assert_eq!(q.count_lines(".env.example", "DATABASE_URL="), 1);
    assert!(q
        .read(".env.example")
        .starts_with("DATABASE_URL=postgres://real@host/db\n"));
}

/// A5: adding again skips what is there, for services and blueprints (I-016).
#[test]
fn a5_adding_again_skips_what_is_there() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("Nothing to add");
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 1);
    run(p.path(), &["add", "blueprint", "fastapi_ai", "--yes"])
        .assert_code(0)
        .assert_contains("already present");
    run(p.path(), &["add", "blueprint", "fastapi_ai", "--yes"])
        .assert_code(0)
        .assert_contains("Nothing to add");
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 1);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 1);
}

/// A6: ids. Unknown and mistyped ids are refused with help; any catalogue id
/// works through `service`, and says what it is (I-001, I-018).
#[test]
fn a6_ids() {
    let p = Project::new();
    run(p.path(), &["add", "service", "nosuchsvc", "--yes"])
        .assert_code(2)
        .assert_contains("evnx init --list-components");
    run(p.path(), &["add", "service", "postgress", "--yes"])
        .assert_code(2)
        .assert_contains("Did you mean: postgresql");
    run(p.path(), &["add", "service", "vercel", "--yes"])
        .assert_code(0)
        .assert_contains("vercel is an infra component");
    run(p.path(), &["add", "service", "nextjs", "--yes"])
        .assert_code(0)
        .assert_contains("nextjs is a javascript_typescript framework");
    run(p.path(), &["add", "infra", "docker", "--yes"]).assert_code(0);
}

/// A7: with a spec, the new names are recorded once.
#[test]
fn a7_contract_writeback() {
    let p = Project::new().toml("[vars.A]\nrequired = true\n");
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("to [vars] in .evnx.toml");
    let tables = || p.count_lines(".evnx.toml", "[vars.");
    let after_first = tables();
    assert_eq!(after_first, 6, "{}", p.read(".evnx.toml"));
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert_eq!(tables(), after_first);
}

/// A8: custom variables without a terminal: as arguments (I-020, I-025).
#[test]
fn a8_custom_without_a_terminal() {
    let p = Project::new();
    run(p.path(), &["add", "custom", "--yes"])
        .assert_code(2)
        .assert_contains("evnx add custom NAME=value");
    run(p.path(), &["add", "custom", "MY_KEY=value", "--yes"])
        .assert_code(0)
        .assert_contains("Added 1 custom variable");
    // DEC-U8-1 (I-245): the value in `.env` only, a placeholder in the template.
    assert!(p.read(".env").contains("\nMY_KEY=value\n"));
    assert!(p.read(".env.example").contains("\nMY_KEY="));
    assert!(!p.read(".env.example").contains("MY_KEY=value"));
}

/// A9: `-p`, `-q`, `-v`.
#[test]
fn a9_path_quiet_verbose() {
    let p = Project::new().dir("pkg");
    run(
        p.path(),
        &["add", "--path", "packages/api", "service", "redis", "--yes"],
    )
    .assert_code(2)
    .assert_contains("packages/api");
    assert!(!p.exists("packages"));
    run(p.path(), &["add", "service", "redis", "-p", "pkg", "--yes"]).assert_code(0);
    assert!(p.exists("pkg/.env.example"));
    let quiet = run(p.path(), &["add", "-q", "service", "stripe", "--yes"]);
    quiet.assert_code(0);
    assert_eq!(quiet.stdout.trim(), "", "{}", quiet.stdout);
    run(p.path(), &["add", "-v", "service", "sentry", "--yes"])
        .assert_code(0)
        .assert_contains("[DEBUG] Appended to")
        .assert_not_contains("TODO");
}

/// A10: an agent or CI job: flags only, `--dry-run` first, then `--json`.
#[test]
fn a10_agent_style() {
    let p = Project::new();
    run(p.path(), &["add", "--list"])
        .assert_code(0)
        .assert_contains("postgresql");
    run(p.path(), &["add", "service", "postgresql", "--dry-run"])
        .assert_code(0)
        .assert_contains("would add 6 variables");
    assert!(!p.exists(".env.example"));
    let out = run(
        p.path(),
        &["add", "service", "postgresql", "--yes", "--json"],
    );
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["added"].as_array().unwrap().len(), 6);
    run(p.path(), &["add", "service", "nope", "--yes"]).assert_code(2);
}

/// A11 (interactive): `add custom` through a terminal, the runbook's key
/// script: name, value, "More options?" yes, a description, no category,
/// required, then an empty name and yes.
#[cfg(unix)]
#[test]
fn a11_custom_interactive() {
    use support::pty::{Pty, ENTER};
    let p = Project::new().example("APP=1\n").env("APP=1\n");
    let mut s = Pty::spawn(p.path(), &["add", "custom"]);
    s.wait_for("Variable name");
    s.type_line("MY_API_KEY");
    s.wait_for(".env only; .env.example gets");
    s.type_line("sk_test_xxx");
    s.wait_for("More options?");
    s.press(&["y", ENTER]);
    s.wait_for("Description (Enter for none)");
    s.type_line("Payment key");
    s.wait_for("Category");
    s.press(&[ENTER]); // None
    s.wait_for("Is this variable required?");
    s.press(&["y", ENTER]);
    s.wait_for("Variable name");
    s.press(&[ENTER]);
    s.wait_for("Append these variables to .env.example?");
    s.press(&["y", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 0, "{rest}");
    // DEC-U8-1 (I-245): the value in `.env` only, a placeholder in the template.
    assert!(
        p.read(".env")
            .contains("# Payment key (required)\nMY_API_KEY=sk_test_xxx\n"),
        "{}",
        p.read(".env")
    );
    assert!(p
        .read(".env.example")
        .contains("# Payment key (required)\nMY_API_KEY="));
    assert!(!p.read(".env.example").contains("sk_test_xxx"));
}
