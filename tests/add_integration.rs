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

/// I-016 (fixed in 0.10.2, AD-S; was `known_i016_*`). T-003: this is the
/// test that sat commented out as `add_service_multiple_times_deduplicates`,
/// with the right expectation, while the product duplicated every name.
#[test]
fn add_service_multiple_times_deduplicates() {
    let p = Project::new().example("APP_NAME=test\n").env("APP_NAME=\n");
    run(p.path(), &["add", "service", "postgresql", "--yes"]).assert_code(0);
    run(p.path(), &["add", "service", "postgresql", "--yes"])
        .assert_code(0)
        .assert_contains("Nothing to add: 6 variables for PostgreSQL already in .env.example")
        .assert_not_contains("Added");
    assert_eq!(p.count_lines(".env.example", "DATABASE_URL="), 1);
    assert_eq!(p.count_lines(".env", "DATABASE_URL="), 1);
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

    // stderr: a warning and its remedy are notices, not output (DEC-G2).
    let out = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
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
            !out.contains("is not in .gitignore"),
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
    assert!(!out.contains("is not in .gitignore"), "{out}");
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
        // 3, not 4: `NEXT_PUBLIC_VERCEL_ENV` only in a Next.js project
        // (I-002b, DQ-23); this one has no framework.
        ("vercel", "Added 3 variables for Vercel"),
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
        &["add", "framework", "nope", "--yes"],
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

/// I-016 (fixed in 0.10.2, AD-S; was `known_i016d_*`): `add blueprint` skips
/// a name already present with the same value, and says how many it skipped.
#[test]
fn blueprint_skips_names_already_present() {
    let p = Project::new();
    run(p.path(), &["add", "service", "postgresql", "--yes"]).assert_code(0);
    run(p.path(), &["add", "blueprint", "t3_modern", "--yes"])
        .assert_code(0)
        // t3_modern's six PostgreSQL names are already there, same values.
        .assert_contains("skipped 6 already present");
    assert_eq!(p.count_lines(".env.example", "DATABASE_URL="), 1);
}

/// I-016, DQ-A3 (fixed in 0.10.2, AD-S; was `known_i016e_*`): a name whose
/// value in `.env.example` differs is kept, not appended, and listed. All four
/// subcommands do this now; only `blueprint` did on 0.10.1.
#[test]
fn a_differing_value_is_kept_and_listed() {
    let dir = TempDir::new().unwrap();
    setup_postgres_project(dir.path()).unwrap();
    run(dir.path(), &["add", "service", "postgresql", "--yes"])
        .assert_code(0)
        .assert_contains("with a different value in .env.example: kept as it is")
        .assert_contains("DATABASE_URL (existing: \"postgresql://user:pass@localhost/prod_db\"");
    let example = read_env_example(dir.path()).unwrap();
    assert_eq!(
        example
            .lines()
            .filter(|l| l.starts_with("DATABASE_URL="))
            .count(),
        1
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

/// N-11 (fixed in 0.10.2, AD-S; was `known_n11_*`): a name `.env` already
/// has is not appended there again, so the user's real value stays the one in
/// effect. On 0.10.1 the placeholder landed below it and won.
#[test]
fn a_re_add_keeps_the_real_env_value() {
    let p = Project::new()
        .example("REDIS_URL=redis://localhost:6379/0\n")
        .env(&format!("{REAL_REDIS}\n"));
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 1);
    assert_eq!(
        effective(&p.read(".env"), "REDIS_URL").as_deref(),
        Some(REAL_REDIS)
    );
}

/// N-11b (fixed in 0.10.2, AD-S; was `known_n11b_*`): `add blueprint` keeps
/// a real `.env` value too. `.env`'s value is never printed: it is a secret,
/// and differs from the placeholder by design.
#[test]
fn blueprint_keeps_the_real_env_value() {
    let p = Project::new()
        .example("REDIS_URL=redis://localhost:6379/0\n")
        .env(&format!("{REAL_REDIS}\n"));
    run(p.path(), &["add", "blueprint", "fastapi_ai", "--yes"])
        .assert_code(0)
        .assert_not_contains("real-host");
    assert_eq!(
        effective(&p.read(".env"), "REDIS_URL").as_deref(),
        Some(REAL_REDIS)
    );
}

/// DQ-A2: per file. A name `.env.example` already has but `.env` lacks is
/// added to `.env` only, and the run says so.
#[test]
fn env_catches_up_with_the_template() {
    let p = Project::new()
        .example("REDIS_URL=redis://localhost:6379/0\n")
        .env("OTHER=1\n");
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("Added 4 variables for Redis, skipped 1 already present")
        .assert_contains("Added 1 variable to .env that .env.example already had: REDIS_URL");
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 1);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 1);
}

