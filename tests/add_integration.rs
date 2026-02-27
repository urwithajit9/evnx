//! Integration tests for `evnx add` command.
#![allow(deprecated)]
mod common;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

use common::{fixtures, read_env_example};
// use common::assertions::*;

// ─────────────────────────────────────────────────────────────
// Add Service Tests
// ─────────────────────────────────────────────────────────────

// #[test]
// fn add_service_postgresql_appends_vars() {
//     let dir = TempDir::new().unwrap();
//     fixtures::setup_minimal_project(dir.path()).unwrap();

//     Command::cargo_bin("evnx")
//         .unwrap()
//         .arg("add")
//         .arg("service")
//         .arg("postgresql")
//         .arg("--path")
//         .arg(dir.path())
//         .arg("--yes")
//         .assert()
//         .success()
//         .stdout(predicate::str::contains("Appended"))
//         .stdout(predicate::str::contains("postgresql"));

//     let example = read_env_example(dir.path()).unwrap();

//     // Should have PostgreSQL vars
//     for var in fixtures::POSTGRES_VARS {
//         assert!(example.contains(&format!("{}=", var)),
//                 "Should have {}", var);
//     }

//     // Should have section marker
//     assert!(example.contains("# [ADDED] Database"),
//             "Should have [ADDED] section marker");

//     // Original content should be preserved
//     assert!(example.contains("APP_NAME=test"),
//             "Original APP_NAME should be preserved");
// }

#[test]
fn add_service_unknown_returns_error() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
        .arg("add")
        .arg("service")
        .arg("nonexistent_service_xyz")
        .arg("--path")
        .arg(dir.path())
        .arg("--yes")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"));
}

// #[test]
// fn add_service_multiple_times_deduplicates() {
//     let dir = TempDir::new().unwrap();
//     fixtures::setup_minimal_project(dir.path()).unwrap();

//     // Add PostgreSQL twice
//     for _ in 0..2 {
//         Command::cargo_bin("evnx")
//             .unwrap()
//             .arg("add")
//             .arg("service")
//             .arg("postgresql")
//             .arg("--path")
//             .arg(dir.path())
//             .arg("--yes")
//             .assert()
//             .success();
//     }

//     let example = read_env_example(dir.path()).unwrap();

//     // DATABASE_URL should appear only once (deduplication)
//     let db_url_count = example.lines()
//         .filter(|line| line.trim().starts_with("DATABASE_URL="))
//         .count();
//     assert_eq!(db_url_count, 1, "DATABASE_URL should appear exactly once after duplicate adds");
// }

// ─────────────────────────────────────────────────────────────
// Add Framework Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn add_framework_django_appends_vars() {
    let dir = TempDir::new().unwrap();
    fixtures::setup_minimal_project(dir.path()).unwrap();

    Command::cargo_bin("evnx")
        .unwrap()
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
    for var in fixtures::DJANGO_VARS {
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

    Command::cargo_bin("evnx")
        .unwrap()
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
    fixtures::setup_postgres_project(dir.path()).unwrap();

    // Add T3 blueprint which includes PostgreSQL (has DATABASE_URL)
    Command::cargo_bin("evnx")
        .unwrap()
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

    Command::cargo_bin("evnx")
        .unwrap()
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
// ─────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires TTY; run manually with `cargo test -- --ignored`"]
fn add_custom_interactive() {
    let dir = TempDir::new().unwrap();
    fixtures::setup_minimal_project(dir.path()).unwrap();

    // Simulate interactive custom addition
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("add")
        .arg("custom")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("MY_API_KEY\nsk_test_xxx\nn\nn\nn\nn\n\n") // name, value, no desc, no cat, not required, no more, finish
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"))
        .stdout(predicate::str::contains("custom variables"));

    let example = read_env_example(dir.path()).unwrap();

    // Should have custom var
    assert!(
        example.contains("MY_API_KEY=sk_test_xxx"),
        "Should have custom variable"
    );

    // Should have custom section marker
    assert!(
        example.contains("# ── Custom Variables ──"),
        "Should have Custom Variables section"
    );
}

// ─────────────────────────────────────────────────────────────
// Combined Workflow Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn workflow_init_then_add_service() {
    let dir = TempDir::new().unwrap();

    // Step 1: Init with Blank mode
    Command::cargo_bin("evnx")
        .unwrap()
        .arg("init")
        .arg("--yes")
        .arg("--path")
        .arg(dir.path())
        .write_stdin("0\n") // Blank
        .assert()
        .success();

    // Step 2: Add PostgreSQL service
    Command::cargo_bin("evnx")
        .unwrap()
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
    assert!(
        example.contains("# [ADDED] Database"),
        "Should have [ADDED] marker"
    );
}

// #[test]
// fn workflow_add_multiple_services() {
//     let dir = TempDir::new().unwrap();
//     fixtures::setup_minimal_project(dir.path()).unwrap();

//     // Add multiple services in sequence
//     for service in ["redis", "stripe", "sentry"] {
//         Command::cargo_bin("evnx")
//             .unwrap()
//             .arg("add")
//             .arg("service")
//             .arg(service)
//             .arg("--path")
//             .arg(dir.path())
//             .arg("--yes")
//             .assert()
//             .success();
//     }

//     let example = read_env_example(dir.path()).unwrap();

//     // Should have vars from all services
//     assert!(example.contains("REDIS_URL="), "Should have Redis");
//     assert!(example.contains("STRIPE_SECRET_KEY="), "Should have Stripe");
//     assert!(example.contains("SENTRY_DSN="), "Should have Sentry");

//     // Should be organized by category
//     assert!(example.contains("# [ADDED] Cache"), "Should have Cache section");
//     assert!(example.contains("# [ADDED] Payments"), "Should have Payments section");
//     assert!(example.contains("# [ADDED] Monitoring"), "Should have Monitoring section");
// }
