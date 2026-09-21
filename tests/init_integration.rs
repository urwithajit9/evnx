// tests/init_integration.rs - Add at top

//! Integration tests for `evnx init` command.
//!
//! # `--yes` is Blank; a blueprint is named
//!
//! These tests used to drive blueprints through `--yes` and could therefore
//! assert almost nothing — the blueprint list came out of a `HashMap`, so the
//! stack chosen changed on every run, and every assertion here had been widened
//! until it passed for *any* of them ("typically supabase_fullstack").
//!
//! `--yes` now means Blank, and `--blueprint <ID>` names a stack explicitly, so
//! these can assert what they were always trying to: that `t3_modern` produces
//! Next.js variables and `rust_high_perf` produces Rust ones.
//!
//! Architect mode remains interactive-only and its tests stay `#[ignore]`d.

#![allow(deprecated)]
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

// fn debug_print_content(label: &str, content: &str) {
//     eprintln!("\n=== {} ===", label);
//     eprintln!("{}", content);
//     eprintln!("=== END {} ===\n", label);
// }

fn read_env_example(dir: &std::path::Path) -> anyhow::Result<String> {
    std::fs::read_to_string(dir.join(".env.example")).map_err(|e| anyhow::anyhow!("{}", e))
}

fn count_env_vars(content: &str) -> usize {
    content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && t.contains('=')
        })
        .count()
}

// ─────────────────────────────────────────────────────────────
// Blank Mode Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn init_blank_creates_minimal_files() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created empty .env.example"));

    assert!(
        dir.path().join(".env.example").exists(),
        ".env.example should exist"
    );
    assert!(dir.path().join(".env").exists(), ".env should exist");
    assert!(
        dir.path().join(".gitignore").exists(),
        ".gitignore should exist"
    );

    // Blank means blank: `--yes` must not invent somebody else's stack.
    let example = read_env_example(dir.path()).unwrap();
    assert_eq!(
        count_env_vars(&example),
        0,
        "--yes must produce an empty scaffold, not a guessed stack:\n{example}"
    );
}

/// The regression that `--blueprint` exists to prevent.
///
/// `evnx init --yes` in one empty directory produced Laravel, Next.js, Laravel,
/// Go, Rust and MERN across six runs, because the blueprint list was a `HashMap`
/// and the non-interactive path took its first entry.
#[test]
fn init_yes_is_deterministic() {
    let mut digests = Vec::new();

    for _ in 0..4 {
        let dir = TempDir::new().unwrap();
        Command::cargo_bin("evnx")
            .unwrap()
            .arg("init")
            .arg("--yes")
            .arg("--path")
            .arg(dir.path())
            .assert()
            .success();
        digests.push(read_env_example(dir.path()).unwrap());
    }

    assert!(
        digests.windows(2).all(|w| w[0] == w[1]),
        "`evnx init --yes` must produce identical output every run"
    );
}

/// Same guarantee for the named-blueprint path.
#[test]
fn init_blueprint_is_deterministic() {
    let mut outputs = Vec::new();

    for _ in 0..4 {
        let dir = TempDir::new().unwrap();
        Command::cargo_bin("evnx")
            .unwrap()
            .args(["init", "--yes", "--blueprint", "t3_modern", "--path"])
            .arg(dir.path())
            .assert()
            .success();
        outputs.push(read_env_example(dir.path()).unwrap());
    }

    assert!(
        outputs.windows(2).all(|w| w[0] == w[1]),
        "`--blueprint t3_modern` must produce identical output every run"
    );
}

#[test]
fn init_unknown_blueprint_lists_the_real_ones() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--blueprint", "no_such_stack", "--path"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Unknown blueprint 'no_such_stack'",
        ))
        .stderr(predicate::str::contains("t3_modern"))
        .stderr(predicate::str::contains("rust_high_perf"));

    assert!(
        !dir.path().join(".env.example").exists(),
        "nothing should be written when the blueprint is unknown"
    );
}

