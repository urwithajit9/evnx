//! `evnx add custom NAME --from-file <path|->` and `evnx set NAME --from-file`:
//! one value, several lines long, from a file or a pipe.
//!
//! What is pinned: the value is read back by the parser exactly (a PEM block
//! and a service-account JSON, whose private key has `\n`, quotes and `$`);
//! the 64 KiB cap, one final newline, CRLF, base64 and non-UTF-8 input; one
//! NAME and no `NAME=value`; an existing name is kept; a dry run says sizes and
//! never the value; the warning for a source file git would commit.

mod support;

use evnx::core::Parser;
use support::{Project, run, run_with};

const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nKcwggSjAgEAAoIBAQC7VJTUt9Us8cKj\n-----END PRIVATE KEY-----";

fn sa_json() -> String {
    serde_json::json!({
        "type": "service_account",
        "project_id": "demo-123",
        "private_key": "-----BEGIN PRIVATE KEY-----\nabc$HOME\\d\"e\n-----END PRIVATE KEY-----\n",
        "client_email": "svc@demo-123.iam.example.com",
    })
    .to_string()
}

fn value_of(p: &Project, file: &str, name: &str) -> String {
    let parsed = Parser::default()
        .parse_content(&p.read(file))
        .expect("the written file parses");
    parsed
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not in {file}"))
        .expose()
        .to_string()
}

#[test]
fn a_pem_block_reads_back_exactly() {
    let p = Project::new()
        .env("A=1\n")
        .file("key.pem", &format!("{PEM}\n"));
    run(
        p.path(),
        &[
            "add",
            "custom",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "TLS_KEY"), PEM);
    // One line, escaped: the file is still one assignment per line.
    assert!(
        p.read(".env").contains("TLS_KEY=\"-----BEGIN"),
        "{}",
        p.read(".env")
    );
}

#[test]
fn a_service_account_json_reads_back_exactly() {
    let json = sa_json();
    let p = Project::new().env("A=1\n").file("sa.json", &json);
    run(
        p.path(),
        &[
            "add",
            "custom",
            "GOOGLE_CREDENTIALS",
            "--from-file",
            "sa.json",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "GOOGLE_CREDENTIALS"), json);
}

#[test]
fn the_template_gets_a_placeholder_never_the_value() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\n")
        .file("key.pem", PEM);
    run(
        p.path(),
        &[
            "add",
            "custom",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--yes",
        ],
    )
    .assert_code(0);
    let example = p.read(".env.example");
    assert!(example.contains("TLS_KEY="), "{example}");
    assert!(!example.contains("BEGIN"), "{example}");
}

#[test]
fn set_from_file_replaces_the_value_and_reads_back() {
    let json = sa_json();
    let p = Project::new()
        .env("# keep\nGOOGLE_CREDENTIALS=old\nB=2\n")
        .file("sa.json", &json);
    run(
        p.path(),
        &[
            "set",
            "GOOGLE_CREDENTIALS",
            "--from-file",
            "sa.json",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "GOOGLE_CREDENTIALS"), json);
    assert_eq!(value_of(&p, ".env", "B"), "2");
    assert!(p.read(".env").starts_with("# keep\n"));
}

#[test]
fn set_from_file_reads_stdin_with_a_dash() {
    let p = Project::new().env("A=1\n");
    run_with(
        p.path(),
        &["set", "TLS_KEY", "--from-file", "-", "--yes"],
        &[],
        Some(format!("{PEM}\n").as_bytes()),
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "TLS_KEY"), PEM);
}

#[test]
fn add_custom_reads_stdin_with_a_dash() {
    let p = Project::new().env("A=1\n");
    run_with(
        p.path(),
        &["add", "custom", "TLS_KEY", "--from-file", "-", "--yes"],
        &[],
        Some(PEM.as_bytes()),
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "TLS_KEY"), PEM);
}

