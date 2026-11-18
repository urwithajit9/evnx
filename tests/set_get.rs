//! `evnx set` and `evnx get`: one variable, in place.
//!
//! What is pinned:
//!
//! * `set` changes exactly one line of a file with comments, `export` and
//!   every quote style (a golden file), keeps the mode, and leaves one `.bak`;
//! * a secret never shows in output, in the warning about the command line, in
//!   the `.bak` message or in `--json`;
//! * `PORT=$(evnx get PORT)` works in a shell;
//! * `get` refuses a secret without `--reveal` (exit 4), also piped;
//! * `--if-missing`, `--dry-run`, `--exists`, and every exit code.
//!
//! The golden file is rewritten with `UPDATE_GOLDEN=1`.

mod support;

use support::snapshot::check_golden;
use support::{Project, evnx_bin, run, run_with};

/// A file with a comment on every other line, `export`, all three quote
/// styles, spacing around `=`, an inline comment, a multiline value, CRLF-free.
const FIXTURE: &str = "\
# Web
export PORT=\"3000\" # the web port
HOST = localhost

# Quoting
SINGLE='keep $this literal'
DOUBLE=\"two words\"
BARE=plain
MULTI=\"line one
line two\"

# Secrets
API_TOKEN=old-token-value-1111
";

fn project() -> Project {
    Project::new().env(FIXTURE)
}

/// The lines of `after` that are not in `before`, and the other way round.
fn changed_lines(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    (
        a.iter()
            .filter(|l| !b.contains(l))
            .map(|l| l.to_string())
            .collect(),
        b.iter()
            .filter(|l| !a.contains(l))
            .map(|l| l.to_string())
            .collect(),
    )
}

// ── set: one line ────────────────────────────────────────────────────────

#[test]
fn set_changes_exactly_one_line_and_keeps_the_rest() {
    let p = project();
    run(p.path(), &["set", "PORT=8080", "--yes"]).assert_code(0);
    let after = p.read(".env");
    check_golden("set/port_changed.env", &after);
    let (added, removed) = changed_lines(FIXTURE, &after);
    assert_eq!(added, ["export PORT=\"8080\" # the web port"]);
    assert_eq!(removed, ["export PORT=\"3000\" # the web port"]);
    assert_eq!(after.lines().count(), FIXTURE.lines().count());
}

#[test]
fn set_keeps_each_quote_style() {
    for (name, value, want) in [
        ("SINGLE", "other", "SINGLE='other'"),
        ("DOUBLE", "x", "DOUBLE=\"x\""),
        ("BARE", "v2", "BARE=v2"),
        ("HOST", "example.org", "HOST = example.org"),
        ("BARE", "has space", "BARE=\"has space\""),
    ] {
        let p = project();
        run(p.path(), &["set", &format!("{name}={value}"), "--yes"]).assert_code(0);
        let (added, _) = changed_lines(FIXTURE, &p.read(".env"));
        assert_eq!(added, [want], "{name}");
    }
}

#[test]
fn set_replaces_a_multiline_value_whole() {
    let p = project();
    run(p.path(), &["set", "MULTI=one", "--yes"]).assert_code(0);
    let after = p.read(".env");
    assert!(after.contains("MULTI=\"one\"\n\n# Secrets"), "{after}");
    assert!(!after.contains("line two"), "{after}");
}

#[test]
fn set_appends_a_missing_name_at_the_end() {
    let p = project();
    run(p.path(), &["set", "NEW_NAME=1"]).assert_code(0);
    assert_eq!(p.read(".env"), format!("{FIXTURE}NEW_NAME=1\n"));
}

