//! WP-14: `convert --from`, `--to dotenv`, `--kind`, and `-` for stdin.
//!
//! The acceptance rule: `convert --to json`, then `convert --from json --to
//! dotenv`, gives the same values, whatever they hold.

mod support;

use evnx::core::Parser;
use support::{run, run_with, Project};

/// The `.env` the tests start from: every kind of value that needs quoting.
const TRICKY: &str = concat!(
    "PLAIN=value\n",
    "EMPTY=\n",
    "SPACED=\"two words here\"\n",
    "HASH=\"a # not a comment\"\n",
    "EQUALS=https://x.test/?a=b&c=d\n",
    "DOLLAR='$HOME and ${NOT_EXPANDED}'\n",
    "DOUBLE_QUOTE=\"say \\\"hi\\\"\"\n",
    "SINGLE_QUOTE=\"it's\"\n",
    "BACKSLASH='C:\\dir\\file'\n",
    "NEWLINE=\"line one\\nline two\"\n",
    "UNICODE=caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1f600}\n",
    "PADDED=\"  leading and trailing  \"\n",
    "TAB=\"a\\tb\"\n",
);

fn parse(text: &str) -> Vec<(String, String)> {
    Parser::default()
        .parse_content(text)
        .unwrap_or_else(|e| panic!("does not parse: {e}\n{text}"))
        .into_iter()
        .collect()
}

fn convert(p: &Project, args: &[&str]) -> support::Output {
    let mut full = vec!["convert"];
    full.extend_from_slice(args);
    run(p.path(), &full)
}

#[test]
fn wp14_roundtrip_json_to_dotenv_same_values() {
    let p = Project::new().env(TRICKY);
    let json = convert(&p, &["--to", "json"]);
    json.assert_code(0);
    let p = p.file("vars.json", &json.stdout);
    let back = convert(
        &p,
        &["--from", "json", "--to", "dotenv", "--env", "vars.json"],
    );
    back.assert_code(0);
    let original = parse(TRICKY);
    let again = parse(&back.stdout);
    assert_eq!(original.len(), 13, "the fixture lost a line");
    // Same names, same values, same order.
    assert_eq!(
        original, again,
        "round trip changed something:\n{}",
        back.stdout
    );
}

/// One value at a time, so a failure names the value.
#[test]
fn wp14_roundtrip_every_dotenv_edge_case_table() {
    let values = [
        "plain",
        "",
        " ",
        "  both  ",
        "a b",
        "a#b",
        "a #b",
        "#",
        "=",
        "a=b=c",
        "$",
        "$HOME",
        "${X}",
        "$(whoami)",
        "`tick`",
        "'",
        "\"",
        "'\"",
        "it's \"quoted\"",
        "back\\slash",
        "\\",
        "\\n is not a newline",
        "line1\nline2",
        "line1\r\nline2",
        "trailing newline\n",
        "tab\there",
        "caf\u{e9}",
        "\u{65e5}\u{672c}\u{8a9e}",
        "\u{1f600}",
        "-----BEGIN KEY-----\nabc\n-----END KEY-----",
        "export",
        "# comment-looking",
    ];
    for (i, value) in values.iter().enumerate() {
        let json = format!(
            "{{{}: {}, \"AFTER\": \"x\"}}",
            serde_json::to_string(&format!("K{i}")).unwrap(),
            serde_json::to_string(value).unwrap()
        );
        let p = Project::new().file("in.json", &json);
        let out = convert(
            &p,
            &["--from", "json", "--to", "dotenv", "--env", "in.json"],
        );
        out.assert_code(0);
        let vars = parse(&out.stdout);
        assert_eq!(
            vars,
            [
                (format!("K{i}"), (*value).to_string()),
                ("AFTER".into(), "x".into())
            ],
            "{value:?} did not survive; the file was:\n{}",
            out.stdout
        );
    }
}

#[test]
fn wp14_from_json_stdin() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &["convert", "--from", "json", "--to", "dotenv", "--env", "-"],
        &[],
        Some(br#"{"B": "two words", "A": "1"}"#),
    );
    out.assert_code(0);
    assert_eq!(out.stdout.trim_end(), "B=\"two words\"\nA=1");
}

#[test]
fn wp14_dotenv_from_stdin_is_converted_too() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &["convert", "--to", "json", "--env", "-"],
        &[],
        Some(b"A=1\nB='x y'\n"),
    );
    out.assert_code(0);
    let json: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(json["A"], "1");
    assert_eq!(json["B"], "x y");
}

/// Nothing piped is an error that says so, not an empty conversion.
#[test]
fn wp14_env_dash_with_nothing_piped_is_refused() {
    let p = Project::new();
    let out = run_with(
        p.path(),
        &["convert", "--to", "json", "--env", "-"],
        &[],
        Some(b""),
    );
    out.assert_code(2).assert_contains("nothing was piped");
}