#[test]
fn init_yes_refuses_to_overwrite_an_existing_example() {
    let dir = TempDir::new().unwrap();
    let example = dir.path().join(".env.example");
    std::fs::write(&example, "MY_CAREFULLY_WRITTEN_TEMPLATE=1\n").unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    assert_eq!(
        std::fs::read_to_string(&example).unwrap(),
        "MY_CAREFULLY_WRITTEN_TEMPLATE=1\n",
        "the existing template must survive"
    );
}

#[test]
fn init_force_replaces_the_example_but_never_the_env() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".env.example"), "OLD=1\n").unwrap();
    std::fs::write(dir.path().join(".env"), "REAL_SECRET=keepme\n").unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--force", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    assert!(!read_env_example(dir.path()).unwrap().contains("OLD=1"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "REAL_SECRET=keepme\n",
        ".env holds real values and must never be replaced"
    );
}

// ─────────────────────────────────────────────────────────────
// Blueprint Mode Tests
// ─────────────────────────────────────────────────────────────

// tests/init_integration.rs

#[test]
fn init_blueprint_t3_modern_generates_expected_vars() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--blueprint", "t3_modern", "--path"])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created .env.example"));

    let example = read_env_example(dir.path()).unwrap();

    // Now that the stack is named, assert the stack — not "any blueprint".
    for var in [
        "NEXTAUTH_URL",
        "NEXTAUTH_SECRET",
        "NEXT_PUBLIC_APP_URL",
        "DATABASE_URL",
        "CLERK_SECRET_KEY",
    ] {
        assert!(
            example.contains(var),
            "t3_modern must define {var}:\n{example}"
        );
    }

    assert!(
        example.contains("# ──"),
        "Should have section headers like '# ── Category ──'"
    );
    assert!(
        example.contains("# Generated by evnx v"),
        "Should have generation footer"
    );

    let var_count = count_env_vars(&example);
    assert!(
        (5..=50).contains(&var_count),
        "Should have 5-50 variables, got {var_count}"
    );
}

#[test]
fn init_blueprint_rust_high_perf() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--blueprint", "rust_high_perf", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    for var in ["RUST_LOG", "SOCKET_ADDR", "DATABASE_URL", "REDIS_URL"] {
        assert!(
            example.contains(var),
            "rust_high_perf must define {var}:\n{example}"
        );
    }
    // And must NOT look like the Next.js stack — the assertion the old,
    // blueprint-is-random version of this test could never make.
    assert!(
        !example.contains("NEXTAUTH_URL"),
        "rust_high_perf must not carry Next.js variables"
    );

    assert!(example.contains("# ──"), "Should have section headers");
    assert!(count_env_vars(&example) >= 5);
}

/// Was an "architect" test driven through `--yes`, which never reached Architect
/// mode. Recast as what it actually measured: a blueprint that pulls in several
/// services at once.
#[test]
fn init_blueprint_with_many_services() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--blueprint", "go_microservice", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    for var in ["GIN_MODE", "DATABASE_URL", "KAFKA_BROKERS"] {
        assert!(
            example.contains(var),
            "go_microservice must define {var}:\n{example}"
        );
    }
    assert!(example.contains("# ──"), "Should have section markers");
    assert!(count_env_vars(&example) >= 5);
}

#[test]
#[ignore = "requires interactive TTY - run manually to test specific blueprint"]
fn init_blueprint_specific_t3_modern() {
    let dir = TempDir::new().unwrap();

    // This only works in a real terminal:
    // 1 = Blueprint mode, then select t3_modern by index
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("1\n0\ny\n") // Blueprint, first option (t3_modern), confirm
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    // Now we can check for T3-specific vars
    assert!(example.contains("NEXTAUTH_SECRET=") || example.contains("NEXT_PUBLIC_"));
    assert!(example.contains("CLERK_") || example.contains("DATABASE_URL="));
}

// ─────────────────────────────────────────────────────────────
// Architect Mode Tests
// ─────────────────────────────────────────────────────────────