/// I-017 (fixed in 0.10.2, AD-C; was `known_i017_*`): `--language` is
/// optional, as framework ids are unique; aliases work too. On 0.10.1
/// `add framework nextjs` was a clap error, though the README showed it.
#[test]
fn framework_needs_no_language() {
    for id in ["nextjs", "next"] {
        let p = Project::new();
        run(p.path(), &["add", "framework", id, "--yes"])
            .assert_code(0)
            .assert_contains("Added 4 variables for Next.js");
    }
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
    .assert_code(0)
    .assert_contains("Added 5 variables for Django");
}

/// I-017b (fixed in 0.10.2, AD-C; was `known_i017b_*`): the wrong language
/// for a known framework says which language it belongs to. An unknown id
/// with a language still lists that language's frameworks; without one it
/// gets the shared catalogue message with near matches.
#[test]
fn wrong_language_names_the_right_one() {
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
    .assert_contains(
        "nextjs is a javascript_typescript framework: use --language javascript_typescript",
    );
    run(
        p.path(),
        &["add", "framework", "--language", "python", "nope", "--yes"],
    )
    .assert_code(2)
    .assert_contains("Available: django, fastapi_flask");
    run(p.path(), &["add", "framework", "nxtjs", "--yes"])
        .assert_code(2)
        .assert_contains("Did you mean: nextjs");
    assert!(!p.exists(".env.example"));
}

/// I-018 (fixed in 0.10.2, AD-C, DQ-A7; was `known_i018_*`): one id space.
/// `add service` and `add framework` take any catalogue id and say what it
/// is; `add infra` is `add service` under another name. On 0.10.1 an infra id
/// went through `add service` silently and `add infra` did not exist.
#[test]
fn any_id_through_any_subcommand_with_a_hint() {
    for (args, hint) in [
        (
            &["add", "service", "vercel"][..],
            "vercel is an infra component; adding it.",
        ),
        (
            &["add", "infra", "vercel"],
            "vercel is an infra component; adding it.",
        ),
        (
            &["add", "service", "nextjs"],
            "nextjs is a javascript_typescript framework; adding it.",
        ),
        (
            &["add", "framework", "postgresql"],
            "postgresql is a service component; adding it.",
        ),
        (
            &["add", "framework", "vercel"],
            "vercel is an infra component; adding it.",
        ),
    ] {
        let p = Project::new();
        let mut full = args.to_vec();
        full.push("--yes");
        let out = run(p.path(), &full);
        out.assert_code(0).assert_contains(hint);
        assert_eq!(out.all().matches("adding it.").count(), 1, "{}", out.all());
    }
    // A service through `add service` or `add infra` needs no hint.
    let p = Project::new();
    run(p.path(), &["add", "infra", "postgresql", "--yes"])
        .assert_code(0)
        .assert_not_contains("adding it.");
}

/// I-019b and N-15 (fixed in 0.10.2, AD-A, DQ-A6; was
/// `known_i019b_n15_*`): `--yes` never previews, in any subcommand, and `-q`
/// leaves only problems. On 0.10.1 `framework` and `blueprint` previewed under
/// `--yes` and under `-q`; `service` did not.
#[test]
fn yes_never_previews_and_quiet_shows_only_problems() {
    let p = Project::new();
    for args in [
        &["add", "framework", "-l", "python", "django", "--yes"][..],
        &["add", "blueprint", "mern_v2", "--yes"],
        &["add", "service", "redis", "--yes"],
    ] {
        run(p.path(), args)
            .assert_code(0)
            .assert_not_contains("Preview:");
    }
    let q = Project::new().example("DATABASE_URL=postgres://mine\n");
    let out = run(q.path(), &["-q", "add", "blueprint", "t3_modern", "--yes"]);
    out.assert_code(0)
        .assert_contains("1 variable with a different value in .env.example")
        .assert_contains("DATABASE_URL (existing")
        .assert_not_contains("Added")
        .assert_not_contains("Preview:");
}

