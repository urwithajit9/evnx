//! Machine-readable output is a contract. These tests pin it byte for byte.
//!
//! # Why this exists
//!
//! `scan --format json|sarif|github`, `validate --format json`, `diff --format
//! json`, `convert --to …` and `EVNX_OUTPUT_JSON=1 doctor` are parsed by other
//! programs — CI gates, SARIF uploaders, `jq` in a deploy script. A change to any
//! of them breaks someone's pipeline silently, because a pipeline reading a field
//! that moved does not crash, it reads the wrong thing.
//!
//! Human-facing output is deliberately **not** covered here. It is meant to
//! change, and pinning it would make every improvement a test edit.
//!
//! # Updating a golden file
//!
//! ```bash
//! UPDATE_GOLDEN=1 cargo test --all-features --test golden_output
//! git diff tests/golden/          # read every line before committing
//! ```
//!
//! ⚠️ A diff here is either a bug or a decision. There is no third case. If it is
//! a decision, say which one in the commit message — the last time one of these
//! changed on purpose it was to stop `scan --format json` writing a live AWS key
//! into `report.json`.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A fixture with one high-confidence secret, one drifted key and one extra, so
/// every surface has something to say.
///
/// ⚠️ `.env`'s mode is set explicitly. `fs::write` creates a file at
/// `0666 & !umask`, so it lands at 664 on a machine with umask 002 and 644 on
/// one with umask 022 — and `doctor` reports that mode in its JSON. The first
/// version of this fixture left it to the umask and passed locally while failing
/// in CI, which is exactly the class of difference a golden file is supposed to
/// make impossible.
///
/// 0644 rather than 0600 on purpose: it keeps the permissions check in its
/// warning state, so the golden covers that branch rather than the clean one.
fn fixture() -> TempDir {
    let d = TempDir::new().unwrap();
    let env = d.path().join(".env");
    fs::write(
        &env,
        "DB_HOST=localhost\nAWS_KEY=AKIA4OZRMFJ3VREALKEY\nEXTRA=1\nPORT=8080\n",
    )
    .unwrap();
    fs::write(d.path().join(".env.example"), "DB_HOST=\nAWS_KEY=\nPORT=\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&env).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&env, perms).unwrap();
    }

    d
}

/// Blank out the fields that legitimately differ between runs, so a golden file
/// pins the *shape* rather than the clock.
fn normalise(raw: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut out = String::with_capacity(raw.len());

    for line in raw.lines() {
        let line = if line.trim_start().starts_with("\"timestamp\"") {
            // RFC3339 with nanoseconds — different every run.
            let indent = &line[..line.len() - line.trim_start().len()];
            format!("{indent}\"timestamp\": \"<normalised>\"")
        } else {
            // The crate version moves on every release and is not part of the
            // contract being pinned here.
            line.replace(version, "<version>")
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// Compare against the golden file, or rewrite it under `UPDATE_GOLDEN=1`.
fn check(name: &str, actual_raw: &str) {
    let actual = normalise(actual_raw);
    let path = golden_path(name);

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, &actual).unwrap();
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden file {}\n\n\
             Create it with:  UPDATE_GOLDEN=1 cargo test --all-features --test golden_output",
            path.display()
        )
    });

    assert_eq!(
        expected, actual,
        "\n\n{} changed.\n\n\
         This output is parsed by other programs. A diff here is either a bug or a\n\
         deliberate decision — if it is deliberate, regenerate with UPDATE_GOLDEN=1\n\
         and say which decision in the commit message.\n",
        name
    );
}

/// Run evnx in the fixture and return stdout — the data channel. stderr is
/// deliberately discarded: banners, hints and warnings live there and are free
/// to change.
fn stdout_of(dir: &TempDir, args: &[&str]) -> String {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(args)
        .output()
        .expect("evnx runs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stdout_with_env(dir: &TempDir, args: &[&str], key: &str, value: &str) -> String {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .args(args)
        .env(key, value)
        .output()
        .expect("evnx runs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn scan_json_is_stable() {
    let d = fixture();
    check(
        "scan.json",
        &stdout_of(&d, &["scan", ".", "--format", "json", "--exit-zero"]),
    );
}

/// ⚠️ SARIF is consumed by GitHub code scanning. A shape change here is not a
/// cosmetic one — it is an upload that stops working.
#[test]
fn scan_sarif_is_stable() {
    let d = fixture();
    check(
        "scan.sarif",
        &stdout_of(&d, &["scan", ".", "--format", "sarif", "--exit-zero"]),
    );
}

/// GitHub workflow commands: `::error file=…,line=…::message`. The escaping rules
/// are part of the format, not of the message.
#[test]
fn scan_github_is_stable() {
    let d = fixture();
    check(
        "scan.github",
        &stdout_of(&d, &["scan", ".", "--format", "github", "--exit-zero"]),
    );
}

#[test]
fn validate_json_is_stable() {
    let d = fixture();
    check(
        "validate.json",
        &stdout_of(&d, &["validate", "--format", "json", "--exit-zero"]),
    );
}

/// ⚠️ The plain `validate --format json` fixture produces **no issues**, so its
/// golden never covered the `issues[]` array or the `location` field inside it.
/// A change to how a location is rendered passed straight through the net.
///
/// `--strict` is what makes the extra-variable check run, so this is the case
/// that actually exercises an issue.
#[test]
fn validate_strict_json_is_stable() {
    let d = fixture();
    check(
        "validate-strict.json",
        &stdout_of(
            &d,
            &["validate", "--strict", "--format", "json", "--exit-zero"],
        ),
    );
}

#[test]
fn diff_json_is_stable() {
    let d = fixture();
    check("diff.json", &stdout_of(&d, &["diff", "--format", "json"]));
}

/// `convert`'s stdout *is* the artefact — it gets redirected into a file that
/// something else reads. It has no banner to change.
#[test]
fn convert_json_is_stable() {
    let d = fixture();
    check("convert.json", &stdout_of(&d, &["convert", "--to", "json"]));
}

#[test]
fn convert_yaml_is_stable() {
    let d = fixture();
    check("convert.yaml", &stdout_of(&d, &["convert", "--to", "yaml"]));
}

#[test]
fn doctor_json_is_stable() {
    let d = fixture();
    check(
        "doctor.json",
        &stdout_with_env(&d, &["doctor"], "EVNX_OUTPUT_JSON", "1"),
    );
}

/// The property behind all of the above: nothing decorative may reach stdout.
/// A banner on the data channel is how a `| jq` recipe starts failing.
#[test]
fn no_machine_surface_carries_terminal_chrome() {
    let d = fixture();
    for args in [
        vec!["scan", ".", "--format", "json", "--exit-zero"],
        vec!["scan", ".", "--format", "sarif", "--exit-zero"],
        vec!["validate", "--format", "json", "--exit-zero"],
        vec!["convert", "--to", "json"],
    ] {
        let out = stdout_of(&d, &args);
        for ch in ['┌', '│', '└', '─', '✓', '✗', '⚠', 'ℹ', '📖', '📊'] {
            assert!(!out.contains(ch), "{args:?} put {ch:?} on stdout:\n{out}");
        }
    }
}