/// Also an "architect" test driven through `--yes`. Recast as the Django stack
/// it was describing, which is now selectable by name.
#[test]
fn init_blueprint_django_enterprise() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args([
            "init",
            "--yes",
            "--blueprint",
            "django_enterprise",
            "--path",
        ])
        .arg(dir.path())
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();

    for var in [
        "DJANGO_SETTINGS_MODULE",
        "SECRET_KEY",
        "ALLOWED_HOSTS",
        "DATABASE_URL",
    ] {
        assert!(
            example.contains(var),
            "django_enterprise must define {var}:\n{example}"
        );
    }
    assert!(
        example.contains("# ──") || example.contains("# [ADDED]"),
        "Should have section headers"
    );
}

#[test]
#[ignore = "requires interactive TTY - run manually"]
fn init_architect_interactive() {
    let dir = TempDir::new().unwrap();

    // This would work in a real terminal
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("2\n0\n0\n0\n\n") // Architect, Python, Django, PostgreSQL, no infra, confirm
        .assert()
        .success();

    let example = read_env_example(dir.path()).unwrap();
    assert!(
        example.contains("SECRET_KEY="),
        "Should have Django SECRET_KEY"
    );
    assert!(
        example.contains("DATABASE_URL="),
        "Should have DATABASE_URL"
    );
}

// ─────────────────────────────────────────────────────────────
// File Operations Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn init_creates_nested_directory() {
    let dir = TempDir::new().unwrap();
    let nested_path = dir.path().join("deep").join("nested").join("project");

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(&nested_path)
        .write_stdin("0\n") // Blank mode
        .assert()
        .success();

    // Verify directory was created
    assert!(nested_path.exists(), "Nested directory should be created");
    assert!(
        nested_path.join(".env.example").exists(),
        ".env.example should exist in nested path"
    );
}

#[test]
fn init_updates_gitignore() {
    let dir = TempDir::new().unwrap();

    // Pre-create .gitignore with some content
    std::fs::write(dir.path().join(".gitignore"), "# My project\n*.log\n").unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("0\n") // Blank mode
        .assert()
        .success();

    // Verify .gitignore was updated, not replaced
    let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(
        gitignore.contains("*.log"),
        "Original content should be preserved"
    );
    assert!(
        gitignore.contains("# My project"),
        "The user's own comments should be preserved"
    );
    // Was `.env` + `.env.local`; now one pattern covering every env file, which
    // is what stops `.env.production` being committable.
    assert!(
        gitignore.lines().any(|l| l.trim() == ".env*"),
        "Should add the .env* rule:\n{gitignore}"
    );
    assert!(
        gitignore.lines().any(|l| l.trim() == "!.env.example"),
        "Should keep .env.example committable:\n{gitignore}"
    );
}

#[test]
fn init_does_not_overwrite_existing_env() {
    let dir = TempDir::new().unwrap();

    // Pre-create .env with custom content
    let original_env = "MY_CUSTOM_VAR=keep_this\n";
    std::fs::write(dir.path().join(".env"), original_env).unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("0\n") // Blank mode
        .assert()
        .success();

    // Verify .env was NOT overwritten
    let env_content = std::fs::read_to_string(dir.path().join(".env")).unwrap();
    assert!(
        env_content.contains("MY_CUSTOM_VAR=keep_this"),
        "Existing .env should not be overwritten"
    );
}

// ─────────────────────────────────────────────────────────────
// Interactive Mode Simulation Tests
// ─────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires interactive TTY - run manually"]
fn init_interactive_mode_selection() {
    let dir = TempDir::new().unwrap();

    // Simulate interactive selection: choose Blueprint, then first blueprint
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("1\n0\ny\n") // Blueprint mode, first blueprint, confirm
        .assert()
        .success()
        .stdout(predicate::str::contains("Preview:"))
        .stdout(predicate::str::contains("Generate .env files"));

    assert!(dir.path().join(".env.example").exists());
}

#[test]
#[ignore = "requires interactive TTY - run manually"]
fn init_interactive_abort() {
    let dir = TempDir::new().unwrap();

    // Simulate aborting at confirmation
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("1\n0\nn\n") // Blueprint, first blueprint, NO to confirm
        .assert()
        .success()
        .stdout(predicate::str::contains("Aborted"));

    // Verify no files were created
    assert!(
        !dir.path().join(".env.example").exists(),
        "Should not create files when aborted"
    );
}

