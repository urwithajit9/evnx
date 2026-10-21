//! `evnx scan` and `.evnx.toml`, `-v` and `-q` (R-scan, 0.10.14).
//!
//! * I-056 — `[scan] disable` silences the name-based finding too: for a value
//!   a disabled rule matches, and by name for the heuristics themselves. The
//!   names are listed in `--help`.
//! * D-36 — a `[[scan.patterns]]` rule does not report its own declaration.
//! * I-058 — `-v` counts built-in rules and custom patterns separately.
//! * I-049, I-060 — `-q` is findings only, plus the line saying the project
//!   config weakens the scan.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

const GH: &str = "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8";

fn project(files: &[(&str, &str)]) -> TempDir {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    d
}

/// (exit code, stdout, stderr)
fn run(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn patterns(dir: &TempDir, target: &str) -> Vec<String> {
    let (_, stdout, _) = run(dir, &["scan", target, "--format", "json"]);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"));
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            format!(
                "{} @ {}",
                f["pattern"].as_str().unwrap(),
                f["location"].as_str().unwrap()
            )
        })
        .collect()
}

// ── I-056 ───────────────────────────────────────────────────────────────────

/// Disabling GitHub Token left `Sensitive config key: GITHUB_TOKEN`, high,
/// for the same value.
#[test]
fn t_i056_disabling_a_rule_also_silences_the_name_finding_for_its_value() {
    let d = project(&[
        (
            ".env",
            &format!("GITHUB_TOKEN={GH}\nDB_PASSWORD=correct-horse-battery\n"),
        ),
        (".evnx.toml", "[scan]\ndisable = [\"GitHub Token\"]\n"),
    ]);
    let found = patterns(&d, ".env");
    assert_eq!(
        found,
        vec!["Sensitive config key: DB_PASSWORD @ .env:2 (DB_PASSWORD)".to_string()],
        "the token is silenced, the unrelated password is not"
    );
}

/// The name rule and the entropy fallback can be disabled by name.
#[test]
fn t_i056_the_heuristics_can_be_disabled_by_name() {
    let d = project(&[
        (".env", "DB_PASSWORD=correct-horse-battery\n"),
        (
            ".evnx.toml",
            "[scan]\ndisable = [\"sensitive config key\"]\n",
        ),
    ]);
    assert!(patterns(&d, ".env").is_empty());

    let d = project(&[
        ("a.txt", "blob q8R9s0T1u2V3w4X5y6Z7a8B9cDeFgHiJkL\n"),
        (
            ".evnx.toml",
            "[scan]\ndisable = [\"High-entropy string\"]\n",
        ),
    ]);
    assert!(patterns(&d, "a.txt").is_empty());
}

#[test]
fn t_i056_help_lists_the_names_disable_accepts() {
    let d = project(&[]);
    let (_, help, _) = run(&d, &["scan", "--help"]);
    for name in [
        "AWS Access Key",
        "GitHub Token",
        "Credentials in a connection string",
        "Sensitive config key",
        "High-entropy string",
        "AWS Secret Access Key",
    ] {
        assert!(help.contains(name), "{name} missing from --help:\n{help}");
    }
    assert!(help.contains("[scan] disable"), "{help}");
}

// ── D-36 ────────────────────────────────────────────────────────────────────

#[test]
fn t_d36_a_literal_pattern_does_not_report_its_own_declaration() {
    let d = project(&[
        (
            ".evnx.toml",
            "[[scan.patterns]]\nname = \"Acme\"\nregex = \"ACME-INTERNAL-TOKEN\"\n",
        ),
        ("src/app.py", "token = \"ACME-INTERNAL-TOKEN\"\n"),
    ]);
    let found = patterns(&d, ".");
    assert_eq!(
        found,
        vec!["Acme @ ./src/app.py:1".to_string()],
        "{found:?}"
    );
}

/// The config file is still scanned: a real key in it is still reported.
#[test]
fn t_d36_a_real_key_in_evnx_toml_is_still_found() {
    let d = project(&[(
        ".evnx.toml",
        &format!("# token for the bot: {GH}\n[scan]\nseverity = \"low\"\n"),
    )]);
    let found = patterns(&d, ".");
    assert_eq!(
        found,
        vec!["GitHub Token @ ./.evnx.toml:1".to_string()],
        "{found:?}"
    );
}

// ── I-058 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i058_verbose_counts_built_in_rules_and_custom_patterns_apart() {
    let d = project(&[(".env", "A=1\n")]);
    let (_, _, stderr) = run(&d, &["scan", ".", "-v"]);
    assert!(
        stderr.contains("13 built-in rules, 0 custom patterns"),
        "{stderr}"
    );
    assert!(!stderr.contains("custom pattern(s) active"), "{stderr}");
    assert!(stderr.contains("AWS Access Key"), "names at -v: {stderr}");

    let (_, _, stderr) = run(&d, &["scan", ".", "-v", "--pattern", "ACME-[0-9]{8}"]);
    assert!(
        stderr.contains("13 built-in rules, 1 custom pattern"),
        "{stderr}"
    );

    fs::write(d.path().join(".evnx.toml"), "[scan]\nbuiltins = false\n").unwrap();
    let (_, _, stderr) = run(&d, &["scan", ".", "-v", "--pattern", "ACME-[0-9]{8}"]);
    assert!(
        stderr.contains("0 built-in rules, 1 custom pattern"),
        "{stderr}"
    );
}

// ── I-049, I-060 ────────────────────────────────────────────────────────────

/// `-q` hid the one line saying the committed config weakens the scan.
#[test]
fn t_i060_quiet_keeps_the_weakening_line_for_scan() {
    let d = project(&[
        (".env", &format!("GITHUB_TOKEN={GH}\n")),
        (".evnx.toml", "[scan]\nseverity = \"high\"\n"),
    ]);
    let (code, stdout, stderr) = run(&d, &["scan", ".", "-q"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("scan.severity=high"), "{stderr}");
    // Findings only otherwise: no banner, no next steps.
    assert!(!stdout.contains("Checking for exposed secrets"), "{stdout}");
    assert!(!stdout.contains("Next steps"), "{stdout}");
    assert!(stdout.contains("GitHub Token"), "{stdout}");
}

/// A config that weakens nothing has nothing to say under `-q`; nor does
/// another command, for which the scan settings do not apply.
#[test]
fn t_i060_quiet_drops_a_banner_with_nothing_to_announce() {
    let d = project(&[
        (".env", "A=1\n"),
        (".evnx.toml", "[defaults]\nenv_name = \"dev\"\n"),
    ]);
    let (_, _, stderr) = run(&d, &["scan", ".", "-q"]);
    assert!(!stderr.contains(".evnx.toml"), "{stderr}");

    let d = project(&[
        (".env", "A=1\n"),
        (".evnx.toml", "[scan]\nseverity = \"high\"\n"),
    ]);
    let (_, _, stderr) = run(&d, &["validate", "-q"]);
    assert!(!stderr.contains(".evnx.toml"), "{stderr}");
}
