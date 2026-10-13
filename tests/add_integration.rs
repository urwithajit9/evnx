//! Integration tests for `evnx add` command.
//!
//! # Conventions
//!
//! Defects known on 0.10.1 are pinned first as `known_<issue>_*` tests that
//! assert today's *wrong* behaviour (section "KNOWN IN 0.10.1" below). The PR
//! that fixes an issue flips its test: it asserts the right behaviour and drops
//! the `known_` prefix, so every fix is a visible, reviewed change. Same rule as
//! `init_integration.rs`. The plan that schedules each fix is
//! `local/add/plan/PLAN-ADD.md` in the review package; the package that flips
//! a test is named in its doc comment.
//!
//! Interactive prompts are tested in `add_pty.rs`, the files `add` writes are
//! pinned byte for byte in `add_golden.rs`.

mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use support::{run, Project};
use tempfile::TempDir;

fn read_env_example(dir: &std::path::Path) -> anyhow::Result<String> {
    std::fs::read_to_string(dir.join(".env.example")).map_err(|e| anyhow::anyhow!("{}", e))
}

fn setup_minimal_project(dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::write(
        dir.join(".env.example"),
        "# Minimal project\nAPP_NAME=test\n",
    )?;
    std::fs::write(dir.join(".env"), "APP_NAME=\n")?;
    Ok(())
}

fn setup_postgres_project(dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::write(dir.join(".env.example"),
        "DATABASE_URL=postgresql://user:pass@localhost/prod_db\nDB_HOST=prod-db.example.com\nDB_NAME=production\n")?;
    Ok(())
}

const DJANGO_VARS: &[&str] = &[
    "SECRET_KEY",
    "DEBUG",
    "ALLOWED_HOSTS",
    "DJANGO_SETTINGS_MODULE",
];

// ─────────────────────────────────────────────────────────────
// Add Service Tests
// ─────────────────────────────────────────────────────────────

/// T-003: was commented out, asserting 0.10.0-era wording (`Appended`,
/// `# [ADDED] Database`). Rewritten against today's output.
#[test]
fn add_service_postgresql_appends_vars() {
    let dir = TempDir::new().unwrap();
    setup_minimal_project(dir.path()).unwrap();

    cargo_bin_cmd!("evnx")
        .args(["add", "service", "postgresql", "--yes", "--path"])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 6 variables for PostgreSQL"));

    let example = read_env_example(dir.path()).unwrap();
    for var in [
        "DATABASE_URL",
        "DB_HOST",
        "DB_NAME",
        "DB_PASSWORD",
        "DB_PORT",
        "DB_USER",
    ] {
        assert!(
            example.contains(&format!("\n{var}=")),
            "missing {var}:\n{example}"
        );
    }
    assert!(example.contains("# ── Database ──"), "{example}");
    assert!(example.contains("APP_NAME=test"), "original content lost");

    let env = std::fs::read_to_string(dir.path().join(".env")).unwrap();
    assert!(env.contains("\nDATABASE_URL="), "{env}");
}

#[test]
fn add_service_unknown_returns_error() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("service")
        .arg("nonexistent_service_xyz")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .failure()
        // The message now comes from the shared catalogue, so it suggests near
        // matches and points at --list-components rather than at this
        // subcommand's --help.
        .stderr(predicate::str::contains("unknown component"));
}

/// I-016 (KNOWN in 0.10.1; flipped by AD-S): adding a service twice writes
/// every name twice, in both files, and the second run still says it added
/// them. T-003: this was `add_service_multiple_times_deduplicates`, commented
/// out with the right expectation (`DATABASE_URL` once); it comes back as that
/// test when AD-S lands.
#[test]
fn known_i016_re_adding_a_service_duplicates_every_name() {
    let p = Project::new().example("APP_NAME=test\n").env("APP_NAME=\n");
    run(p.path(), &["add", "service", "postgresql", "--yes"]).assert_code(0);
    run(p.path(), &["add", "service", "postgresql", "--yes"])
        .assert_code(0)
        .assert_contains("Added 6 variables for PostgreSQL");
    assert_eq!(p.count_lines(".env.example", "DATABASE_URL="), 2);
    assert_eq!(p.count_lines(".env", "DATABASE_URL="), 2);
}