/// `init` used to write `.env`, `.env.local`, `.env.*.local` — the Next.js
/// convention, which assumes `.env.production` holds non-secret defaults. For a
/// secrets tool that assumption is backwards, and it left `.env.production`,
/// `.env.staging` and `.env.test` committable.
#[test]
fn init_gitignores_every_env_file_but_not_the_template() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    let rules: Vec<&str> = gitignore.lines().map(str::trim).collect();

    assert!(rules.contains(&".env*"), "{gitignore}");
    for exempt in ["!.env.example", "!.env.sample", "!.env.template"] {
        assert!(rules.contains(&exempt), "missing {exempt}:\n{gitignore}");
    }

    // Order is load-bearing: a negation before the pattern it carves out of
    // does nothing in gitignore.
    let glob = rules.iter().position(|r| *r == ".env*").unwrap();
    let negation = rules.iter().position(|r| *r == "!.env.example").unwrap();
    assert!(
        glob < negation,
        "the negation must follow the pattern:\n{gitignore}"
    );
}

// ─────────────────────────────────────────────────────────────
// --with: naming the components directly
// ─────────────────────────────────────────────────────────────

/// ⚠️ The claim that makes `--with` a refactor and not a second code path: a
/// blueprint is a component list, so both routes must produce the same file
/// byte for byte. If this ever fails, blueprints have stopped being aliases.
#[test]
fn a_blueprint_and_its_component_list_generate_the_same_file() {
    let by_blueprint = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(by_blueprint.path())
        .args(["init", "--blueprint", "t3_modern", "--yes"])
        .assert()
        .success();

    let by_components = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(by_components.path())
        .args([
            "init",
            "--with",
            "nextjs,postgresql,clerk,aws_s3,stripe,github_actions,vercel",
            "--yes",
        ])
        .assert()
        .success();

    let a = std::fs::read_to_string(by_blueprint.path().join(".env.example")).unwrap();
    let b = std::fs::read_to_string(by_components.path().join(".env.example")).unwrap();
    assert_eq!(a, b, "t3_modern and its component list diverge");
    assert!(a.len() > 200, "suspiciously small output: {a}");
}

/// The combination no blueprint expresses, which is why `--with` exists.
#[test]
fn components_no_blueprint_offers_can_be_combined() {
    let d = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(d.path())
        .args(["init", "--with", "django,kafka,clerk", "--yes"])
        .assert()
        .success();

    let example = std::fs::read_to_string(d.path().join(".env.example")).unwrap();
    assert!(example.contains("KAFKA"), "{example}");
    assert!(example.contains("CLERK"), "{example}");
}

/// Repeatable as well as comma-separated.
#[test]
fn with_accepts_repeats_and_commas_alike() {
    let combined = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(combined.path())
        .args(["init", "--with", "postgresql,redis", "--yes"])
        .assert()
        .success();

    let repeated = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(repeated.path())
        .args(["init", "--with", "postgresql", "--with", "redis", "--yes"])
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(combined.path().join(".env.example")).unwrap(),
        std::fs::read_to_string(repeated.path().join(".env.example")).unwrap()
    );
}

/// ⚠️ An unknown name must fail *before* anything is written — not after a
/// preview has implied it worked.
#[test]
fn an_unknown_component_writes_nothing() {
    let d = TempDir::new().unwrap();

    let assert = Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(d.path())
        .args(["init", "--with", "postgresql,nope", "--yes"])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("unknown component 'nope'"), "{stderr}");
    assert!(
        !d.path().join(".env.example").exists(),
        "a refused run must leave no .env.example behind"
    );
}

