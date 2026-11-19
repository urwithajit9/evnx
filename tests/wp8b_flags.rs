//! WP-8b: `sync`, `validate`, `align`, `convert` and `template` on the shared
//! flags (`--yes`, `-n`, `--json`, value enums).

mod support;
use support::{Project, run};

#[test]
fn validate_fix_yes_applies_the_fixes() {
    let p = Project::new()
        .example("SECRET_KEY=\nA=1\n")
        .env("A=1\nSECRET_KEY=changeme\n");
    run(p.path(), &["validate", "--fix", "--yes", "--exit-zero"]).assert_code(0);
    // `--yes` is for `--fix`: alone it is refused.
    run(p.path(), &["validate", "--yes"]).assert_code(2);
    // And `-n` still needs `--fix`.
    run(p.path(), &["validate", "-n"]).assert_code(2);
}

#[test]
fn validate_input_format_and_fail_on_are_value_lists() {
    let p = Project::new().example("A=1\n").env("A=1\n");
    for (flag, bad) in [("--input-format", "yaml"), ("--fail-on", "never")] {
        let out = run(p.path(), &["validate", flag, bad]);
        out.assert_code(2);
        assert!(out.stderr.contains("possible values"), "{}", out.stderr);
    }
    run(
        p.path(),
        &[
            "validate",
            "--input-format",
            "dotenv",
            "--fail-on",
            "warning",
        ],
    )
    .assert_code(0);
}

#[test]
fn align_takes_json_as_a_shorthand() {
    let p = Project::new().example("A=1\nB=2\n").env("B=2\nA=1\n");
    let out = run(p.path(), &["align", "--dry-run", "--json"]);
    out.assert_code(0);
    serde_json::from_str::<serde_json::Value>(&out.stdout).expect("JSON on stdout");
    run(p.path(), &["align", "--format", "xml"]).assert_code(2);
}

#[test]
fn convert_yes_without_to_names_to() {
    let p = Project::new().env("A=1\n");
    let out = run(p.path(), &["convert", "--yes"]);
    out.assert_code(2);
    assert!(out.stderr.contains("--to"), "{}", out.stderr);
    assert!(out.stdout.is_empty(), "{}", out.stdout);
}

#[test]
fn convert_dry_run_writes_nothing() {
    let p = Project::new().env("A=1\n");
    let out = run(
        p.path(),
        &["convert", "--to", "json", "-o", "out.json", "--dry-run"],
    );
    out.assert_code(0);
    assert!(!p.path().join("out.json").exists());
    assert!(out.stdout.contains("would convert"), "{}", out.stdout);
    // Nothing is written without --output, so `--dry-run` needs it.
    run(p.path(), &["convert", "--to", "json", "-n"]).assert_code(2);
    // Every refusal of the real run still applies.
    run(
        p.path(),
        &["convert", "--to", "gcp-secrets", "-o", "o.sh", "-n"],
    )
    .assert_code(2);
}

#[test]
fn convert_to_is_a_value_list_with_github() {
    let p = Project::new().env("A=1\n");
    run(p.path(), &["convert", "--to", "github"]).assert_code(0);
    let out = run(p.path(), &["convert", "--to", "nope"]);
    out.assert_code(2);
    assert!(out.stderr.contains("github"), "{}", out.stderr);
    // Case and the short aliases are still accepted.
    run(p.path(), &["convert", "--to", "JSON"]).assert_code(0);
    run(p.path(), &["convert", "--to", "yml"]).assert_code(0);
}

#[test]
fn template_dry_run_has_a_short_flag() {
    let p = Project::new().env("A=1\n");
    std::fs::write(p.path().join("t.tpl"), "a={{A}}\n").unwrap();
    run(
        p.path(),
        &["template", "--input", "t.tpl", "--output", "t.out", "-n"],
    )
    .assert_code(0);
    assert!(!p.path().join("t.out").exists());
}
