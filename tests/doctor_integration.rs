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

/// ⚠️ This asserted on the warning *glyph* — `contains("Warning").or(contains("⚠️"))`
/// — and `⚠️` is presentation. It broke the moment the glyph set changed width,
/// which is the only assertion in the suite that did.
///
/// It now asserts what the command is actually for: a project with no `.env`
/// gets told so, by name.
#[test]
fn test_doctor_missing_env_warning() {
    let dir = TempDir::new().unwrap();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".env"));
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

// ─────────────────────────────────────────────────────────────
// --fix, --path, --strict, and the 0/1/2 exit contract
// ─────────────────────────────────────────────────────────────

/// A project whose `.env` is neither gitignored nor mode 0600 — two errors that
/// `--fix` can genuinely repair.
fn unhealthy_project() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".env"), "AWS=AKIA4OZRMFJ3VREALKEY\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(d.path().join(".env")).unwrap().permissions();
        p.set_mode(0o644);
        fs::set_permissions(d.path().join(".env"), p).unwrap();
    }
    d
}

/// ⚠️ The capability existed behind `EVNX_AUTO_FIX=1` and no flag reached it, so
/// the docs kept inventing one — `--fix`, `--check-config`, `--fail-on-warning`,
/// `--check-example-only` all appeared in published guides.
#[test]
fn fix_repairs_what_it_reports() {
    let d = unhealthy_project();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .assert()
        .code(1);

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .arg("--fix")
        .assert()
        .success();

    let gitignore = fs::read_to_string(d.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".env*"), "{gitignore}");
    assert!(gitignore.contains("!.env.example"), "{gitignore}");

    // And the project is now healthy without --fix.
    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .assert()
        .success();
}

/// `EVNX_AUTO_FIX=1` shipped first and is documented; the flag does not replace it.
#[test]
fn the_environment_variable_still_works() {
    let d = unhealthy_project();

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .env("EVNX_AUTO_FIX", "1")
        .assert()
        .success();

    assert!(fs::read_to_string(d.path().join(".gitignore"))
        .unwrap()
        .contains(".env*"));
}

/// Warnings are reported and exit 0; `--strict` is what makes them count.
#[test]
fn strict_is_what_makes_a_warning_fail() {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".env"), "A=1\n").unwrap();
    fs::write(d.path().join(".gitignore"), ".env*\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(d.path().join(".env")).unwrap().permissions();
        p.set_mode(0o600);
        fs::set_permissions(d.path().join(".env"), p).unwrap();
    }

    // No .env.example — a warning, not an error.
    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .assert()
        .code(0);

    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .arg("--strict")
        .assert()
        .code(1);
}

/// ⚠️ 2 is not a louder 1. Diagnosing a directory that is not there would
/// otherwise report "no .env file" — a finding about the project rather than
/// about the path, which is how a CI step passes on an empty checkout.
#[test]
fn a_missing_directory_is_trouble_not_a_verdict() {
    let d = TempDir::new().unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path().join("not-here"))
        .assert()
        .code(2);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("No verdict"), "{stderr}");
}

/// The documented spelling, which was never accepted.
#[test]
fn path_is_accepted_as_a_flag_and_conflicts_with_the_positional() {
    let d = unhealthy_project();

    cargo_bin_cmd!("evnx")
        .args(["doctor", "--path"])
        .arg(d.path())
        .arg("--fix")
        .assert()
        .success();

    // Giving both is an error rather than a silent preference.
    cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .arg("--path")
        .arg(d.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

/// ⚠️ `env_example`'s "not tracked" branch was `fixable: true` with a fix action
/// that only printed advice and returned false. The summary counted a repair that
/// could never happen — invisible while auto-fix was an undocumented environment
/// variable, a lie once `--fix` is a flag someone runs expecting the count to drop.
#[test]
fn nothing_is_reported_fixable_unless_it_can_actually_be_fixed() {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".env"), "A=1\n").unwrap();
    fs::write(d.path().join(".gitignore"), ".env*\n").unwrap();
    // Present, but never `git add`ed.
    fs::write(d.path().join(".env.example"), "A=\n").unwrap();

    let assert = cargo_bin_cmd!("evnx")
        .arg("doctor")
        .arg(d.path())
        .arg("--fix")
        .env("EVNX_OUTPUT_JSON", "1")
        .assert();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        report["summary"]["fixable"], 0,
        "after --fix nothing may still be counted as fixable: {stdout}"
    );
    for check in report["checks"].as_array().unwrap() {
        if check["fixable"] == true {
            assert_eq!(
                check["fixed"], true,
                "{} says fixable but was not fixed",
                check["name"]
            );
        }
    }
}