/// The error points at this, so it has to work with no project and no prompt.
#[test]
fn list_components_needs_no_project() {
    let d = TempDir::new().unwrap();
    let assert = Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(d.path())
        .args(["init", "--list-components"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    for expected in ["nextjs", "postgresql", "stripe", "FRAMEWORK", "SERVICE"] {
        assert!(
            stdout.contains(expected),
            "{expected} missing from:\n{stdout}"
        );
    }
    assert!(
        !d.path().join(".env.example").exists(),
        "listing must not touch the project"
    );
}

/// `--with` and `--blueprint` answer the same question two ways; giving both is
/// an error rather than a silent preference.
#[test]
fn with_and_blueprint_conflict() {
    let d = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .current_dir(d.path())
        .args([
            "init",
            "--with",
            "nextjs",
            "--blueprint",
            "t3_modern",
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

// ─────────────────────────────────────────────────────────────
// Detection: propose what the project already declares
// ─────────────────────────────────────────────────────────────

/// A project whose stack is fully described on disk — which is the case `init`
/// used to ask about anyway.
fn detectable_project() -> TempDir {
    let d = TempDir::new().unwrap();
    std::fs::write(
        d.path().join("package.json"),
        r#"{"dependencies":{"next":"16","stripe":"14","@clerk/nextjs":"5"}}"#,
    )
    .unwrap();
    std::fs::write(
        d.path().join("docker-compose.yml"),
        "services:\n  postgres:\n    image: postgres:16\nvolumes:\n  postgres_data:\n",
    )
    .unwrap();
    d
}

#[test]
fn detect_reads_the_project_and_writes_its_variables() {
    let d = detectable_project();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--detect", "--path"])
        .arg(d.path())
        .assert()
        .success();

    let example = std::fs::read_to_string(d.path().join(".env.example")).unwrap();
    assert!(example.contains("DATABASE_URL"), "{example}");
    assert!(example.contains("STRIPE"), "{example}");
    assert!(example.contains("CLERK"), "{example}");
}

/// ⚠️ Detection is a proposal, so accepting it without a human present has to be
/// asked for. Before this, `dialoguer` failed with "IO error: not a terminal",
/// which says nothing about what to do instead.
#[test]
fn a_non_tty_is_told_how_to_proceed_rather_than_failing_obscurely() {
    let d = detectable_project();

    let assert = Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--path"])
        .arg(d.path())
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("--detect"), "{stderr}");
    assert!(stderr.contains("--with"), "{stderr}");
    assert!(!stderr.contains("not a terminal"), "{stderr}");
    assert!(
        !d.path().join(".env.example").exists(),
        "nothing should be written when the proposal was never accepted"
    );
}

/// Detection only chooses the component list. Everything after that is the same
/// path `--with` takes, so the two must agree.
#[test]
fn detected_output_equals_naming_the_same_components() {
    let detected = detectable_project();
    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--detect", "--path"])
        .arg(detected.path())
        .assert()
        .success();

    let named = TempDir::new().unwrap();
    Command::cargo_bin("evnx")
        .unwrap()
        .args([
            "init",
            "--with",
            "nextjs,stripe,clerk,postgresql",
            "--yes",
            "--path",
        ])
        .arg(named.path())
        .assert()
        .success();

    let strip = |p: &std::path::Path| {
        std::fs::read_to_string(p.join(".env.example"))
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("# Generated by"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(strip(detected.path()), strip(named.path()));
}

/// ⚠️ An explicit request wins. Proposing something over the top of what the
/// user already named would be worse than not detecting at all.
#[test]
fn an_explicit_request_suppresses_detection() {
    let d = detectable_project();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--with", "redis", "--yes", "--path"])
        .arg(d.path())
        .assert()
        .success();

    let example = std::fs::read_to_string(d.path().join(".env.example")).unwrap();
    assert!(example.contains("REDIS"), "{example}");
    assert!(
        !example.contains("STRIPE"),
        "detection should not have added Stripe over an explicit --with:\n{example}"
    );
}

/// A project evnx cannot read is no worse off than before: `--yes` still writes
/// blank files.
#[test]
fn an_undetectable_project_still_gets_the_blank_path() {
    let d = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .args(["init", "--yes", "--path"])
        .arg(d.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created empty .env.example"));
}