// ─────────────────────────────────────────────────────────────
// Add Framework Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn add_framework_django_appends_vars() {
    let dir = TempDir::new().unwrap();
    setup_minimal_project(dir.path()).unwrap();

    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("framework")
        .arg("--language")
        .arg("python")
        .arg("django")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    // Should have Django vars
    for var in DJANGO_VARS {
        assert!(
            example.contains(&format!("{}=", var)),
            "Should have {}",
            var
        );
    }

    // Should have framework section
    assert!(
        example.contains("Framework: Django") || example.contains("# ── Framework ──"),
        "Should have framework section marker"
    );
}

#[test]
fn add_framework_unknown_language_error() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("framework")
        .arg("--language")
        .arg("unknown_lang_xyz")
        .arg("django")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown language"));
}

// ─────────────────────────────────────────────────────────────
// Add Blueprint Tests (with conflict detection)
// ─────────────────────────────────────────────────────────────

#[test]
fn add_blueprint_skips_conflicting_vars() {
    let dir = TempDir::new().unwrap();
    setup_postgres_project(dir.path()).unwrap();

    // Add T3 blueprint which includes PostgreSQL (has DATABASE_URL)
    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("blueprint")
        .arg("t3_modern")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Conflicts detected").or(predicate::str::contains("Added")),
        );

    let example = read_env_example(dir.path()).unwrap();

    // Original DATABASE_URL should be preserved (conflict skipped)
    assert!(
        example.contains("DATABASE_URL=postgresql://user:pass@localhost/prod_db"),
        "Original DATABASE_URL should be preserved"
    );

    // Other T3 vars should be added
    assert!(
        example.contains("NEXTAUTH_SECRET="),
        "Should add Next.js vars despite conflict"
    );
    assert!(
        example.contains("CLERK_SECRET_KEY="),
        "Should add Clerk vars"
    );
}

#[test]
fn add_blueprint_to_empty_project() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("blueprint")
        .arg("rust_high_perf")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    // Should have Rust framework vars
    assert!(example.contains("RUST_LOG="));
    assert!(example.contains("SOCKET_ADDR="));

    // Should have service vars
    assert!(example.contains("DATABASE_URL=")); // PostgreSQL
    assert!(example.contains("REDIS_URL=")); // Redis
    assert!(example.contains("SENTRY_DSN=")); // Sentry

    // Should be properly sectioned
    assert!(
        example.contains("# ── Blueprint:"),
        "Should have blueprint section header"
    );
}

// ─────────────────────────────────────────────────────────────
// Add Custom Tests
//
// `add custom` is interactive only on 0.10.1 (I-025), so its tests live in
// `add_pty.rs`. T-002: the `#[ignore]`d `add_custom_interactive` that piped
// keys into stdin (which dialoguer refuses) is replaced there by
// `custom_writes_the_variable_and_its_section`.
// ─────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────
// Combined Workflow Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn workflow_init_then_add_service() {
    let dir = TempDir::new().unwrap();

    // Step 1: Init with Blank mode
    cargo_bin_cmd!("evnx")
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("0\n") // Blank
        .assert()
        .success();

    // Step 2: Add PostgreSQL service
    cargo_bin_cmd!("evnx")
        .arg("add")
        .arg("service")
        .arg("postgresql")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    // Should have PostgreSQL vars added to initially blank file
    assert!(
        example.contains("DATABASE_URL="),
        "Should have added PostgreSQL vars"
    );
    // ⚠️ `add` writes the same section header `init` does. It used to write
    // `# [ADDED] Database` into the same file `init` had written
    // `# ── Database ──` into, so one `.env.example` carried two formats
    // depending on which verb put each block there.
    assert!(
        example.contains("# ── Database ──"),
        "add should write init's section header:\n{example}"
    );
    assert!(
        !example.contains("[ADDED]"),
        "the [ADDED] marker should be gone:\n{example}"
    );
}