/// I-020 (fixed in 0.10.2, AD-A; was `known_i020_*`): with no terminal and no
/// `--yes`, `add` stops before the preview and says how to run it. On 0.10.1
/// it printed the preview, then dialoguer's raw, doubled `not a terminal`.
#[test]
fn no_terminal_says_to_add_yes() {
    for (args, example) in [
        (
            &["add", "service", "twilio"][..],
            "evnx add service twilio --yes",
        ),
        (
            &["add", "framework", "-l", "python", "django"],
            "evnx add framework --language python django --yes",
        ),
        (
            &["add", "blueprint", "mern_v2"],
            "evnx add blueprint mern_v2 --yes",
        ),
    ] {
        let p = Project::new();
        run(p.path(), args)
            .assert_code(2)
            .assert_contains("there is no terminal to confirm with")
            .assert_contains(example)
            .assert_not_contains("Preview:")
            .assert_not_contains("not a terminal: not a terminal");
        assert!(!p.exists(".env.example"));
    }
}

/// I-020 (fixed in 0.10.2, AD-F; was `known_i020c_*`): with no terminal,
/// `add custom` with no arguments points at the argument form, which needs
/// none. On 0.10.1 `add custom --yes` could not run without a terminal at all.
#[test]
fn custom_without_a_terminal_points_at_arguments() {
    let p = Project::new();
    run(p.path(), &["add", "custom", "--yes"])
        .assert_code(2)
        .assert_contains("evnx add custom NAME=value OTHER_NAME=value --yes");
    assert!(!p.exists(".env.example"));
    run(p.path(), &["add", "custom", "A=1", "--yes"]).assert_code(0);
    // ⚠️ DEC-U8-1 (I-245): the value goes to `.env` only; `.env.example` gets a placeholder.
    assert_eq!(p.count_lines(".env", "A=1"), 1);
    assert!(
        p.read(".env.example").contains("\nA="),
        "the name is in the template"
    );
}

/// I-021, DQ-A4 (fixed in 0.10.2, AD-S; was `known_i021_*`): with no `.env`,
/// `add` creates it from the template, as `init` does: `.gitignore` first,
/// then a byte copy. Under `--yes` without asking. On 0.10.1 it wrote only
/// `.env.example` and said nothing.
#[test]
fn no_env_is_created_from_the_template() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--yes"])
        .assert_code(0)
        .assert_contains("Created .env from .env.example");
    assert_eq!(p.read(".env"), p.read(".env.example"));
    assert!(
        p.read(".gitignore").contains(".env*"),
        "{}",
        p.read(".gitignore")
    );
}

/// I-022 (fixed in 0.10.2, AD-A; was `known_i022c_*`): one conflict is
/// singular. 0.10.1 said `1 conflicts detected`, `Skipped 1 conflicting
/// variables`.
#[test]
fn one_conflict_is_singular() {
    let p = Project::new().example("DATABASE_URL=postgres://mine\n");
    run(p.path(), &["add", "blueprint", "t3_modern", "--yes"])
        .assert_code(0)
        .assert_contains("1 variable with a different value in .env.example")
        .assert_contains("skipped 1 already present")
        .assert_not_contains("1 variables");
}

/// I-024 (fixed in 0.10.2, AD-A; was `known_i024_*`; DA-15): `-v` says what
/// it did to `.env`. It said "with TODO placeholders" two releases after
/// `add` stopped writing them.
#[test]
fn verbose_says_appended() {
    let p = Project::new().example("A=\n").env("A=1\n");
    let out = run(p.path(), &["-v", "add", "service", "redis", "--yes"]);
    out.assert_code(0).assert_not_contains("TODO");
    // The path is printed with the platform's separator (`.\.env` on
    // Windows); the test pins the words, not the separator.
    let stdout = out.stdout.replace('\\', "/");
    assert!(stdout.contains("[DEBUG] Appended to ./.env\n"), "{stdout}");
}

