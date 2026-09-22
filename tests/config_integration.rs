//! `.evnx.toml` applied end to end.
//!
//! The unit tests cover parsing and precedence in isolation; these check that a
//! config file actually reaches the commands, and that it is announced.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project(config: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), "A=1\nEXTRA_ONLY_HERE=2\n").unwrap();
    fs::write(d.path().join(".env.example"), "A=x\n").unwrap();
    fs::write(d.path().join(".env.production"), "A=prod\n").unwrap();
    if !config.is_empty() {
        fs::write(d.path().join(".evnx.toml"), config).unwrap();
    }
    d
}

#[test]
fn validate_strict_comes_from_config() {
    let without = project("");
    cargo_bin_cmd!("evnx")
        .current_dir(without.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE").not());

    let with = project("[validate]\nstrict = true\n");
    cargo_bin_cmd!("evnx")
        .current_dir(with.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE"));
}

#[test]
fn defaults_env_name_selects_the_file() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("prod"), "{stdout}");
}

/// ⚠️ The precedence bug that `--env` becoming `Option` exists to fix.
#[test]
fn an_explicit_flag_beats_config() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--env", ".env", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("EXTRA_ONLY_HERE"),
        "--env must win over [defaults] env_name:\n{stdout}"
    );
}

/// ⚠️ A committed config can weaken scanning, so every run that loads one says
/// so — and names the settings that changed a security default.
#[test]
fn a_config_that_weakens_scanning_is_announced() {
    let d = project("[scan]\nseverity = \"high\"\nexclude = [\"fixtures/\"]\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml"))
        .stderr(predicate::str::contains("scan.severity=high"))
        .stderr(predicate::str::contains("1 patterns"));
}

#[test]
fn a_config_that_changes_nothing_security_relevant_is_announced_plainly() {
    let d = project("[defaults]\nenv_name = \"production\"\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml"))
        .stderr(predicate::str::contains("scan.severity").not());
}

/// The banner and warnings are stderr-only, or `evnx convert --to json > out`
/// would stop being machine-readable.
#[test]
fn nothing_about_config_reaches_stdout() {
    let d = project("[scan]\nseverity = \"high\"\n\n[nonesuch]\nx = 1\n");

    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(!stdout.contains(".evnx.toml"), "{stdout}");
    assert!(!stdout.contains("unknown key"), "{stdout}");
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout must stay valid JSON");
}

#[test]
fn unknown_keys_are_named_but_do_not_fail_the_run() {
    let d = project("[scan]\nseverity = \"high\"\nentropy_threshold = 4.5\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--exit-zero"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "unknown key `scan.entropy_threshold`",
        ));
}