/// T-003: was commented out, asserting `# [ADDED] <category>` headings that no
/// longer exist. Rewritten: three services, three of init's section headings.
#[test]
fn workflow_add_multiple_services() {
    let dir = TempDir::new().unwrap();
    setup_minimal_project(dir.path()).unwrap();

    for service in ["redis", "stripe", "sentry"] {
        cargo_bin_cmd!("evnx")
            .args(["add", "service", service, "--yes", "--path"])
            .arg(dir.path())
            .assert()
            .success();
    }

    let example = read_env_example(dir.path()).unwrap();
    for var in ["REDIS_URL=", "STRIPE_SECRET_KEY=", "SENTRY_DSN="] {
        assert!(example.contains(var), "missing {var}:\n{example}");
    }
    for heading in [
        "# ── Cache ──",
        "# ── Payments ──",
        "# ── Error Tracking ──",
    ] {
        assert!(example.contains(heading), "missing {heading}:\n{example}");
    }
}

// ─────────────────────────────────────────────────────────────
// .gitignore: add writes to .env, so it has to look
// ─────────────────────────────────────────────────────────────

/// ⚠️ The gap this closes. `evnx add` appends to `.env` and never looked at
/// `.gitignore`, so in an unprotected repository evnx itself added lines to a
/// file holding live credentials, on its way into a commit, without a word.
#[test]
fn add_warns_when_env_is_committable() {
    let dir = TempDir::new().unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=live_value\n").unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .args(["add", "service", "postgresql", "--yes", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(out.contains(".gitignore"), "no warning:\n{out}");
    assert!(out.contains("doctor --fix"), "no remedy offered:\n{out}");
}

/// ⚠️ And it must stay quiet when the file *is* protected — including by the
/// `.env*` that `evnx init` writes. An exact-match check would warn on every
/// project evnx set up correctly, and a warning people learn to ignore is one
/// they will ignore when it matters.
#[test]
fn add_is_quiet_when_env_is_already_ignored() {
    for gitignore in [".env*\n!.env.example\n", ".env\n"] {
        let dir = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=x\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), gitignore).unwrap();

        let assert = cargo_bin_cmd!("evnx")
            .args(["add", "service", "postgresql", "--yes", "--path"])
            .arg(dir.path())
            .assert()
            .success();

        let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
        assert!(
            !out.contains("not covered by .gitignore"),
            "warned despite {gitignore:?}:\n{out}"
        );
    }
}

/// No `.env` means nothing was written to, so nothing to warn about.
#[test]
fn add_says_nothing_when_there_is_no_env_file() {
    let dir = TempDir::new().unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .args(["add", "service", "postgresql", "--yes", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(!out.contains("not covered by .gitignore"), "{out}");
}

// ── What `add` writes into the file people then read and edit ───────────────

/// ⚠️ `  # (required)` used to be written on a line of its own *after* the
/// assignment, which left it floating between two unrelated variables with
/// nothing to say which one it described:
///
/// ```text
/// # TODO: DB_NAME=your_db_name_value  # <-- Fill in real value
///   # (required)
/// # Database password (use strong value in production)
/// ```
///
/// Harmless to dotenv parsing — leading whitespace then `#` is still a comment —
/// and wrong in a file whose whole job is to be read by a person.
#[test]
fn a_required_marker_sits_above_the_variable_it_describes() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["add", "service", "postgresql", "--yes"])
        .assert()
        .success();

    let body = std::fs::read_to_string(dir.path().join(".env")).unwrap();

    assert!(
        !body.contains("\n  # (required)"),
        "the marker must not be written as a detached, indented line:\n{body}"
    );

    // Every occurrence must be immediately followed by a variable line, so it is
    // attached to something.
    let lines: Vec<&str> = body.lines().collect();
    let markers = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == "# (required)" || l.trim_end().ends_with("(required)"));
    let mut seen = 0;
    for (i, line) in markers {
        seen += 1;
        let next = lines.get(i + 1).copied().unwrap_or("");
        assert!(
            next.contains('='),
            "`{line}` is not attached to a variable; next line was `{next}`\n{body}"
        );
    }
    assert!(seen > 0, "postgresql has required variables:\n{body}");
}

/// A text file that does not end with a newline makes `cat a b` join the last
/// line of the first to the first line of the second, and tools that read
/// `.env` line by line can drop the final variable.
#[test]
fn the_files_add_writes_end_with_a_newline() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(["add", "service", "postgresql", "--yes"])
        .assert()
        .success();

    for name in [".env", ".env.example"] {
        let body = std::fs::read_to_string(dir.path().join(name)).unwrap();
        assert!(
            body.ends_with('\n'),
            "{name} does not end with a newline; last 40 bytes: {:?}",
            &body[body.len().saturating_sub(40)..]
        );
        assert!(
            !body.ends_with("\n\n"),
            "{name} ends with a blank line, which `add` should not accumulate"
        );
    }
}