/// I-025, T-ADD-40 (fixed in 0.10.2, AD-F; was `known_i025_*`): `add custom`
/// takes `NAME=value` arguments, several at once, with no terminal.
#[test]
fn custom_takes_name_value_arguments() {
    let p = Project::new().example("APP=1\n").env("APP=1\n");
    run(
        p.path(),
        &[
            "add",
            "custom",
            "API_URL=https://api.example.com",
            "DEBUG=false",
            "--yes",
        ],
    )
    .assert_code(0)
    .assert_contains("Added 2 custom variables");
    for f in [".env.example", ".env"] {
        let text = p.read(f);
        assert!(text.contains("\nAPI_URL="), "{f}:\n{text}");
        assert!(text.contains("\nDEBUG="), "{f}:\n{text}");
        assert!(text.contains("# ── Custom Variables ──"), "{f}:\n{text}");
    }
    // ⚠️ DEC-U8-1 (I-245): the value goes to `.env` only; `.env.example` gets a placeholder.
    let env = p.read(".env");
    assert!(env.contains("\nAPI_URL=https://api.example.com\n"), "{env}");
    assert!(env.contains("\nDEBUG=false\n"), "{env}");
    let example = p.read(".env.example");
    assert!(
        !example.contains("api.example.com") && !example.contains("DEBUG=false"),
        "{example}"
    );
}

/// T-ADD-41: a name without a value gets the placeholder (no terminal, or
/// `--yes`, so nothing is asked even for a secret-looking name).
#[test]
fn custom_name_alone_gets_the_placeholder() {
    let p = Project::new();
    run(
        p.path(),
        &[
            "add",
            "custom",
            "FEATURE_FLAG",
            "STRIPE_SECRET_KEY",
            "--yes",
        ],
    )
    .assert_code(0);
    // ⚠️ DEC-U8-1 (I-245): the value goes to `.env` only; `.env.example` gets a placeholder.
    // A name alone gets the same placeholder in both files (they used to
    // disagree: `your_value_here` in .env, `YOUR_VALUE_HERE` in the template).
    let env = p.read(".env");
    let example = p.read(".env.example");
    for line in [
        "FEATURE_FLAG=YOUR_VALUE_HERE",
        "STRIPE_SECRET_KEY=YOUR_KEY_HERE",
    ] {
        assert!(env.contains(line), "{line}:\n{env}");
        assert!(example.contains(line), "{line}:\n{example}");
    }
}

/// T-ADD-42: an invalid or repeated name fails before anything is written,
/// even when the names before it are fine.
#[test]
fn custom_checks_every_name_before_writing() {
    let p = Project::new();
    for args in [
        &["add", "custom", "GOOD=1", "my var=2", "--yes"][..],
        &["add", "custom", "GOOD=1", "9LIVES=2", "--yes"],
        &["add", "custom", "GOOD=1", "GOOD=2", "--yes"],
        &["add", "custom", "GOOD=1", "--category", "nope", "--yes"],
    ] {
        run(p.path(), args).assert_code(2);
        assert!(!p.exists(".env.example"), "{args:?}");
    }
}

/// T-ADD-43: `--desc`, `--category` (any case) and `--required` apply to
/// every name given, in one comment line above each.
#[test]
fn custom_flags_go_into_the_written_block() {
    let p = Project::new();
    run(
        p.path(),
        &[
            "add",
            "custom",
            "WEBHOOK_SECRET",
            "--desc",
            "Webhook signing secret",
            "--category",
            "security",
            "--required",
            "--yes",
        ],
    )
    .assert_code(0);
    // The comment line goes above the variable in both files (DEC-U8-1:
    // the template's value is a placeholder).
    for f in [".env.example", ".env"] {
        let text = p.read(f);
        assert!(
            text.contains("# [Security] Webhook signing secret (required)\nWEBHOOK_SECRET="),
            "{f}:\n{text}"
        );
    }
}

