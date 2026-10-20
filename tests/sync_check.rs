//! `sync --check`: one verdict whatever the output format (R-sync, 0.10.12).
//!
//! - I-117: `--check --format json` was a second implementation. It left out
//!   the spec's `[vars]` in reverse, ignored `--naming-policy error` and the
//!   `allow_actual` guard, and called a missing `.env` in reverse exit 2 where
//!   the pretty path said 1. Same flags, two verdicts. Now one drift
//!   computation feeds both.
//! - I-119, DEC-S3, DEC-S17: without `--direction`, `--check` is two-way — a
//!   name only the template has is drift as well (the developer's local
//!   check). An explicit `--direction` is one-way, which is how CI and the
//!   pre-commit hook ask.
//! - I-118: the fix the message names follows the direction.
//! - I-127, DEC-S14: an empty `.env` beside a populated template is no
//!   verdict under `--check`, not a pass.
//! - I-255: a name the spec requires is not attributed to the template.
//! - I-275, DEC-S20: reverse with no `.env` adds the spec's required names.
//! - I-121, DEC-S11: an unknown key in the placeholder config, or an invalid
//!   `[sync] naming_policy`, is an error naming the allowed values.

mod support;

use serde_json::{json, Value};
use support::{run, Project};

fn json_of(out: &support::Output) -> Value {
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", out.all()))
}

// ── I-117: one verdict for pretty and JSON ──────────────────────────────────

/// The same inputs must give the same exit code in both formats.
#[test]
fn t_i117_pretty_and_json_agree_on_every_case() {
    struct Case {
        name: &'static str,
        env: Option<&'static str>,
        example: Option<&'static str>,
        toml: Option<&'static str>,
        args: &'static [&'static str],
    }
    let cases = [
        Case {
            name: "forward in sync",
            env: Some("A=1\n"),
            example: Some("A=\n"),
            toml: None,
            args: &[],
        },
        Case {
            name: "forward drift",
            env: Some("A=1\nB=2\n"),
            example: Some("A=\n"),
            toml: None,
            args: &[],
        },
        Case {
            name: "reverse, .env missing",
            env: None,
            example: Some("A=\n"),
            toml: None,
            args: &["--direction", "reverse"],
        },
        Case {
            name: "reverse, spec-only name",
            env: Some("A=1\n"),
            example: Some("A=\n"),
            toml: Some("[vars.ONLY_SPEC]\n"),
            args: &["--direction", "reverse"],
        },
        Case {
            name: "naming policy error",
            env: Some("A=1\nbad_name=2\n"),
            example: Some("A=\n"),
            toml: None,
            args: &["--naming-policy", "error"],
        },
        Case {
            name: "allow_actual credential",
            env: Some("A=1\nSTRIPE=sk_live_abcdefghijklmnopqrstuvwx\n"),
            example: Some("A=\n"),
            toml: None,
            args: &["--template-config", "tc.json"],
        },
        Case {
            name: "template is unparsable",
            env: Some("A=1\n"),
            example: Some("A=\"open\n"),
            toml: None,
            args: &[],
        },
    ];
    for c in cases {
        let mut p = Project::new().file("tc.json", r#"{"allow_actual":["STRIPE"]}"#);
        if let Some(e) = c.env {
            p = p.env(e);
        }
        if let Some(x) = c.example {
            p = p.example(x);
        }
        if let Some(t) = c.toml {
            p = p.toml(t);
        }
        let mut pretty = vec!["sync", "--check"];
        pretty.extend_from_slice(c.args);
        let mut as_json = pretty.clone();
        as_json.extend_from_slice(&["--format", "json"]);
        let a = run(p.path(), &pretty);
        let b = run(p.path(), &as_json);
        assert_eq!(
            a.code,
            b.code,
            "{}: pretty and json disagree\n--- pretty\n{}\n--- json\n{}",
            c.name,
            a.all(),
            b.all()
        );
    }
}

/// T-SYNC-05: reverse JSON lists a name only the spec declares.
#[test]
fn t_i117_reverse_json_includes_spec_required_names() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\nC=3\n")
        .toml("[vars.ONLY_SPEC]\n");
    let out = run(
        p.path(),
        &[
            "sync",
            "--check",
            "--direction",
            "reverse",
            "--format",
            "json",
        ],
    );
    out.assert_code(1);
    let j = json_of(&out);
    assert_eq!(j["missing"], json!(["C", "ONLY_SPEC"]), "{}", out.stdout);
}

/// Reverse with no `.env`: "would create it" is drift in both formats.
#[test]
fn t_i117_reverse_json_without_env_is_drift_not_error() {
    let p = Project::new().example("A=\nB=\n");
    let out = run(
        p.path(),
        &[
            "sync",
            "--check",
            "--direction",
            "reverse",
            "--format",
            "json",
        ],
    );
    out.assert_code(1);
    assert_eq!(json_of(&out)["missing"], json!(["A", "B"]));
}