// ─────────────────────────────────────────────────────────────
// Coverage of existing behaviour (AD-0, 0.10.2 baseline)
// ─────────────────────────────────────────────────────────────

/// The names an `Added N variables` run wrote, counted from the file.
fn assignments(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            let l = l.trim_start();
            !l.starts_with('#') && l.contains('=')
        })
        .count()
}

/// T-ADD-30: one id from each catalogue group goes through `add service`, and
/// the count it reports is the count it writes.
#[test]
fn add_service_happy_path_for_each_group() {
    for (id, said) in [
        ("postgresql", "Added 6 variables for PostgreSQL"),
        ("vercel", "Added 4 variables for Vercel"),
        ("nextjs", "Added 4 variables for Next.js"),
    ] {
        let p = Project::new();
        run(p.path(), &["add", "service", id, "--yes"])
            .assert_code(0)
            .assert_contains(said);
        let n: usize = said.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert_eq!(assignments(&p.read(".env.example")), n, "{id}");
    }
}

/// T-ADD-32: an unknown blueprint lists the real ones, exits 2, writes nothing.
#[test]
fn add_blueprint_unknown_id_lists_the_blueprints() {
    let p = Project::new();
    run(p.path(), &["add", "blueprint", "fastapi_full", "--yes"])
        .assert_code(2)
        .assert_contains("Unknown blueprint 'fastapi_full'")
        .assert_contains("fastapi_ai");
    assert!(!p.exists(".env.example"));
}

/// T-ADD-33: every refusal exits 2, not 1 (DA-08), and writes nothing.
#[test]
fn every_refusal_exits_2_and_writes_nothing() {
    for args in [
        &["add", "service", "no_such_component", "--yes"][..],
        &[
            "add",
            "framework",
            "--language",
            "klingon",
            "django",
            "--yes",
        ],
        &["add", "framework", "--language", "python", "nope", "--yes"],
        &["add", "framework", "nextjs", "--yes"],
        &["add", "blueprint", "nope", "--yes"],
        &["add"],
    ] {
        let p = Project::new();
        run(p.path(), args).assert_code(2);
        assert!(!p.exists(".env.example"), "{args:?} wrote a file");
    }
}

/// T-ADD-34: with a spec, `add service` records the new names once; a second
/// add leaves one table per name.
#[test]
fn add_service_records_names_in_the_spec_once() {
    let p = Project::new()
        .example("APP_NAME=test\n")
        .env("APP_NAME=x\n");
    run(p.path(), &["spec", "init"]).assert_code(0);
    run(p.path(), &["add", "service", "stripe", "--yes"])
        .assert_code(0)
        .assert_contains("to [vars] in .evnx.toml");
    run(p.path(), &["add", "service", "stripe", "--yes"]).assert_code(0);
    assert_eq!(
        p.count_lines(".evnx.toml", "[vars.STRIPE_SECRET_KEY]"),
        1,
        "{}",
        p.read(".evnx.toml")
    );
}

/// I-019 (half fixed in 0.10.1 by init's shared `-q`): `add service` is
/// silent under `-q`, and errors still print. The other half is
/// `known_i019b_*` below.
#[test]
fn quiet_add_service_prints_nothing_but_errors() {
    let p = Project::new();
    let out = run(p.path(), &["-q", "add", "service", "stripe", "--yes"]);
    out.assert_code(0);
    assert_eq!(out.all().trim(), "", "{}", out.all());
    run(p.path(), &["-q", "add", "service", "nosuch", "--yes"])
        .assert_code(2)
        .assert_contains("unknown component");
}

/// Aliases and typo hints reach `add service` through init's shared resolver
/// (fixed in 0.10.1, CH-INIT-11/12). Pinned so `add` keeps them.
#[test]
fn add_service_takes_aliases_and_suggests_on_typos() {
    let p = Project::new();
    run(p.path(), &["add", "service", "pg", "--yes"])
        .assert_code(0)
        .assert_contains("Added 6 variables for PostgreSQL");
    run(p.path(), &["add", "service", "supabse", "--yes"])
        .assert_code(2)
        .assert_contains("Did you mean: supabase");
}