#[test]
fn wp14_from_json_nested_is_error_without_value() {
    let p = Project::new().file("in.json", r#"{"DB": {"password": "hunter2-secret"}}"#);
    let out = convert(
        &p,
        &["--from", "json", "--to", "dotenv", "--env", "in.json"],
    );
    out.assert_code(2).assert_contains("DB");
    assert!(!out.all().contains("hunter2-secret"), "{}", out.all());
}

#[test]
fn wp14_from_json_number_rejected_names_key() {
    let p = Project::new().file("in.json", r#"{"PORT": 8080, "OTHER": "x"}"#);
    let out = convert(
        &p,
        &["--from", "json", "--to", "dotenv", "--env", "in.json"],
    );
    out.assert_code(2).assert_contains("PORT");
    assert!(!out.all().contains("8080"), "{}", out.all());
}

#[test]
fn wp14_from_json_not_json_is_exit_2() {
    let p = Project::new().file("in.json", "A=1\n");
    convert(
        &p,
        &["--from", "json", "--to", "dotenv", "--env", "in.json"],
    )
    .assert_code(2)
    .assert_contains("--from json");
}

#[test]
fn wp14_from_json_missing_file_says_so() {
    let p = Project::new();
    convert(
        &p,
        &["--from", "json", "--to", "dotenv", "--env", "nope.json"],
    )
    .assert_code(2)
    .assert_contains("nope.json does not exist");
}

#[test]
fn wp14_to_dotenv_same_as_from_dotenv_is_idempotent() {
    let p = Project::new().env(TRICKY);
    let once = convert(&p, &["--to", "dotenv"]);
    once.assert_code(0);
    let p = p.file("once.env", &once.stdout);
    let twice = convert(&p, &["--env", "once.env", "--to", "dotenv"]);
    twice.assert_code(0);
    assert_eq!(once.stdout, twice.stdout);
    assert_eq!(parse(TRICKY), parse(&once.stdout));
}

#[test]
fn wp14_to_dotenv_writes_a_private_file() {
    let p = Project::new().env("A=1\n");
    let out = convert(&p, &["--to", "dotenv", "-o", "copy.env", "--no-gitignore"]);
    out.assert_code(0);
    assert_eq!(p.read("copy.env"), "A=1\n");
}

#[test]
fn wp14_stdin_input_never_collides_with_output() {
    let p = Project::new().file("taken.env", "OLD=1\n");
    let out = run_with(
        p.path(),
        &["convert", "--to", "dotenv", "--env", "-", "-o", "taken.env"],
        &[],
        Some(b"A=1\n"),
    );
    out.assert_code(2).assert_contains("already exists");
    assert_eq!(p.read("taken.env"), "OLD=1\n");
}

// ── --kind ─────────────────────────────────────────────────────────────────────

const MIXED: &str = "APP_NAME=demo\nAPI_SECRET=abc123\nPORT=8080\nDB_PASSWORD=pw\n";

#[test]
fn wp14_kind_configmap_shape() {
    let p = Project::new().env(MIXED);
    let out = convert(
        &p,
        &["--to", "kubernetes", "--name", "app", "--kind", "configmap"],
    );
    out.assert_code(0);
    assert!(out
        .stdout
        .starts_with("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: app\n"));
    assert!(
        out.stdout.contains("\ndata:\n  APP_NAME: \"demo\"\n"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("  API_SECRET: \"abc123\""),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("stringData"), "{}", out.stdout);
    assert!(!out.stdout.contains("---"), "{}", out.stdout);
}

#[test]
fn wp14_kind_split_two_documents() {
    let p = Project::new().env(MIXED);
    let out = convert(
        &p,
        &[
            "--to",
            "kubernetes",
            "--name",
            "app",
            "--namespace",
            "prod",
            "--kind",
            "split",
        ],
    );
    out.assert_code(0);
    let docs: Vec<&str> = out.stdout.split("---\n").collect();
    assert_eq!(docs.len(), 2, "{}", out.stdout);
    let (secret, config) = (docs[0], docs[1]);
    assert!(secret.contains("kind: Secret\n"), "{secret}");
    assert!(secret.contains("  name: app\n"), "{secret}");
    assert!(secret.contains("  namespace: prod\n"), "{secret}");
    assert!(secret.contains("stringData:"), "{secret}");
    assert!(
        secret.contains("API_SECRET") && secret.contains("DB_PASSWORD"),
        "{secret}"
    );
    assert!(
        !secret.contains("APP_NAME") && !secret.contains("PORT"),
        "{secret}"
    );
    assert!(config.contains("kind: ConfigMap\n"), "{config}");
    assert!(config.contains("  name: app-config\n"), "{config}");
    assert!(config.contains("  namespace: prod\n"), "{config}");
    assert!(
        config.contains("APP_NAME") && config.contains("PORT"),
        "{config}"
    );
    assert!(
        !config.contains("API_SECRET") && !config.contains("DB_PASSWORD"),
        "{config}"
    );
}

#[test]
fn wp14_kind_split_no_secrets_one_document() {
    let p = Project::new().env("APP_NAME=demo\nPORT=8080\n");
    let out = convert(
        &p,
        &["--to", "kubernetes", "--name", "app", "--kind", "split"],
    );
    out.assert_code(0);
    assert!(!out.stdout.contains("---"), "{}", out.stdout);
    assert!(!out.stdout.contains("kind: Secret"), "{}", out.stdout);
    assert!(out.stdout.contains("kind: ConfigMap"), "{}", out.stdout);
}

#[test]
fn wp14_kind_split_only_secrets_one_document() {
    let p = Project::new().env("API_SECRET=abc\nDB_PASSWORD=pw\n");
    let out = convert(
        &p,
        &["--to", "kubernetes", "--name", "app", "--kind", "split"],
    );
    out.assert_code(0);
    assert!(!out.stdout.contains("---"), "{}", out.stdout);
    assert!(!out.stdout.contains("ConfigMap"), "{}", out.stdout);
}

#[test]
fn wp14_kind_split_base64_encodes_only_the_secret() {
    let p = Project::new().env("API_SECRET=abc\nPORT=8080\n");
    let out = convert(
        &p,
        &[
            "--to",
            "kubernetes",
            "--name",
            "app",
            "--kind",
            "split",
            "--base64",
        ],
    );
    out.assert_code(0);
    // base64("abc") = YWJj
    assert!(
        out.stdout.contains("data:\n  API_SECRET: \"YWJj\""),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("  PORT: \"8080\""), "{}", out.stdout);
}

#[test]
fn wp14_kind_with_base64_configmap_usage_2() {
    let p = Project::new().env(MIXED);
    convert(
        &p,
        &[
            "--to",
            "kubernetes",
            "--name",
            "app",
            "--kind",
            "configmap",
            "--base64",
        ],
    )
    .assert_code(2)
    .assert_contains("--base64");
}

#[test]
fn wp14_kind_default_is_the_secret_as_before() {
    let p = Project::new().env("A=1\n");
    let plain = convert(&p, &["--to", "kubernetes", "--name", "app"]);
    let explicit = convert(
        &p,
        &["--to", "kubernetes", "--name", "app", "--kind", "secret"],
    );
    plain.assert_code(0);
    assert_eq!(plain.stdout, explicit.stdout);
    assert!(plain.stdout.contains("kind: Secret"));
}

#[test]
fn wp14_kind_refused_for_other_targets() {
    let p = Project::new().env("A=1\n");
    convert(&p, &["--to", "json", "--kind", "split"])
        .assert_code(2)
        .assert_contains("--kind is for --to kubernetes");
}

// ── CONV-5: each format's flags are for that format only ───────────────────────

/// Every flag that belongs to one format, with a value its own name rule
/// accepts, against every other format.
#[test]
fn wp14_each_target_flag_refused_for_every_other_format() {
    let flags: [(&[&str], &str); 7] = [
        (&["--project", "my-project-1"], "gcp-secrets"),
        (&["--vault-name", "myvault"], "azure-keyvault"),
        (&["--heroku-app", "my-app"], "heroku"),
        (&["--name", "myapp"], "kubernetes"),
        (&["--namespace", "prod"], "kubernetes"),
        (&["--terraform-declarations"], "terraform"),
        (&["--kind", "split"], "kubernetes"),
    ];
    let p = Project::new().env("A=1\n");
    for (flag, belongs) in flags {
        for format in evnx::formats::FORMATS {
            let to = evnx::formats::convert_name(format.name);
            let mut args = vec!["--to", to];
            args.extend_from_slice(flag);
            let out = convert(&p, &args);
            if format.name == belongs {
                // Its own format never refuses it for being the wrong one.
                assert!(
                    !out.all().contains("does not use it"),
                    "{} refused for --to {to}:\n{}",
                    flag[0],
                    out.all()
                );
            } else {
                out.assert_code(2);
                assert!(
                    out.stderr
                        .contains(&format!("{} is for --to {belongs}", flag[0])),
                    "{} for --to {to}:\n{}",
                    flag[0],
                    out.stderr
                );
            }
        }
    }
}

#[test]
fn wp14_short_names_still_accepted() {
    let p = Project::new().env("A=1\n");
    for (short, marker) in [
        ("yml", "A: "),
        ("k8s", "kind: Secret"),
        ("tf", "A = "),
        ("compose", "environment:"),
        ("bash", "export A="),
        ("env", "A=1"),
    ] {
        let mut args = vec!["--to", short];
        if short == "k8s" {
            args.extend(["--name", "app"]);
        }
        let out = convert(&p, &args);
        out.assert_code(0);
        assert!(out.stdout.contains(marker), "{short}:\n{}", out.stdout);
    }
}

#[test]
fn wp14_convert_help_says_file_versus_provider() {
    let p = Project::new();
    run(p.path(), &["convert", "--help"])
        .assert_code(0)
        .assert_contains("convert writes a file; migrate talks to a provider");
}