/// T-ADD-45: a name the template already has is skipped and reported (AD-S).
#[test]
fn custom_argument_already_present_is_skipped() {
    let p = Project::new().example("DEBUG=false\n");
    run(
        p.path(),
        &["add", "custom", "DEBUG=false", "NEW=1", "--yes"],
    )
    .assert_code(0)
    .assert_contains("Added 1 custom variable, skipped 1 already present");
    assert_eq!(p.count_lines(".env.example", "DEBUG="), 1);
    assert_eq!(p.count_lines(".env.example", "NEW="), 1);
}

/// The ids in a `--list`-style listing: the first word of every indented
/// line that looks like an id.
fn listed_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("    "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|w| {
            w.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_string)
        .collect()
}

/// I-026, 7a, 7f, T-ADD-50 (fixed in 0.10.2, AD-G; was `known_i026_*`):
/// `add --list` lists every component `init --list-components` lists, from
/// the same function, then the blueprints.
#[test]
fn add_list_matches_init_and_adds_blueprints() {
    let p = Project::new();
    let add = run(p.path(), &["add", "--list"]);
    add.assert_code(0).assert_contains("BLUEPRINT");
    let init = run(p.path(), &["init", "--list-components"]);
    init.assert_code(0);
    let (components, blueprints) = add.stdout.split_once("BLUEPRINT").unwrap();
    assert_eq!(listed_ids(components), listed_ids(&init.stdout));
    assert!(listed_ids(&init.stdout).len() > 40);
    assert_eq!(listed_ids(blueprints).len(), 10, "{blueprints}");
    assert!(!p.exists(".env.example"));
}

/// I-026, T-ADD-51: `--dry-run` writes nothing, needs no terminal, and exits 0,
/// for every subcommand; it says what would happen.
#[test]
fn dry_run_writes_nothing_in_every_subcommand() {
    for args in [
        &["add", "service", "postgresql", "--dry-run"][..],
        &["add", "framework", "nextjs", "--dry-run"],
        &["add", "blueprint", "t3_modern", "--dry-run"],
        &["add", "custom", "A=1", "B", "--dry-run"],
    ] {
        let p = Project::new()
            .example("DATABASE_URL=postgres://mine\n")
            .env("X=1\n");
        let before = (p.read(".env.example"), p.read(".env"));
        run(p.path(), args)
            .assert_code(0)
            .assert_contains("Dry run")
            .assert_contains("nothing was written")
            .assert_contains(".env.example: would add");
        assert_eq!((p.read(".env.example"), p.read(".env")), before, "{args:?}");
        assert!(!p.exists(".gitignore"), "{args:?}");
    }
}

/// I-026: with no `.env`, a dry run says one would be created, and creates none.
#[test]
fn dry_run_says_env_would_be_created() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--dry-run"])
        .assert_code(0)
        .assert_contains(".env: would be created from .env.example");
    assert!(!p.exists(".env") && !p.exists(".env.example"));
}

/// I-026, 7c, T-ADD-52: `--json` prints one document, alone on stdout, with
/// the stable fields; with `--dry-run` it writes nothing.
#[test]
fn json_reports_added_skipped_and_conflicts() {
    let setup = || {
        Project::new()
            .example("DATABASE_URL=postgres://mine\nDB_PORT=5432\n")
            .env("DB_PORT=1\n")
            .file(".gitignore", ".env\n")
    };
    for (extra, written) in [("--dry-run", false), ("--yes", true)] {
        let p = setup();
        let out = run(p.path(), &["add", "service", "postgresql", "--json", extra]);
        out.assert_code(0);
        let doc: serde_json::Value = serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{}", out.stdout));
        assert_eq!(doc["dry_run"], !written);
        assert_eq!(doc["written"], written);
        assert_eq!(
            doc["added"],
            serde_json::json!(["DB_HOST", "DB_NAME", "DB_PASSWORD", "DB_USER"])
        );
        assert_eq!(doc["skipped"], serde_json::json!(["DB_PORT"]));
        assert_eq!(doc["conflicts"][0]["name"], "DATABASE_URL");
        assert_eq!(doc["conflicts"][0]["existing"], "postgres://mine");
        assert_eq!(doc["env_added"].as_array().unwrap().len(), 4);
        assert_eq!(doc["env_created"], false);
        // Nothing from .env ever appears: its DB_PORT value is "1".
        assert!(!out.stdout.contains("\"1\""), "{}", out.stdout);
        assert_eq!(
            p.count_lines(".env.example", "DB_HOST="),
            usize::from(written)
        );
    }
}