// ─────────────────────────────────────────────────────────────
// Decided to stay (accepted remainders; PLAN-ADD §4)
// ─────────────────────────────────────────────────────────────

/// I-022, DQ-A5 (2026-10-14): the `# Added by evnx add on <date>` stamp stays
/// the last line of the block it describes, the way init's footer closes its
/// output.
#[test]
fn the_add_stamp_closes_its_block() {
    let p = Project::new().example("APP_NAME=test\n");
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    let last = example.lines().rfind(|l| !l.trim().is_empty()).unwrap();
    assert!(last.starts_with("# Added by evnx add on "), "{example}");
}

/// I-022b, DQ-A5: an add after `init` appends below init's footer; the footer
/// stays where init wrote it.
#[test]
fn init_footer_stays_where_init_wrote_it() {
    let p = Project::new();
    run(p.path(), &["init", "--with", "vite", "--yes"]).assert_code(0);
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    let footer = example.find("# Generated by evnx").expect("init footer");
    let redis = example.find("REDIS_URL=").expect("redis block");
    assert!(footer < redis, "{example}");
}

/// I-027, DQ-A11 (T-ADD-62): `add` writes `.env` and `.env.example` only;
/// other environment files are not touched.
#[test]
fn other_env_files_are_left_alone() {
    let p = Project::new()
        .example("A=\n")
        .env("A=1\n")
        .file(".env.local", "X=1\n")
        .file(".env.staging", "X=1\n")
        .file(".env.production", "X=1\n");
    run(p.path(), &["add", "service", "stripe", "--yes"]).assert_code(0);
    for f in [".env.local", ".env.staging", ".env.production"] {
        assert_eq!(p.read(f), "X=1\n", "{f} changed");
    }
}

// ─────────────────────────────────────────────────────────────
// KNOWN IN 0.10.1 — each test pins a defect; the named package flips it
// ─────────────────────────────────────────────────────────────

/// I-016 (AD-S): `add blueprint` re-adds a name whose value is identical; it
/// skips only names whose value differs.
#[test]
fn known_i016d_blueprint_re_adds_identical_names() {
    let p = Project::new();
    run(p.path(), &["add", "service", "postgresql", "--yes"]).assert_code(0);
    run(p.path(), &["add", "blueprint", "t3_modern", "--yes"]).assert_code(0);
    assert_eq!(p.count_lines(".env.example", "DATABASE_URL="), 2);
}

/// I-016 (AD-S; DA-14 in the docs): `add service` appends a name whose
/// existing value differs, with no conflict shown.
#[test]
fn known_i016e_a_differing_value_is_appended_without_warning() {
    let dir = TempDir::new().unwrap();
    setup_postgres_project(dir.path()).unwrap();
    let out = run(dir.path(), &["add", "service", "postgresql", "--yes"]);
    out.assert_code(0);
    assert!(
        !out.all().to_lowercase().contains("conflict"),
        "{}",
        out.all()
    );
    let example = read_env_example(dir.path()).unwrap();
    assert_eq!(
        example
            .lines()
            .filter(|l| l.starts_with("DATABASE_URL="))
            .count(),
        2
    );
}

/// The real value `REDIS_URL` holds in `.env` before each N-11 run.
const REAL_REDIS: &str = "REDIS_URL=redis://real-host:6379/0";

/// The value dotenv uses: the last assignment wins.
fn effective(text: &str, key: &str) -> Option<String> {
    text.lines()
        .rfind(|l| l.starts_with(&format!("{key}=")))
        .map(str::to_string)
}

/// N-11 (high; AD-S): a re-add appends the placeholder below the user's real
/// value in `.env`, and the placeholder is the one that takes effect.
#[test]
fn known_n11_a_re_add_shadows_the_real_env_value() {
    let p = Project::new()
        .example("REDIS_URL=redis://localhost:6379/0\n")
        .env(&format!("{REAL_REDIS}\n"));
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert_eq!(
        effective(&p.read(".env"), "REDIS_URL").as_deref(),
        Some("REDIS_URL=redis://localhost:6379/0")
    );
}