#[test]
fn one_final_newline_goes_and_crlf_inside_stays() {
    let p = Project::new().env("A=1\n").file("v.txt", "a\r\nb\r\n");
    run(
        p.path(),
        &["add", "custom", "V", "--from-file", "v.txt", "--yes"],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "V"), "a\r\nb");
}

#[test]
fn keep_trailing_newline_keeps_it() {
    let p = Project::new().env("A=1\n").file("v.txt", "a\nb\n");
    run(
        p.path(),
        &[
            "add",
            "custom",
            "V",
            "--from-file",
            "v.txt",
            "--keep-trailing-newline",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "V"), "a\nb\n");
}

#[test]
fn the_cap_is_64_kib() {
    let p = Project::new()
        .env("A=1\n")
        .file("ok.txt", &"a".repeat(64 * 1024))
        .file("big.txt", &"a".repeat(70 * 1024));
    run(
        p.path(),
        &["add", "custom", "OK", "--from-file", "ok.txt", "--yes"],
    )
    .assert_code(0);
    let before = p.read(".env");
    let big = run(
        p.path(),
        &["add", "custom", "BIG", "--from-file", "big.txt", "--yes"],
    );
    big.assert_code(2);
    big.assert_contains("is 70 KiB; the limit is 64 KiB");
    assert_eq!(p.read(".env"), before, "a refused run wrote something");
    // The same for set.
    run(p.path(), &["set", "BIG", "--from-file", "big.txt", "--yes"]).assert_code(2);
}

#[test]
fn binary_needs_base64() {
    let p = Project::new().env("A=1\n");
    std::fs::write(p.join("k.bin"), [0xffu8, 0xfe, 0x00, 0x01]).unwrap();
    let refused = run(
        p.path(),
        &["add", "custom", "K", "--from-file", "k.bin", "--yes"],
    );
    refused.assert_code(2);
    refused.assert_contains("--base64");
    run(
        p.path(),
        &[
            "add",
            "custom",
            "K",
            "--from-file",
            "k.bin",
            "--base64",
            "--yes",
        ],
    )
    .assert_code(0);
    assert_eq!(value_of(&p, ".env", "K"), "//4AAQ==");
}

#[test]
fn base64_without_from_file_is_a_usage_error() {
    let p = Project::new().env("A=1\n");
    run(p.path(), &["add", "custom", "K", "--base64", "--yes"]).assert_code(2);
}

#[test]
fn exactly_one_name_and_no_assignment() {
    let p = Project::new().env("A=1\n").file("v.txt", "v");
    for args in [
        vec!["add", "custom", "X=1", "--from-file", "v.txt", "--yes"],
        vec!["add", "custom", "X", "Y", "--from-file", "v.txt", "--yes"],
        vec!["add", "custom", "--from-file", "v.txt", "--yes"],
        vec!["set", "X=1", "--from-file", "v.txt", "--yes"],
    ] {
        run(p.path(), &args).assert_code(2);
    }
    assert_eq!(p.read(".env"), "A=1\n");
}

#[test]
fn an_empty_file_and_a_missing_file_are_refused() {
    let p = Project::new().env("A=1\n").file("empty.txt", "\n");
    run(
        p.path(),
        &["add", "custom", "X", "--from-file", "empty.txt", "--yes"],
    )
    .assert_code(2);
    run(
        p.path(),
        &["add", "custom", "X", "--from-file", "nope.txt", "--yes"],
    )
    .assert_code(2);
}

#[test]
fn an_existing_name_is_kept_and_no_value_is_printed() {
    let p = Project::new()
        .env("TLS_KEY=old-value-canary\n")
        .file("key.pem", PEM);
    let out = run(
        p.path(),
        &[
            "add",
            "custom",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--yes",
        ],
    );
    out.assert_code(0);
    assert_eq!(p.read(".env"), "TLS_KEY=old-value-canary\n");
    let all = out.all();
    assert!(
        !all.contains("old-value-canary") && !all.contains("BEGIN"),
        "{all}"
    );
}