/// I-026: `--json` never asks, so it needs `--yes` or `--dry-run`.
#[test]
fn json_needs_yes_or_dry_run() {
    let p = Project::new();
    run(p.path(), &["add", "service", "redis", "--json"])
        .assert_code(2)
        .assert_contains("--json needs --yes or --dry-run");
    assert!(!p.exists(".env.example"));
}

/// T-SHARE-33 on `add`: the pretty and JSON runs agree on the exit code and on
/// every byte written.
#[test]
fn pretty_and_json_write_the_same_files() {
    support::renderers::both_renderers(
        || {
            Project::new()
                .example("DATABASE_URL=postgres://mine\n")
                .env("X=1\n")
                .file(".gitignore", ".env\n")
        },
        &["add", "service", "postgresql", "--yes"],
        &["add", "service", "postgresql", "--yes", "--json"],
    );
}

/// I-026, 7e, T-ADD-53: every subcommand runs without a terminal under
/// `--yes`; with no id and no terminal, the error points at `--list`.
#[test]
fn every_subcommand_runs_without_a_terminal() {
    for args in [
        &["add", "service", "redis", "--yes"][..],
        &["add", "infra", "vercel", "--yes"],
        &["add", "framework", "django", "--yes"],
        &["add", "blueprint", "mern_v2", "--yes"],
        &["add", "custom", "A=1", "--yes"],
    ] {
        let p = Project::new();
        run(p.path(), args).assert_code(0);
        assert!(p.exists(".env.example"), "{args:?}");
    }
    for args in [
        &["add", "service", "--yes"][..],
        &["add", "framework"],
        &["add", "blueprint", "--dry-run"],
    ] {
        let p = Project::new();
        run(p.path(), args)
            .assert_code(2)
            .assert_contains("evnx add --list");
    }
}

/// N-14 (fixed in 0.10.2, AD-S; was `known_n14_*`): a read-only `.env` stops
/// the run before anything is written. On 0.10.1 `.env.example` had already
/// been appended. N-14b (AD-A): the error names the file.
#[cfg(unix)]
#[test]
fn a_read_only_env_stops_the_run_before_any_write() {
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
    // N-14b (fixed in 0.10.2, AD-A): the error names the file.
    out.assert_code(2)
        .assert_contains("Failed to write ./.env: Permission denied");
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 0);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 0);
}

/// N-16 (fixed in 0.10.2, AD-B; was `known_n16_*`): what `add` appends
/// follows each file's own line endings, and the user's lines are untouched.
/// On 0.10.1 a CRLF file came back with LF lines appended.
#[test]
fn appended_lines_follow_each_files_line_endings() {
    let p = Project::new().example("A=1\r\nB=2\r\n").env("A=x\nB=y\n");
    run(p.path(), &["add", "service", "redis", "--yes"]).assert_code(0);

    let example = String::from_utf8(std::fs::read(p.join(".env.example")).unwrap()).unwrap();
    assert!(example.starts_with("A=1\r\nB=2\r\n"), "{example:?}");
    assert!(
        !example.replace("\r\n", "").contains('\n'),
        "LF in a CRLF file: {example:?}"
    );
    assert!(example.contains("\r\nREDIS_URL="), "{example:?}");

    let env = String::from_utf8(std::fs::read(p.join(".env")).unwrap()).unwrap();
    assert!(!env.contains('\r'), "CRLF in an LF file: {env:?}");
}