/// T-SYNC-06: the `allow_actual` guard runs before the JSON report too.
#[test]
fn t_i117_json_refuses_allow_actual_credentials() {
    let p = Project::new()
        .env("A=1\nSTRIPE=sk_live_abcdefghijklmnopqrstuvwx\n")
        .example("A=\n")
        .file("tc.json", r#"{"allow_actual":["STRIPE"]}"#);
    let out = run(
        p.path(),
        &[
            "sync",
            "--check",
            "--format",
            "json",
            "--template-config",
            "tc.json",
        ],
    );
    out.assert_code(2);
    assert!(out.stdout.trim().is_empty(), "no report:\n{}", out.stdout);
    out.assert_contains("STRIPE");
}

// ── I-119, DEC-S3, DEC-S17: two-way without --direction ─────────────────────

/// T-SYNC-08: a stale template entry is drift for the local check.
#[test]
fn t_i119_template_only_name_is_drift_without_direction() {
    let p = Project::new().env("A=1\n").example("A=\nOLD_FEATURE=\n");
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(1);
    out.assert_contains("OLD_FEATURE");
    // Both fixes are named: the template may be stale, or `.env` may be.
    out.assert_contains("--direction reverse");
}

/// DEC-S17: an explicit `--direction` asks one way only (CI, the hook).
#[test]
fn t_i119_explicit_direction_is_one_way() {
    let p = Project::new().env("A=1\n").example("A=\nOLD_FEATURE=\n");
    run(p.path(), &["sync", "--check", "--direction", "forward"]).assert_code(0);
    // Reverse with an explicit direction: names only in `.env` are local
    // settings, not drift.
    let p = Project::new().env("A=1\nLOCAL=1\n").example("A=\n");
    run(p.path(), &["sync", "--check", "--direction", "reverse"]).assert_code(0);
}

/// T-SYNC-60: JSON reports the other side as `template_only`.
#[test]
fn t_i119_json_reports_template_only() {
    let p = Project::new()
        .env("A=1\nNEW=1\n")
        .example("A=\nOLD_FEATURE=\n");
    let out = run(p.path(), &["sync", "--check", "--format", "json"]);
    out.assert_code(1);
    let j = json_of(&out);
    assert_eq!(j["missing"], json!(["NEW"]));
    assert_eq!(j["template_only"], json!(["OLD_FEATURE"]));
    assert_eq!(j["summary"]["template_only"], 1);

    // One-way: the field is absent, so its presence means it was checked.
    let out = run(
        p.path(),
        &[
            "sync",
            "--check",
            "--format",
            "json",
            "--direction",
            "forward",
        ],
    );
    out.assert_code(1);
    let j = json_of(&out);
    assert!(j.get("template_only").is_none(), "{}", out.stdout);
}

/// Plain `sync` still only adds, and says what only the template has.
#[test]
fn t_i119_plain_sync_lists_template_only_names_and_removes_nothing() {
    let p = Project::new().env("A=1\n").example("A=\nOLD_FEATURE=x\n");
    let out = run(p.path(), &["sync", "-f"]);
    out.assert_code(0);
    out.assert_contains("OLD_FEATURE");
    assert!(p.read(".env.example").contains("OLD_FEATURE=x"));
}

/// The shipped hook asks one way, so a teammate whose `.env` lacks a name the
/// template has can still commit.
#[test]
fn t_i119_the_shipped_hook_is_one_way() {
    let hooks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".pre-commit-hooks.yaml"),
    )
    .unwrap();
    let entry = hooks
        .split("- id: evnx-sync\n")
        .nth(1)
        .and_then(|h| h.lines().find(|l| l.trim_start().starts_with("entry:")))
        .expect("evnx-sync entry");
    assert!(entry.contains("--direction forward"), "{entry}");
    assert!(entry.contains("--check"), "{entry}");
}

// ── I-118: the fix follows the direction ────────────────────────────────────

/// T-SYNC-07: reverse adds to a git-ignored `.env`; nothing is committed.
#[test]
fn t_i118_reverse_message_names_env_and_no_commit() {
    let p = Project::new().env("A=1\n").example("A=\nB=\n");
    let out = run(p.path(), &["sync", "--check", "--direction", "reverse"]);
    out.assert_code(1);
    out.assert_not_contains("commit the result");
    out.assert_contains("evnx sync --direction reverse");
}

// ── I-127, DEC-S14 ──────────────────────────────────────────────────────────

/// T-SYNC-67.
#[test]
fn t_i127_empty_env_beside_a_full_template_is_no_verdict() {
    let p = Project::new().env("").example("A=\nB=\n");
    let out = run(p.path(), &["sync", "--check", "--direction", "forward"]);
    out.assert_code(2);
    let out = run(p.path(), &["sync", "--check"]);
    out.assert_code(2);
    // Plain sync stays a warning.
    run(p.path(), &["sync", "-f"]).assert_code(0);
}

// ── I-255: where a missing name comes from ──────────────────────────────────