#[test]
fn quiet_suppresses_the_banner() {
    let d = project("[scan]\nseverity = \"high\"\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "--quiet", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains(".evnx.toml").not());
}

/// A malformed config is an error: ignoring one written on purpose would mean
/// running with policy the user believes is in force.
#[test]
fn a_malformed_config_stops_the_run() {
    let d = project("[scan\nseverity = ");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["convert", "--to", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(".evnx.toml"));
}

/// ⚠️ The config must be found from a **subdirectory** of the project.
///
/// `find` walks upward with `Path::parent`, and `Path::new(".").parent()` is
/// `""` rather than the parent directory — so passing a relative start makes the
/// walk inert and the file is found only in the working directory. `main.rs` did
/// exactly that, and every unit test missed it by supplying an absolute
/// `TempDir` path.
#[test]
fn config_is_found_from_a_subdirectory() {
    let d = project("[validate]\nstrict = true\n");
    let nested = d.path().join("packages").join("api");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join(".env"), "A=1\nEXTRA_ONLY_HERE=2\n").unwrap();
    fs::write(nested.join(".env.example"), "A=x\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(&nested)
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE"));
}

/// The project boundary still holds: a config above the repository root is not
/// this project's policy, however far the walk would otherwise reach.
#[test]
fn a_config_outside_the_repository_is_not_used() {
    let outer = TempDir::new().unwrap();
    fs::write(
        outer.path().join(".evnx.toml"),
        "[validate]\nstrict = true\n",
    )
    .unwrap();

    let repo = outer.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join(".env"), "A=1\nEXTRA_ONLY_HERE=2\n").unwrap();
    fs::write(repo.join(".env.example"), "A=x\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(&repo)
        .args(["validate", "--exit-zero"])
        .assert()
        .stdout(predicate::str::contains("EXTRA_ONLY_HERE").not());
}

/// The banner is printed on every run, so it stays short: the loader resolves an
/// absolute path, and what gets shown is relative to the working directory.
/// From a package that reads `../../.evnx.toml`, which says the policy came from
/// above rather than from here.
#[test]
fn the_banner_shows_where_the_config_came_from() {
    let d = project("[scan]\nseverity = \"high\"\n");
    let nested = d.path().join("packages").join("api");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join(".env"), "A=1\n").unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains("config    .evnx.toml"));

    cargo_bin_cmd!("evnx")
        .current_dir(&nested)
        .args(["validate", "--exit-zero"])
        .assert()
        .stderr(predicate::str::contains("config    ../../.evnx.toml"));
}

// ─── [vars] — the variable contract (slice 1: parsed, not yet applied) ───────

/// ⚠️ This test asserted the **opposite** until slice 3.
///
/// While `[vars]` was parsed and unread, `OPTIONAL_THING` was reported missing
/// even though the spec declares `required = false`, because `validate` counted
/// `.env.example` and the template cannot say "optional". That assertion was
/// written to flip, and this is the flip: the contract is now load-bearing.
#[test]
fn an_optional_variable_is_no_longer_reported_missing() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "DATABASE_URL=postgres://h/d
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "DATABASE_URL=
OPTIONAL_THING=
",
    )
    .unwrap();
    fs::write(
        d.path().join(".evnx.toml"),
        r#"
[vars.DATABASE_URL]
required = true
format   = "url"
secret   = true

[vars.OPTIONAL_THING]
required = false
format   = "bool"
"#,
    )
    .unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OPTIONAL_THING").not());
}

/// A project with no `[vars]` keeps the template-driven behaviour exactly.
#[test]
fn without_a_spec_the_template_still_decides() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "A=1
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "A=
MISSING_ONE=
",
    )
    .unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("MISSING_ONE"));
}

/// A declared format is enforced, and the message says what was expected.
#[test]
fn a_declared_format_is_enforced() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "PORT=not-a-port
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "PORT=
",
    )
    .unwrap();
    fs::write(
        d.path().join(".evnx.toml"),
        "[vars.PORT]\nformat = \"port\"\n",
    )
    .unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("PORT is not a port between"));
}

/// `environments` narrows the contract to the file actually being checked.
#[test]
fn an_environment_scoped_variable_only_applies_there() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".env"),
        "BASE=1
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.staging"),
        "BASE=1
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.production"),
        "BASE=1
",
    )
    .unwrap();
    fs::write(
        d.path().join(".env.example"),
        "BASE=
",
    )
    .unwrap();
    fs::write(
        d.path().join(".evnx.toml"),
        "[vars.BASE]\n[vars.PROD_ONLY]\nenvironments = [\"production\"]\n",
    )
    .unwrap();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--env-name", "staging"])
        .assert()
        .success();

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate", "--env-name", "production"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("PROD_ONLY"));
}

/// ⚠️ A regex that will not compile is refused when the config is read, not the
/// first time a value happens to be checked against it. A contract that cannot
/// be applied is broken whether or not anyone has tripped over it.
#[test]
fn a_spec_with_a_broken_regex_is_refused_at_load() {
    let d = project("[vars.A]\nformat = \"([unclosed\"\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["validate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("regex"));
}

