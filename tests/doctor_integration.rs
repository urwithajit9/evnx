// Add this import at the top of your test file
use assert_cmd::cargo::cargo_bin_cmd; // ← NEW IMPORT

// use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_doctor_json_output() {
    let dir = TempDir::new().unwrap();

    fs::write(dir.path().join(".env.example"), "FOO=bar\n").unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();

    // Replace Command::cargo_bin() with cargo_bin_cmd!()
    cargo_bin_cmd!("evnx") // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .env("EVNX_OUTPUT_JSON", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""summary""#))
        .stdout(predicate::str::contains(r#""checks""#));
}

#[test]
fn test_doctor_verbose_mode() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();

    cargo_bin_cmd!("evnx") // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .arg("--verbose")
        .assert()
        .success()
        .stdout(predicate::str::contains("Project path:"));
}

#[test]
fn test_doctor_missing_env_warning() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx") // ← UPDATED
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Warning").or(predicate::str::contains("⚠️")));
}

/// ⚠️ The regression this exists to prevent.
///
/// `doctor` checked `.env` and nothing else, so a project whose `.gitignore`
/// carried only `.env` — which is what older `evnx init` wrote — got a green
/// tick while `.env.production` sat committable. Reporting healthy over a real
/// exposure is worse than reporting nothing.
#[test]
fn doctor_flags_every_unignored_env_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
    fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    fs::write(
        dir.path().join(".env.production"),
        "DATABASE_URL=postgres://prod\n",
    )
    .unwrap();
    fs::write(dir.path().join(".env.test"), "B=2\n").unwrap();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            ".env.production is NOT in .gitignore",
        ))
        .stdout(predicate::str::contains(".env.test is NOT in .gitignore"));
}

#[test]
fn doctor_does_not_flag_the_template_family() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
    fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    fs::write(dir.path().join(".env.example"), "A=\n").unwrap();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .stdout(predicate::str::contains(".env.example is NOT in .gitignore").not());
}

#[test]
fn auto_fix_covers_every_env_file_and_keeps_the_example_committable() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".gitignore"), ".env\n").unwrap();
    fs::write(dir.path().join(".env"), "A=1\n").unwrap();
    fs::write(dir.path().join(".env.production"), "DATABASE_URL=x\n").unwrap();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(dir.path())
        .env("EVNX_AUTO_FIX", "1")
        .assert();

    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|l| l.trim() == ".env*"),
        "{gitignore}"
    );
    assert!(
        gitignore.lines().any(|l| l.trim() == "!.env.example"),
        "the template must stay committable:\n{gitignore}"
    );

    // And the exposure is gone.
    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("NOT in .gitignore").not());
}