/// BR-1 (fixed in 0.10.2, AD-H, DQ-A8; was `known_br1_*`): `add service`
/// reads the project's framework the way `init --detect` does and adds the
/// browser copies of the public values, as `init --with vite,supabase` does.
#[test]
fn service_adds_browser_copies_for_the_detected_framework() {
    for (package, prefix) in [
        (r#"{"devDependencies":{"vite":"5"}}"#, "VITE_"),
        (r#"{"dependencies":{"next":"16"}}"#, "NEXT_PUBLIC_"),
    ] {
        let p = Project::new()
            .file("package.json", package)
            .example("APP=1\n");
        run(p.path(), &["add", "service", "supabase", "--yes"])
            .assert_code(0)
            .assert_contains(&format!("adding 2 {prefix} copies for the browser"));
        let example = p.read(".env.example");
        for name in ["SUPABASE_URL", "SUPABASE_ANON_KEY"] {
            assert!(example.contains(&format!("\n{prefix}{name}=")), "{example}");
        }
        // Secrets never get a browser copy.
        assert!(!example.contains(&format!("{prefix}SUPABASE_SERVICE_ROLE_KEY")));
    }
}

/// BR-1: the copies are the ones `init --with` writes, line for line.
#[test]
fn service_copies_match_init() {
    let via_add = Project::new()
        .file("package.json", r#"{"devDependencies":{"vite":"5"}}"#)
        .example("APP=1\n");
    run(via_add.path(), &["add", "service", "supabase", "--yes"]).assert_code(0);
    let via_init = Project::new();
    run(
        via_init.path(),
        &["init", "--with", "vite,supabase", "--yes"],
    )
    .assert_code(0);
    let block = |text: &str| -> Vec<String> {
        let lines: Vec<&str> = text.lines().collect();
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with("VITE_SUPABASE_"))
            .flat_map(|(i, l)| [lines[i - 1].to_string(), l.to_string()])
            .collect()
    };
    let added = block(&via_add.read(".env.example"));
    assert_eq!(
        added.len(),
        4,
        "two copies, each with its comment: {added:?}"
    );
    assert_eq!(added, block(&via_init.read(".env.example")));
}

/// BR-1: no browser framework, no copies, and with `-v` one line saying so
/// (N-22: only with `-v` since 0.10.4); a service with no public values says
/// nothing at all.
#[test]
fn no_browser_framework_no_copies() {
    let p = Project::new().example("APP=1\n");
    run(p.path(), &["-v", "add", "service", "supabase", "--yes"])
        .assert_code(0)
        .assert_contains("No browser framework found in this project");
    assert!(!p.read(".env.example").contains("VITE_"));
    run(p.path(), &["add", "service", "postgresql", "--yes"])
        .assert_code(0)
        .assert_not_contains("browser");
}

/// N-11 in `init --from-source` (fixed in 0.10.2, AD-S): it appends through
/// `add`'s planner, so a name the user already set in `.env` is not appended
/// there again below the real value.
#[test]
fn init_from_source_keeps_the_real_env_value() {
    let p = Project::new()
        .file("src/app.ts", "const u = process.env.REDIS_URL;\n")
        .example("APP=1\n")
        .env(&format!("APP=1\n{REAL_REDIS}\n"))
        .file(".gitignore", ".env\n");
    run(p.path(), &["init", "--from-source", "--yes"]).assert_code(0);
    assert_eq!(p.count_lines(".env.example", "REDIS_URL="), 1);
    assert_eq!(p.count_lines(".env", "REDIS_URL="), 1);
    assert_eq!(
        effective(&p.read(".env"), "REDIS_URL").as_deref(),
        Some(REAL_REDIS)
    );
}

/// Flags `add` owns go before or after the subcommand, as they always have
/// (`-p` and `--yes` are global to it). Found while turning the training demo
/// into tests: a clap setting added for `--list` refused the first form.
#[test]
fn add_flags_work_before_the_subcommand() {
    let p = Project::new().dir("api");
    run(
        p.path(),
        &["add", "--path", "api", "--yes", "service", "redis"],
    )
    .assert_code(0);
    run(
        p.path(),
        &["add", "-p", "api", "service", "stripe", "--yes"],
    )
    .assert_code(0);
    run(p.path(), &["add", "--dry-run", "service", "sentry"]).assert_code(0);
    assert_eq!(p.count_lines("api/.env.example", "REDIS_URL="), 1);
    assert_eq!(p.count_lines("api/.env.example", "STRIPE_SECRET_KEY="), 1);
    assert_eq!(p.count_lines("api/.env.example", "SENTRY_DSN="), 0);
    run(p.path(), &["add", "--list", "service", "redis"])
        .assert_code(2)
        .assert_contains("--list takes no subcommand");
}

// ─────────────────────────────────────────────────────────────
// 0.10.4 (R-add)
// ─────────────────────────────────────────────────────────────

/// N-21: Django's `DATABASE_URL` had no default of its own, so the resolver's
/// URL-shaped fallback (`https://example.com`) was written, which no database
/// accepts. It is now Django's own out-of-the-box database (owner decision
/// 2026-10-17: SQLite).
#[test]
fn t_n21_django_database_url_is_sqlite() {
    let p = Project::new();
    run(p.path(), &["add", "framework", "django", "--yes"]).assert_code(0);
    let example = p.read(".env.example");
    assert!(
        example.contains("\nDATABASE_URL=sqlite:///db.sqlite3\n"),
        "{example}"
    );
    assert!(
        !example.contains("DATABASE_URL=https://example.com"),
        "{example}"
    );
}

/// N-22: in a project with no browser framework, `add service` with a public
/// value said "No browser framework found" on every run. It now says so only
/// with `-v` (owner decision 2026-10-17); when a framework is found, the line
/// about the copies is unchanged.
#[test]
fn t_n22_no_framework_line_only_with_verbose() {
    let p = Project::new();
    run(p.path(), &["add", "service", "stripe", "--yes"])
        .assert_code(0)
        .assert_not_contains("No browser framework found");
    let q = Project::new();
    run(q.path(), &["-v", "add", "service", "stripe", "--yes"])
        .assert_code(0)
        .assert_contains("No browser framework found");
    let v = Project::new().file("package.json", r#"{"devDependencies":{"vite":"5"}}"#);
    run(v.path(), &["add", "service", "supabase", "--yes"])
        .assert_code(0)
        .assert_contains("adding 2 VITE_ copies for the browser");
}

/// I-250: in a stack with PostgreSQL, `DATABASE_URL` is PostgreSQL's, not the
/// framework's. The resolver keeps the first value it sees and adds the
/// framework first, so `fastapi_ai`, `rust_high_perf` and `django_enterprise`
/// wrote `https://example.com` (and, with N-21, `django_enterprise` would have
/// written SQLite). A service's own default now wins, in either order.
#[test]
fn t_i250_postgres_stacks_use_the_postgres_url() {
    const PG: &str = "DATABASE_URL=postgresql://user:password@localhost:5432/dbname\n";
    for id in ["fastapi_ai", "rust_high_perf", "django_enterprise"] {
        let p = Project::new();
        run(p.path(), &["add", "blueprint", id, "--yes"]).assert_code(0);
        assert!(p.read(".env.example").contains(PG), "add blueprint {id}");
        let q = Project::new();
        run(q.path(), &["init", "--blueprint", id, "--yes"]).assert_code(0);
        assert!(q.read(".env.example").contains(PG), "init --blueprint {id}");
    }
    for with in ["django,postgresql", "postgresql,django"] {
        let p = Project::new();
        run(p.path(), &["init", "--with", with, "--yes"]).assert_code(0);
        assert!(p.read(".env.example").contains(PG), "init --with {with}");
    }
    // Django on its own keeps its own default (N-21).
    let p = Project::new();
    run(p.path(), &["init", "--with", "django", "--yes"]).assert_code(0);
    assert!(p
        .read(".env.example")
        .contains("DATABASE_URL=sqlite:///db.sqlite3\n"));
}

/// I-251: `add custom` into a folder with no `.env` creates one (0600) holding
/// the values it was given. `.gitignore` comes first, as for every other flow
/// (DQ-A4), and the dry run says what it will do.
#[test]
fn t_i251_custom_protects_the_env_it_creates() {
    let p = Project::new().git();
    run(p.path(), &["add", "custom", "MY_KEY=value", "--dry-run"])
        .assert_code(0)
        .assert_contains(".env: would be created with 1 variable (with .gitignore first): MY_KEY");
    assert!(!p.exists(".gitignore"));
    run(p.path(), &["add", "custom", "MY_KEY=value", "--yes"])
        .assert_code(0)
        .assert_contains("Created .gitignore")
        .assert_not_contains("is not in .gitignore");
    assert!(p.read(".env").contains("MY_KEY=value"));
    assert!(p.read(".gitignore").contains(".env*"));
    assert!(!p.git_cmd(&["status", "--porcelain"]).contains(".env\n"));
}