/// N-11b (AD-S): `add blueprint` looks for conflicts in `.env.example` only, so
/// a real value in `.env` is shadowed and no conflict is shown.
#[test]
fn known_n11b_blueprint_misses_a_conflict_in_env() {
    let p = Project::new()
        .example("REDIS_URL=redis://localhost:6379/0\n")
        .env(&format!("{REAL_REDIS}\n"));
    let out = run(p.path(), &["add", "blueprint", "fastapi_ai", "--yes"]);
    out.assert_code(0)
        .assert_not_contains("REDIS_URL (existing");
    assert_ne!(
        effective(&p.read(".env"), "REDIS_URL").as_deref(),
        Some(REAL_REDIS)
    );
}

/// I-017 (AD-C): `add framework` needs `--language`, though framework ids are
/// unique across languages.
#[test]
fn known_i017_framework_needs_language() {
    let p = Project::new();
    run(p.path(), &["add", "framework", "nextjs", "--yes"])
        .assert_code(2)
        .assert_contains("--language <LANGUAGE>");
}

/// I-017b (AD-C): the wrong language for a known framework lists that
/// language's frameworks and does not say which language `nextjs` belongs to.
#[test]
fn known_i017b_wrong_language_gives_no_hint() {
    let p = Project::new();
    run(
        p.path(),
        &[
            "add",
            "framework",
            "--language",
            "python",
            "nextjs",
            "--yes",
        ],
    )
    .assert_code(2)
    .assert_contains("Unknown framework 'nextjs' for language 'python'")
    .assert_not_contains("javascript_typescript");
}

/// I-018 (AD-C): `add service` takes an infra id without saying so, and there
/// is no `add infra` (DQ-A7 adds it).
#[test]
fn known_i018_service_takes_infra_without_a_hint() {
    let p = Project::new();
    let out = run(p.path(), &["add", "service", "vercel", "--yes"]);
    out.assert_code(0);
    assert!(!out.all().to_lowercase().contains("infra"), "{}", out.all());
    run(p.path(), &["add", "infra", "vercel", "--yes"])
        .assert_code(2)
        .assert_contains("unrecognized subcommand 'infra'");
}

/// I-019b and N-15 (AD-A, DQ-A6): `framework` and `blueprint` print the preview
/// under `--yes`, and under `-q` too; `service` does not.
#[test]
fn known_i019b_n15_yes_and_quiet_still_preview_framework_and_blueprint() {
    let p = Project::new();
    run(
        p.path(),
        &["-q", "add", "framework", "-l", "python", "django", "--yes"],
    )
    .assert_code(0)
    .assert_contains("Preview:");
    run(p.path(), &["add", "blueprint", "mern_v2", "--yes"])
        .assert_code(0)
        .assert_contains("Preview:");
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_not_contains("Preview:");
}

/// I-020 (AD-A): with no terminal and no `--yes`, `add` prints the preview and
/// then dialoguer's raw, doubled error, with no hint. Exit 2, nothing written.
#[test]
fn known_i020_no_terminal_gives_a_raw_error_after_the_preview() {
    let p = Project::new();
    run(p.path(), &["add", "service", "twilio"])
        .assert_code(2)
        .assert_contains("Preview:")
        .assert_contains("not a terminal: not a terminal")
        .assert_not_contains("--yes");
    assert!(!p.exists(".env.example"));
}

/// I-020 / I-025 (AD-F): `add custom --yes` still needs a terminal.
#[test]
fn known_i020c_custom_yes_needs_a_terminal() {
    let p = Project::new();
    run(p.path(), &["add", "custom", "--yes"]).assert_code(2);
    assert!(!p.exists(".env.example"));
}

/// I-021 (AD-S, DQ-A4): with no `.env`, `add` writes only `.env.example`.
/// After AD-S it creates `.env` from the template, as `init` does.
#[test]
fn known_i021_no_env_is_created() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert!(p.exists(".env.example"));
    assert!(!p.exists(".env"));
}

/// I-022 plurals (AD-A): `1 conflicts`, `1 conflicting variables`.
#[test]
fn known_i022c_one_conflict_is_plural() {
    let p = Project::new().example("DATABASE_URL=postgres://mine\n");
    run(p.path(), &["add", "blueprint", "t3_modern", "--yes"])
        .assert_code(0)
        .assert_contains("1 conflicts detected")
        .assert_contains("Skipped 1 conflicting variables");
}