#[test]
fn a_dry_run_says_the_size_and_never_the_value() {
    let p = Project::new()
        .env("A=1\n")
        .file("key.pem", &format!("{PEM}\n"));
    let out = run(
        p.path(),
        &[
            "add",
            "custom",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--dry-run",
        ],
    );
    out.assert_code(0);
    out.assert_contains("TLS_KEY: 118 bytes, 4 lines");
    assert!(!out.all().contains("MIIEvQ"), "{}", out.all());
    assert_eq!(p.read(".env"), "A=1\n");

    // set: the same, and the JSON has no value either.
    let set = run(
        p.path(),
        &["set", "TLS_KEY", "--from-file", "key.pem", "--dry-run"],
    );
    set.assert_code(0);
    set.assert_contains("value hidden; 118 bytes, 4 lines");
    assert!(!set.all().contains("MIIEvQ"));
    let json = run(
        p.path(),
        &[
            "set",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--dry-run",
            "--json",
        ],
    );
    json.assert_code(0);
    assert!(!json.all().contains("MIIEvQ"));
    let add_json = run(
        p.path(),
        &[
            "add",
            "custom",
            "TLS_KEY",
            "--from-file",
            "key.pem",
            "--dry-run",
            "--json",
        ],
    );
    add_json.assert_code(0);
    assert!(!add_json.all().contains("MIIEvQ"));
}

#[test]
fn a_plain_looking_name_still_hides_a_file_value() {
    let p = Project::new().env("NOTE=old\n").file("n.txt", "just text");
    let out = run(p.path(), &["set", "NOTE", "--from-file", "n.txt", "--yes"]);
    out.assert_code(0);
    assert!(!out.all().contains("just text"), "{}", out.all());
    assert!(out.all().contains("value hidden"), "{}", out.all());
}

#[test]
fn warns_when_the_source_file_is_in_the_repo_and_not_ignored() {
    let p = Project::new()
        .git()
        .env("A=1\n")
        .file(".gitignore", ".env\nignored.pem\n")
        .file("loose.pem", PEM)
        .file("ignored.pem", PEM)
        .file("tracked.pem", PEM);
    p.track("tracked.pem");

    let loose = run(
        p.path(),
        &["add", "custom", "K1", "--from-file", "loose.pem", "--yes"],
    );
    loose.assert_code(0);
    loose.assert_contains("git does not ignore it");

    let tracked = run(
        p.path(),
        &["add", "custom", "K2", "--from-file", "tracked.pem", "--yes"],
    );
    tracked.assert_code(0);
    tracked.assert_contains("git rm --cached");

    let ignored = run(
        p.path(),
        &["add", "custom", "K3", "--from-file", "ignored.pem", "--yes"],
    );
    ignored.assert_code(0);
    assert!(
        !ignored.all().contains("git does not ignore") && !ignored.all().contains("git rm"),
        "{}",
        ignored.all()
    );
}

#[test]
fn no_run_prints_the_value() {
    const CANARY: &str = "canary-9f3a1c77-do-not-print";
    let p = Project::new()
        .env("API_TOKEN=old\n")
        .file("v.txt", &format!("line1\n{CANARY}\n"));
    for args in [
        vec!["add", "custom", "NEW_ONE", "--from-file", "v.txt", "--yes"],
        vec![
            "add",
            "custom",
            "NEW_TWO",
            "--from-file",
            "v.txt",
            "--dry-run",
        ],
        vec!["set", "API_TOKEN", "--from-file", "v.txt", "--yes"],
        vec![
            "set",
            "API_TOKEN",
            "--from-file",
            "v.txt",
            "--yes",
            "--json",
        ],
        vec!["set", "OTHER", "--from-file", "v.txt", "--dry-run"],
    ] {
        let out = run(p.path(), &args);
        assert!(!out.all().contains(CANARY), "{args:?}:\n{}", out.all());
    }
}