#[test]
fn t_i255_spec_only_name_is_not_attributed_to_the_template() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\nC=3\n")
        .toml("[vars.ONLY_SPEC]\n");
    let out = run(p.path(), &["sync", "-n", "--direction", "reverse"]);
    out.assert_code(0);
    out.assert_not_contains("in .env.example missing from .env");
    let line = out
        .all()
        .lines()
        .find(|l| l.contains("ONLY_SPEC"))
        .map(str::to_string)
        .unwrap_or_default();
    assert!(line.contains("[vars]"), "names its source: {line:?}");
}

// ── I-275, DEC-S20: reverse with no .env ────────────────────────────────────

#[test]
fn t_i275_new_env_gets_the_specs_required_names() {
    let p = Project::new().example("A=1\n").toml("[vars.ONLY_SPEC]\n");
    run(p.path(), &["sync", "-f", "--direction", "reverse"]).assert_code(0);
    let env = p.read(".env");
    assert!(env.lines().any(|l| l == "A=1"), "{env}");
    assert!(
        env.lines().any(|l| l.starts_with("ONLY_SPEC=")),
        "spec name added:\n{env}"
    );
    // The preview says the same before writing.
    let p = Project::new().example("A=1\n").toml("[vars.ONLY_SPEC]\n");
    run(p.path(), &["sync", "-n", "--direction", "reverse"])
        .assert_code(0)
        .assert_contains("ONLY_SPEC");
}

// ── I-121, DEC-S11: config strictness ───────────────────────────────────────

/// T-SYNC-32 / T-SYNC-61.
#[test]
fn t_i121_unknown_placeholder_config_key_is_an_error() {
    let p = Project::new()
        .env("A=1\nNEW_FEATURE=1\n")
        .example("A=\n")
        .file("tc.json", r#"{"allow_actaul":["NEW_FEATURE"]}"#);
    let out = run(p.path(), &["sync", "-n", "--template-config", "tc.json"]);
    out.assert_code(2);
    out.assert_contains("allow_actaul");
    out.assert_contains("allow_actual");
    // `$schema` and `description` are accepted; `naming_convention` warns.
    let p = Project::new().env("A=1\n").example("A=\n").file(
        "tc.json",
        r#"{"$schema":"x","description":"d","naming_convention":"upper"}"#,
    );
    run(p.path(), &["sync", "-n", "--template-config", "tc.json"])
        .assert_code(0)
        .assert_contains("naming_convention");
}

/// T-SYNC-33.
#[test]
fn t_i121_invalid_naming_policy_in_config_is_an_error() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\n")
        .toml("[sync]\nnaming_policy = \"bogus\"\n");
    let out = run(p.path(), &["sync", "-n"]);
    out.assert_code(2);
    out.assert_contains("bogus");
    out.assert_contains("warn, error, ignore");
    // The flag wins over the file (T-SYNC-34), but the file is still checked,
    // as `[validate] fail_on` is (I-038): a typo there is not hidden by a
    // flag that happens to be passed today.
    run(p.path(), &["sync", "-n", "--naming-policy", "warn"]).assert_code(2);
}

/// I-125: the command a verdict suggests names the files in play, so it fixes
/// the pair that was checked and not the default one.
#[test]
fn t_i125_verdict_command_names_non_default_files() {
    let p = Project::new()
        .file(".env.production", "A=1\nB=2\n")
        .example("A=\nC=\n");
    let out = run(p.path(), &["sync", "--check", "--env-name", "production"]);
    out.assert_code(1);
    out.assert_contains("evnx sync --direction forward --env .env.production");
    out.assert_contains("evnx sync --direction reverse --env .env.production");
}

// ── Docs drift (T-SYNC-53, D-66) ────────────────────────────────────────────

/// Every flag `sync --help` lists is in the README's sync section, and every
/// flag the section names exists. The README showed two of thirteen (D-66),
/// and the docs site listed `-d` and `-p`, which never existed (D-73).
#[test]
fn t_d66_readme_sync_section_matches_help() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let section = readme
        .split("### `evnx sync`")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .expect("README has a sync section");
    let help = run(std::path::Path::new("."), &["sync", "--help"]).stdout;
    let long_flags = |text: &str| -> std::collections::BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|w| {
                w.strip_prefix("--")
                    .is_some_and(|f| f.starts_with(|c: char| c.is_ascii_alphabetic()))
            })
            .map(str::to_string)
            .collect()
    };
    let in_help = long_flags(&help);
    let in_readme = long_flags(section);
    for flag in &in_help {
        // The global flags are documented once, not per command.
        if ["--verbose", "--quiet", "--no-color", "--help", "--version"].contains(&flag.as_str()) {
            continue;
        }
        assert!(
            in_readme.contains(flag),
            "README's sync section lacks {flag}"
        );
    }
    for flag in &in_readme {
        assert!(
            in_help.contains(flag),
            "README names {flag}, which sync does not take"
        );
    }
}
