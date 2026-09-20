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
    assert!(gitignore.contains(".env\n"), "Should add .env entry");
    assert!(
        gitignore.contains(".env.local"),
        "Should add .env.local entry"
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