/// Named formats are accepted; the list is the one `--validate-formats` already
/// implements, so the spec exposes existing behaviour rather than new checks.
#[test]
fn every_named_format_is_accepted_by_the_parser() {
    for name in ["url", "int", "port", "bool", "email"] {
        let d = project(&format!("[vars.A]\nformat = \"{name}\"\n"));
        cargo_bin_cmd!("evnx")
            .current_dir(d.path())
            .args(["validate", "--exit-zero"])
            .assert()
            .success();
    }
}

// ─── [vars] secret — slice 4 of Proposal D ──────────────────────────────────

fn scan_project(env: &str, config: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), env).unwrap();
    if !config.is_empty() {
        fs::write(d.path().join(".evnx.toml"), config).unwrap();
    }
    d
}

/// ⚠️ The false negative the heuristics cannot reach. A real credential whose
/// name gives nothing away — no SECRET, no KEY, no recognisable value prefix —
/// scans clean and exits 0. Declaring it is the only way to say so.
#[test]
fn a_declared_secret_is_reported_even_when_nothing_recognises_it() {
    let env = "TENANT_A=9f3a7c21b85e4d0fa62c\n";

    cargo_bin_cmd!("evnx")
        .current_dir(scan_project(env, "").path())
        .args(["scan", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("No secrets detected"));

    cargo_bin_cmd!("evnx")
        .current_dir(scan_project(env, "[vars.TENANT_A]\nsecret = true\n").path())
        .args(["scan", "."])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Declared secret"))
        .stdout(predicate::str::contains("TENANT_A"));
}

/// A declaration says the variable holds a secret, not that every string in it
/// is one. Flagging an empty or placeholder value would train people to ignore
/// the scanner.
#[test]
fn a_declared_secret_that_is_empty_or_a_placeholder_is_not_reported() {
    for value in ["", "YOUR_KEY_HERE", "changeme"] {
        cargo_bin_cmd!("evnx")
            .current_dir(
                scan_project(
                    &format!("TENANT_A={value}\n"),
                    "[vars.TENANT_A]\nsecret = true\n",
                )
                .path(),
            )
            .args(["scan", "."])
            .assert()
            .success();
    }
}

/// `secret = false` retracts a guess made from the variable's NAME.
#[test]
fn secret_false_retracts_a_name_based_finding() {
    let env = "INTERNAL_SECRET=a-long-looking-config-value-not-a-key\n";

    cargo_bin_cmd!("evnx")
        .current_dir(scan_project(env, "").path())
        .args(["scan", "."])
        .assert()
        .failure()
        .stdout(predicate::str::contains("INTERNAL_SECRET"));

    cargo_bin_cmd!("evnx")
        .current_dir(scan_project(env, "[vars.INTERNAL_SECRET]\nsecret = false\n").path())
        .args(["scan", "."])
        .assert()
        .success();
}

/// ⚠️ The safety property. `.evnx.toml` is committed, so if a declaration could
/// retract a match on the VALUE, one wrong line would silence a live credential
/// for everyone who clones the repository. It cannot.
#[test]
fn secret_false_cannot_silence_a_live_key_in_the_value() {
    let d = scan_project(
        "STRIPE_SECRET_KEY=sk_live_4eC39HqLyjWDarjtT1zdp7dc\n",
        "[vars.STRIPE_SECRET_KEY]\nsecret = false\n",
    );

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "."])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Stripe"));
}

/// Loosening scanning from a committed file is announced, as `scan.exclude`
/// already is.
///
/// ⚠️ On **stderr**, not stdout — `nothing_about_config_reaches_stdout` above
/// pins that, so a `--format json` consumer never has the banner spliced into
/// the document it is parsing.
#[test]
fn secret_false_is_announced_as_a_security_override() {
    let d = scan_project("A=1\n", "[vars.A]\nsecret = false\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", "."])
        .assert()
        .stderr(predicate::str::contains("vars.secret=false on 1"));
}