#[test]
fn set_adds_the_newline_a_file_was_missing() {
    let p = Project::new().env("A=1");
    run(p.path(), &["set", "B=2"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
}

#[test]
fn set_keeps_crlf() {
    let p = Project::new().env("A=1\r\nB=2\r\n");
    run(p.path(), &["set", "A=9", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=9\r\nB=2\r\n");
}

#[test]
fn set_picks_the_file_by_name_or_path_and_folder() {
    let p = Project::new()
        .env("A=1\n")
        .file(".env.staging", "A=1\n")
        .file("deploy.env", "A=1\n")
        .dir("sub")
        .file("sub/.env", "A=1\n");
    run(p.path(), &["set", "A=2", "--env-name", "staging", "--yes"]).assert_code(0);
    run(p.path(), &["set", "A=3", "--env", "deploy.env", "--yes"]).assert_code(0);
    run(p.path(), &["set", "A=4", "-C", "sub", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\n");
    assert_eq!(p.read(".env.staging"), "A=2\n");
    assert_eq!(p.read("deploy.env"), "A=3\n");
    assert_eq!(p.read("sub/.env"), "A=4\n");
}

#[test]
fn set_in_a_missing_file_is_exit_2_with_the_way_out() {
    let p = Project::new();
    let out = run(p.path(), &["set", "A=1"]);
    out.assert_code(2).assert_contains(".env not found");
    assert!(!p.exists(".env"));
}

#[test]
fn set_refuses_a_file_it_cannot_read_whole_and_changes_nothing() {
    let p = Project::new().env("A=1\nthis is not an assignment\n");
    let out = run(p.path(), &["set", "A=2", "--yes"]);
    out.assert_code(2).assert_contains("Line 2");
    assert_eq!(p.read(".env"), "A=1\nthis is not an assignment\n");
    assert!(!p.exists(".env.bak"));
}

#[test]
fn set_names_must_be_names() {
    let p = project();
    run(p.path(), &["set", "bad name=1"]).assert_code(2);
    run(p.path(), &["set", "=1"]).assert_code(2);
    assert_eq!(p.read(".env"), FIXTURE);
}

#[test]
fn set_with_the_same_value_writes_nothing() {
    let p = project();
    let out = run(p.path(), &["set", "BARE=plain"]);
    out.assert_code(0)
        .assert_contains("already holds that value");
    assert!(!p.exists(".env.bak"));
    assert_eq!(p.read(".env"), FIXTURE);
}

#[test]
fn set_warns_when_a_name_is_assigned_twice_and_changes_the_last() {
    let p = Project::new().env("A=1\nB=2\nA=3\n");
    run(p.path(), &["set", "A=9", "--yes"])
        .assert_code(0)
        .assert_contains("assigned 2 times");
    assert_eq!(p.read(".env"), "A=1\nB=2\nA=9\n");
}

// ── set: write, mode, backup, side effects ───────────────────────────────

#[cfg(unix)]
#[test]
fn set_keeps_the_mode_and_leaves_one_private_bak() {
    use std::os::unix::fs::PermissionsExt;
    let p = Project::new().env("A=1\n").mode(".env", 0o640);
    run(p.path(), &["set", "A=2", "--yes"]).assert_code(0);
    let mode = |f: &str| std::fs::metadata(p.join(f)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(".env"), 0o640);
    assert_eq!(mode(".env.bak"), 0o600);
    assert_eq!(p.read(".env.bak"), "A=1\n");
    // A second set replaces the one copy; no numbered pile.
    run(p.path(), &["set", "A=3", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env.bak"), "A=2\n");
    let baks = std::fs::read_dir(p.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".bak"))
        .count();
    assert_eq!(baks, 1);
}

#[cfg(unix)]
#[test]
fn set_writes_through_a_symlink() {
    let p = Project::new()
        .file("real.env", "A=1\n")
        .symlink("real.env", ".env");
    run(p.path(), &["set", "A=2", "--yes"]).assert_code(0);
    assert!(
        std::fs::symlink_metadata(p.join(".env"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(p.read("real.env"), "A=2\n");
}

#[test]
fn set_touches_no_template_contract_or_gitignore() {
    let p = Project::new()
        .env("A=1\n")
        .example("A=\n")
        .toml("[vars.A]\nrequired = true\n")
        .file(".gitignore", ".env\n");
    run(p.path(), &["set", "NEW=1", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env.example"), "A=\n");
    assert_eq!(p.read(".evnx.toml"), "[vars.A]\nrequired = true\n");
    assert_eq!(p.read(".gitignore"), ".env\n");
}

// ── set: asking, and where the value comes from ──────────────────────────

#[test]
fn replacing_a_value_without_a_terminal_and_without_yes_is_exit_4() {
    let p = project();
    let out = run(p.path(), &["set", "PORT=8080"]);
    out.assert_code(4).assert_contains("--yes");
    assert_eq!(p.read(".env"), FIXTURE);
    assert!(!p.exists(".env.bak"));
}

#[test]
fn a_new_name_needs_no_yes() {
    let p = project();
    run(p.path(), &["set", "BRAND_NEW=1"]).assert_code(0);
}

#[test]
fn no_value_and_no_terminal_is_exit_4_and_names_stdin() {
    let p = project();
    let out = run(p.path(), &["set", "API_TOKEN"]);
    out.assert_code(4).assert_contains("evnx set API_TOKEN");
    assert!(out.all().contains("printf %s"), "{}", out.all());
    assert_eq!(p.read(".env"), FIXTURE);
}

#[test]
fn a_value_on_stdin_is_used_without_its_line_ending() {
    let p = project();
    run_with(p.path(), &["set", "PORT", "--yes"], &[], Some(b"9090\n")).assert_code(0);
    assert!(
        p.read(".env")
            .contains("export PORT=\"9090\" # the web port")
    );
}

#[test]
fn if_missing_leaves_a_present_name_alone_and_adds_an_absent_one() {
    let p = project();
    let out = run(p.path(), &["set", "PORT=1", "--if-missing"]);
    out.assert_code(0).assert_contains("left as it is");
    assert_eq!(p.read(".env"), FIXTURE);
    assert!(!p.exists(".env.bak"));
    run(p.path(), &["set", "FRESH=1", "--if-missing"]).assert_code(0);
    assert!(p.read(".env").ends_with("FRESH=1\n"));
}

#[test]
fn if_missing_asks_for_no_value() {
    // No value, no terminal: would be exit 4 if it asked.
    let p = project();
    run(p.path(), &["set", "API_TOKEN", "--if-missing"]).assert_code(0);
}

#[test]
fn dry_run_writes_nothing_and_needs_no_yes() {
    let p = project();
    let out = run(p.path(), &["set", "PORT=8080", "--dry-run"]);
    out.assert_code(0)
        .assert_contains("Dry run: nothing was written")
        .assert_contains("3000 → 8080");
    assert_eq!(p.read(".env"), FIXTURE);
    assert!(!p.exists(".env.bak"));
}

#[test]
fn output_is_old_arrow_new() {
    let p = project();
    run(p.path(), &["set", "BARE=next", "--yes"])
        .assert_code(0)
        .assert_contains("BARE  plain → next");
}

// ── secrets ──────────────────────────────────────────────────────────────

const OLD: &str = "old-token-value-1111";
const NEW: &str = "brand-new-secret-9f3a7c";

#[test]
fn a_secret_name_is_hidden_in_every_output() {
    for args in [
        vec!["set", "API_TOKEN=brand-new-secret-9f3a7c", "--yes"],
        vec!["set", "API_TOKEN=brand-new-secret-9f3a7c", "--dry-run"],
        vec![
            "set",
            "API_TOKEN=brand-new-secret-9f3a7c",
            "--yes",
            "--json",
        ],
        vec![
            "set",
            "API_TOKEN=brand-new-secret-9f3a7c",
            "--dry-run",
            "--json",
        ],
        vec!["set", "API_TOKEN=brand-new-secret-9f3a7c", "--yes", "-v"],
    ] {
        let p = project();
        let out = run(p.path(), &args);
        out.assert_code(0)
            .assert_not_contains(NEW)
            .assert_not_contains(OLD);
        assert!(
            out.all().contains("value hidden") || args.contains(&"--json"),
            "{args:?}\n{}",
            out.all()
        );
    }
}

#[test]
fn a_secret_given_on_the_command_line_warns_about_history_without_the_value() {
    let p = project();
    let out = run(
        p.path(),
        &["set", "API_TOKEN=brand-new-secret-9f3a7c", "--yes"],
    );
    out.assert_code(0).assert_contains("shell history");
    assert!(!out.stderr.contains(NEW), "{}", out.stderr);
    // A name that is not secret does not warn.
    let p = project();
    let out = run(p.path(), &["set", "PORT=1", "--yes"]);
    assert!(!out.all().contains("history"), "{}", out.all());
}

#[test]
fn a_secret_by_value_under_a_plain_name_is_hidden_too() {
    let p = Project::new().env("UPSTREAM=http://x\n");
    let out = run(
        p.path(),
        &["set", "UPSTREAM=https://user:s3cretpass@host/db", "--yes"],
    );
    out.assert_code(0)
        .assert_not_contains("s3cretpass")
        .assert_contains("value hidden");
}

#[test]
fn a_secret_from_stdin_is_not_echoed_and_not_in_the_warning() {
    let p = project();
    let out = run_with(
        p.path(),
        &["set", "API_TOKEN", "--yes"],
        &[],
        Some(format!("{NEW}\n").as_bytes()),
    );
    out.assert_code(0).assert_not_contains(NEW);
    assert!(!out.all().contains("history"), "stdin is not shell history");
    assert!(p.read(".env").contains(&format!("API_TOKEN={NEW}")));
    // The .bak message is a path, never a value.
    out.assert_contains(".env.bak");
}

#[test]
fn json_has_names_and_statuses_and_no_value() {
    let p = project();
    let out = run(p.path(), &["set", "PORT=8081", "--yes", "--json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["run"]["tool"], "evnx");
    assert_eq!(doc["name"], "PORT");
    assert_eq!(doc["status"], "changed");
    assert_eq!(doc["secret"], false);
    assert_eq!(doc["dry_run"], false);
    assert!(
        !out.stdout.contains("8081"),
        "no value in JSON: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("3000"),
        "no value in JSON: {}",
        out.stdout
    );
    let added = run(p.path(), &["set", "ADDED=1", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&added.stdout).unwrap();
    assert_eq!(doc["status"], "added");
    let skipped = run(p.path(), &["set", "PORT=1", "--if-missing", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&skipped.stdout).unwrap();
    assert_eq!(doc["status"], "skipped");
}

#[test]
fn json_that_would_replace_a_value_needs_yes() {
    let p = project();
    run(p.path(), &["set", "PORT=1", "--json"]).assert_code(4);
    assert_eq!(p.read(".env"), FIXTURE);
}

// ── get ──────────────────────────────────────────────────────────────────

#[test]
fn get_prints_the_value_alone() {
    let p = project();
    for (name, want) in [
        ("PORT", "3000\n"),
        ("HOST", "localhost\n"),
        ("SINGLE", "keep $this literal\n"),
        ("DOUBLE", "two words\n"),
        ("MULTI", "line one\nline two\n"),
    ] {
        let out = run(p.path(), &["get", name]);
        out.assert_code(0);
        assert_eq!(out.stdout, want, "{name}");
        assert_eq!(out.stderr, "", "{name}");
    }
}

#[cfg(unix)]
#[test]
fn a_shell_can_capture_it() {
    let p = project();
    let out = std::process::Command::new("sh")
        .current_dir(p.path())
        .env("EVNX", evnx_bin())
        .env("NO_COLOR", "1")
        .args(["-c", "PORT=$(\"$EVNX\" get PORT) && echo \"[$PORT]\""])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[3000]\n");
}

#[test]
fn get_expands_references_as_run_would() {
    let p = Project::new().env("HOST=db\nURL=http://${HOST}/x\n");
    assert_eq!(run(p.path(), &["get", "URL"]).stdout, "http://db/x\n");
}

#[test]
fn get_reads_the_last_of_two_assignments() {
    let p = Project::new().env("A=1\nA=2\n");
    assert_eq!(run(p.path(), &["get", "A"]).stdout, "2\n");
}

#[test]
fn get_a_secret_is_exit_4_without_reveal_even_piped() {
    let p = project();
    let out = run(p.path(), &["get", "API_TOKEN"]);
    out.assert_code(4).assert_contains("--reveal");
    assert_eq!(out.stdout, "");
    assert!(!out.all().contains(OLD), "{}", out.all());
    // stdout is captured by the harness, so this is the piped case.
    let ok = run(p.path(), &["get", "API_TOKEN", "--reveal"]);
    ok.assert_code(0);
    assert_eq!(ok.stdout, format!("{OLD}\n"));
}

#[test]
fn a_secret_by_value_is_refused_under_a_plain_name() {
    let p = Project::new().env("UPSTREAM=https://user:s3cretpass@host/db\n");
    let out = run(p.path(), &["get", "UPSTREAM"]);
    out.assert_code(4).assert_not_contains("s3cretpass");
}

#[test]
fn get_exists_prints_nothing_and_says_it_in_the_exit_code() {
    let p = project();
    for (name, code) in [("PORT", 0), ("API_TOKEN", 0), ("NOPE", 1)] {
        let out = run(p.path(), &["get", "--exists", name]);
        out.assert_code(code);
        assert_eq!(
            (out.stdout.as_str(), out.stderr.as_str()),
            ("", ""),
            "{name}"
        );
    }
}

#[test]
fn get_of_a_missing_name_is_exit_1_with_a_message() {
    let p = project();
    let out = run(p.path(), &["get", "NOPE"]);
    out.assert_code(1).assert_contains("NOPE is not in");
    assert_eq!(out.stdout, "");
}

#[test]
fn get_without_a_file_is_exit_2() {
    let p = Project::new();
    run(p.path(), &["get", "A"]).assert_code(2);
    run(p.path(), &["get", "--exists", "A"]).assert_code(2);
}

#[test]
fn get_with_an_unreadable_line_is_exit_2_not_missing() {
    let p = Project::new().env("A=1\nnot an assignment\n");
    run(p.path(), &["get", "B"]).assert_code(2);
}

#[test]
fn get_json_never_has_a_secret_unless_revealed() {
    let p = project();
    let out = run(p.path(), &["get", "API_TOKEN", "--json"]);
    out.assert_code(4);
    assert!(!out.all().contains(OLD), "{}", out.all());
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        (doc["found"].clone(), doc["secret"].clone()),
        (true.into(), true.into())
    );
    assert!(doc["value"].is_null());

    let shown = run(p.path(), &["get", "API_TOKEN", "--json", "--reveal"]);
    shown.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(doc["value"], OLD);

    let plain = run(p.path(), &["get", "PORT", "--format", "json"]);
    plain.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&plain.stdout).unwrap();
    assert_eq!(
        (doc["value"].clone(), doc["secret"].clone()),
        ("3000".into(), false.into())
    );
    assert_eq!(doc["run"]["tool"], "evnx");

    let missing = run(p.path(), &["get", "NOPE", "--json"]);
    missing.assert_code(1);
    let doc: serde_json::Value = serde_json::from_str(&missing.stdout).unwrap();
    assert_eq!(doc["found"], false);
}

#[test]
fn get_from_a_named_environment() {
    let p = Project::new().env("A=1\n").file(".env.staging", "A=2\n");
    assert_eq!(
        run(p.path(), &["get", "A", "--env-name", "staging"]).stdout,
        "2\n"
    );
    assert_eq!(
        run(p.path(), &["get", "A", "--env", ".env.staging"]).stdout,
        "2\n"
    );
}

#[test]
fn get_exists_does_not_combine_with_reveal_or_json() {
    let p = project();
    run(p.path(), &["get", "PORT", "--exists", "--reveal"]).assert_code(2);
    run(p.path(), &["get", "PORT", "--exists", "--json"]).assert_code(2);
}
