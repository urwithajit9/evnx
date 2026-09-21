//! `evnx template` end to end — the three silent-wrong-answer bugs it had.
//!
//! All three shared one cause: substitution iterated over the *variables that
//! exist* and built a regex per key, rather than scanning the template for
//! placeholders. A form the per-key regex did not match was not reported as
//! unmatched — it was written to the output file verbatim, with exit 0.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn project(template: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), "PORT=8080\nAPP_ENV=production\n").unwrap();
    fs::write(d.path().join("in.tmpl"), template).unwrap();
    d
}

fn render(d: &TempDir, extra: &[&str]) -> String {
    let mut cmd = cargo_bin_cmd!("evnx");
    cmd.current_dir(d.path())
        .args(["template", "--input", "in.tmpl", "--output", "out.conf"])
        .args(extra)
        .assert()
        .success();
    fs::read_to_string(d.path().join("out.conf")).unwrap()
}

/// ⚠️ `{{ PORT }}` — with the spaces most people write — matched nothing and was
/// copied into the generated file as literal text.
#[test]
fn whitespace_inside_a_placeholder_still_substitutes() {
    let d = project(
        "a={{PORT}}\n\
         b={{ PORT }}\n\
         c=${PORT}\n\
         d=${ PORT }\n\
         e=$PORT\n\
         f={{APP_ENV|upper}}\n\
         g={{ APP_ENV | upper }}\n",
    );

    let out = render(&d, &[]);
    for line in out.lines() {
        assert!(
            !line.contains('{') && !line.contains('$'),
            "unsubstituted placeholder: {line}"
        );
    }
    assert_eq!(out.matches("8080").count(), 5, "{out}");
    assert_eq!(out.matches("PRODUCTION").count(), 2, "{out}");
}

/// ⚠️ `|default:` fired only when the variable *existed* — the one case a default
/// is not needed. A genuinely missing variable rendered the whole
/// `{{NOPE|default:1234}}` into the output.
#[test]
fn a_default_applies_when_the_variable_is_absent() {
    let d = project(
        "present={{PORT|default:1234}}\n\
         absent={{NOPE|default:1234}}\n\
         spaced={{ NOPE | default:5678 }}\n",
    );

    let out = render(&d, &[]);
    assert!(
        out.contains("present=8080"),
        "a value beats its default: {out}"
    );
    assert!(out.contains("absent=1234"), "{out}");
    assert!(out.contains("spaced=5678"), "{out}");
    assert!(!out.contains("default:"), "{out}");
}

/// A variable with a fallback is not undefined, so `--strict` must accept it.
#[test]
fn strict_accepts_a_placeholder_that_has_a_default() {
    let d = project("a={{PORT}}\nb={{NOPE|default:ok}}\n");
    let out = render(&d, &["--strict"]);
    assert_eq!(out, "a=8080\nb=ok\n", "{out}");
}

/// ⚠️ The headline: an unresolved placeholder was written through and the command
/// exited 0. The generated file then holds `{{DATABASE_URL}}` where a host name
/// belongs, and whatever reads it next treats that as the setting.
#[test]
fn strict_refuses_and_writes_nothing() {
    let d = project("url={{DATABASE_URL}}\nport={{PORT}}\n");

    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "template", "--input", "in.tmpl", "--output", "out.conf", "--strict",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("DATABASE_URL"))
        .stderr(predicate::str::contains("Nothing was written"));

    assert!(
        !d.path().join("out.conf").exists(),
        "a refused run must not leave a half-built config on disk"
    );
}

/// Without `--strict` the old permissive behaviour stands — it is someone's
/// working pipeline — but it now says what it did rather than offering a tip.
#[test]
fn without_strict_it_writes_through_but_says_so() {
    let d = project("url={{DATABASE_URL}}\n");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["template", "--input", "in.tmpl", "--output", "out.conf"])
        .assert()
        .success();

    // ⚠️ stdout, not stderr: `ui::warning` and `ui::info` use `println!` across
    // the whole CLI. Fine here — `--output` is required, so `template` never
    // writes data to stdout — but see the review's note on the inconsistency.
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("unresolved"), "{stdout}");
    assert!(stdout.contains("--strict"), "{stdout}");

    let out = fs::read_to_string(d.path().join("out.conf")).unwrap();
    assert_eq!(out, "url={{DATABASE_URL}}\n", "{out}");
}

/// `--strict` names every unresolved placeholder, not just the first.
#[test]
fn strict_names_all_of_them() {
    let d = project("a={{ONE}}\nb={{TWO}}\nc={{ THREE }}\n");

    let assert = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args([
            "template", "--input", "in.tmpl", "--output", "out.conf", "--strict",
        ])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    for name in ["ONE", "TWO", "THREE"] {
        assert!(stderr.contains(name), "{name} missing from: {stderr}");
    }
}