/// I-024 (AD-A; DA-15): `-v` says `.env` got "TODO placeholders", which `add`
/// stopped writing in v0.5.0.
#[test]
fn known_i024_verbose_mentions_todo_placeholders() {
    let p = Project::new().example("A=\n").env("A=1\n");
    run(p.path(), &["-v", "add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("with TODO placeholders");
}

/// I-025 (AD-F): `add custom` takes no `NAME=value` arguments.
#[test]
fn known_i025_custom_takes_no_arguments() {
    let p = Project::new();
    run(p.path(), &["add", "custom", "API_KEY=abc", "--yes"])
        .assert_code(2)
        .assert_contains("unexpected argument 'API_KEY=abc'");
}

/// I-026 (AD-G): no `--list`, `--dry-run` or `--json`.
#[test]
fn known_i026_no_list_dry_run_or_json() {
    let p = Project::new();
    for args in [
        &["add", "--list"][..],
        &["add", "service", "redis", "--dry-run", "--yes"],
        &["add", "service", "redis", "--json", "--yes"],
    ] {
        run(p.path(), args)
            .assert_code(2)
            .assert_contains("unexpected argument");
    }
    assert!(!p.exists(".env.example"));
}

/// N-10 (AD-A): the preview's `• framework / service / infra` lines come out
/// of a `HashMap`, so their order changes between runs.
///
/// ⚠️ Probabilistic: with three groups, ten runs all in the same order has a
/// chance of about 1 in 10 million.
#[test]
fn known_n10_preview_order_changes_between_runs() {
    let mut orders = std::collections::BTreeSet::new();
    for _ in 0..10 {
        let p = Project::new();
        let out = run(p.path(), &["add", "blueprint", "t3_modern", "--yes"]);
        let order: Vec<String> = out
            .stdout
            .lines()
            .filter(|l| l.trim_start().starts_with("• "))
            .map(str::to_string)
            .collect();
        orders.insert(order);
    }
    assert!(orders.len() > 1, "one order in 10 runs: {orders:?}");
}

/// N-14 (AD-S) and N-14b (AD-A): with a read-only `.env`, the run fails after
/// `.env.example` was already appended, and the error does not name the file.
#[cfg(unix)]
#[test]
fn known_n14_read_only_env_leaves_a_partial_write() {
    let p = Project::new()
        .example("A=\n")
        .env("A=1\n")
        .file(".gitignore", ".env\n")
        .mode(".env", 0o444);
    if std::fs::OpenOptions::new()
        .append(true)
        .open(p.join(".env"))
        .is_ok()
    {
        return; // running as root: permissions do not apply
    }
    let out = run(p.path(), &["add", "service", "redis", "--yes"]);
    out.assert_code(2).assert_contains("Permission denied");
    assert!(!out.all().contains(".env"), "{}", out.all());
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 1);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 0);
}

/// N-16 (AD-B): a CRLF `.env.example` gets LF lines appended.
#[test]
fn known_n16_crlf_file_gets_lf_lines() {
    let p = Project::new().example("A=1\r\nB=2\r\n");
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    let raw = std::fs::read(p.join(".env.example")).unwrap();
    let text = String::from_utf8(raw).unwrap();
    let line = text
        .split('\n')
        .find(|l| l.starts_with("REDIS_URL="))
        .unwrap();
    assert!(!line.ends_with('\r'), "already CRLF: {line:?}");
    assert!(text.starts_with("A=1\r\n"), "original line endings changed");
}

/// BR-1 (AD-H, DQ-A8): `add service supabase` on a Vite project adds no
/// `VITE_` copies of the public variables. `init --with vite,supabase` does.
#[test]
fn known_br1_service_adds_no_browser_copies() {
    let p = Project::new()
        .file("package.json", r#"{"devDependencies":{"vite":"5"}}"#)
        .example("VITE_APP_TITLE=x\n");
    run(p.path(), &["add", "service", "supabase", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    assert!(example.contains("\nSUPABASE_URL="), "{example}");
    assert!(!example.contains("VITE_SUPABASE_URL="), "{example}");
}
