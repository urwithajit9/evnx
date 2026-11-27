//! `evnx spec init` end to end.
//!
//! Slice 2 of the spec-first work: generating a first contract, so nobody has to
//! write TOML by hand to start. The unit tests cover inference in isolation;
//! these check the command writes a file that evnx can read back.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "DATABASE_URL=postgres://u:pw@h/app\nPORT=8080\nSTRIPE_SECRET_KEY=sk_live_abc\nAPP_NAME=demo\n",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "DATABASE_URL=\nPORT=\nSTRIPE_SECRET_KEY=\nAPP_NAME=\n",
    )
    .unwrap();
    d
}

#[test]
fn stdout_writes_nothing_to_disk() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--stdout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[vars.DATABASE_URL]"));

    assert!(
        !d.path().join(".evnx.toml").exists(),
        "--stdout must not write the config"
    );
}

/// The round trip that matters: what `spec init` writes, `.evnx.toml` reads.
#[test]
fn the_generated_spec_is_read_back_by_the_next_command() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    let written = fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
    assert!(written.contains("[vars.DATABASE_URL]"), "{written}");
    assert!(written.contains("format = \"url\""), "{written}");
    assert!(written.contains("format = \"port\""), "{written}");
    assert!(written.contains("secret = true"), "{written}");

    // If the file did not parse, the config loader would fail the run.
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .success();
}

/// ⚠️ The whole point of appending rather than rewriting: a project's existing
/// config, and its comments, must survive untouched.
#[test]
fn an_existing_config_survives_byte_for_byte() {
    let d = project();
    let before = "# hand-written, do not lose me\n[scan]\nseverity = \"high\"\n";
    fs::write(d.path().join(".evnx.toml"), before).unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    let after = fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
    assert!(
        after.starts_with(before),
        "existing content changed:\n{after}"
    );
}

/// TOML refuses a file that declares the same table twice, so appending a second
/// `[vars]` would produce a config nothing can read. Refuse instead.
#[test]
fn a_second_run_refuses_rather_than_corrupting_the_file() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .success();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already has a [vars] section"));

    // And the file is still readable after the refusal.
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .success();
}

#[test]
fn components_carry_their_descriptions_and_attribution() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--with", "stripe", "--stdout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[vars.STRIPE_SECRET_KEY]"))
        .stdout(predicate::str::contains("# from Stripe"))
        .stdout(predicate::str::contains("description ="));
}

#[test]
fn an_unknown_component_is_refused_with_a_suggestion() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--with", "postgress", "--stdout"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Did you mean: postgresql"));
}

/// DEC-SP12: `postgres` is an alias of `postgresql`. It used to be refused, so
/// the `--help` example `--with django,postgres,redis` failed (D-127).
#[test]
fn postgres_is_accepted_as_an_alias() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "spec",
            "init",
            "--with",
            "django,postgres,redis",
            "--stdout",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("DATABASE_URL"));
}

#[test]
fn a_project_with_nothing_to_read_says_so() {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init", "--stdout"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("evnx init"));
}

/// T-SP-20 / T-SHARE-41 (I-176, DEC-SP1, DEC-G1): `--stdout` prints TOML and
/// nothing else, so `evnx spec init --stdout >> .evnx.toml` stays valid. The
/// banner used to be the first line of stdout.
#[test]
fn t_i176_stdout_is_only_toml() {
    for args in [
        &["spec", "init", "--stdout"][..],
        &[
            "spec",
            "init",
            "--with",
            "django,postgres,redis",
            "--stdout",
        ][..],
    ] {
        let d = project();
        let out = cargo_bin_cmd!("evnx")
            .current_dir(d.path())
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        let parsed: toml::Table = toml::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{args:?}: stdout is not TOML: {e}\n{stdout}"));
        assert!(parsed.contains_key("vars"), "{args:?}");
        assert!(
            !stdout.contains("Generate the variable contract"),
            "{args:?}"
        );
    }
}

/// The banner is not lost, it moved: it is on stderr, where a terminal still
/// shows it and a redirect does not capture it (DEC-G1).
#[test]
fn t_i176_banner_is_on_stderr() {
    let d = project();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["spec", "init", "--stdout"])
        .assert()
        .success()
        .stderr(predicate::str::contains("evnx spec"));
}

/// The spec-init run: a template line `TEST_VAR` (no `=`) was a parse error
/// with no way forward. The error stays; it now says how to write the line.
#[test]
fn a_bare_name_in_the_template_gets_a_hint() {
    let d = tempfile::TempDir::new().unwrap();
    std::fs::write(d.path().join(".env.example"), "TEST_VAR\n").unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing '=' separator"), "{stderr}");
    assert!(stderr.contains("write it as `TEST_VAR=`"), "{stderr}");
}

/// A bare line that looks like a key value is never repeated back.
#[test]
fn a_bare_secret_line_gets_no_hint_and_is_not_printed() {
    let d = tempfile::TempDir::new().unwrap();
    let key = format!("AKIA{}", "4OZRMFJ3VREALKEY");
    std::fs::write(d.path().join(".env.example"), format!("{key}\n")).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["spec", "init"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(&key), "{all}");
    assert!(!all.contains("write it as"), "{all}");
}
