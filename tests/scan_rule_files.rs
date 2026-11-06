//! Rule files you write, and `evnx scan rules` (C6; S3, S5).
//!
//! Each test gets its own `EVNX_CONFIG_DIR`, so the personal folder is never
//! the developer's real one.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn rule_file(id: &str, regex: &str) -> String {
    format!(
        "version = 1\n\n[[rules]]\nid = \"{id}\"\nname = \"{id}\"\nregex = '{regex}'\n\
         severity = \"high\"\n"
    )
}

/// (exit code, stdout, stderr) for `evnx <args>` in `project`, with `home`
/// as the personal config folder.
fn run(project: &Path, home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(project)
        .env("NO_COLOR", "1")
        .env("EVNX_CONFIG_DIR", home)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ids(stdout: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(stdout).unwrap();
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn project_personal_and_flag_rules_all_apply() {
    let (p, h, extra) = (
        TempDir::new().unwrap(),
        TempDir::new().unwrap(),
        TempDir::new().unwrap(),
    );
    fs::create_dir_all(p.path().join(".evnx/rules")).unwrap();
    fs::write(
        p.path().join(".evnx/rules/a.toml"),
        rule_file("proj-tok", "PROJ-[0-9]{8}"),
    )
    .unwrap();
    fs::create_dir_all(h.path().join("rules")).unwrap();
    fs::write(
        h.path().join("rules/b.toml"),
        rule_file("mine-tok", "MINE-[0-9]{8}"),
    )
    .unwrap();
    let flag = extra.path().join("c.toml");
    fs::write(&flag, rule_file("flag-tok", "FLAG-[0-9]{8}")).unwrap();
    fs::write(
        p.path().join("notes.txt"),
        "PROJ-12345678 MINE-12345678 FLAG-12345678\n",
    )
    .unwrap();
    let (code, out, err) = run(
        p.path(),
        h.path(),
        &[
            "scan",
            "--format",
            "json",
            "notes.txt",
            "--rules",
            &flag.to_string_lossy(),
        ],
    );
    assert_eq!(code, 1, "{err}");
    let mut found = ids(&out);
    found.sort();
    assert_eq!(found, ["flag-tok", "mine-tok", "proj-tok"]);
    // S3: personal rules are always named.
    assert!(err.contains("+ 1 personal rule(s) from"), "{err}");
}

#[test]
fn a_duplicate_id_is_refused_naming_both_files() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::create_dir_all(p.path().join(".evnx/rules")).unwrap();
    fs::write(
        p.path().join(".evnx/rules/a.toml"),
        rule_file("dup-tok", "AAA-[0-9]{8}"),
    )
    .unwrap();
    fs::write(
        p.path().join(".evnx/rules/b.toml"),
        rule_file("dup-tok", "BBB-[0-9]{8}"),
    )
    .unwrap();
    let (code, _, err) = run(p.path(), h.path(), &["scan", "."]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains(".evnx/rules/a.toml") && err.contains(".evnx/rules/b.toml"),
        "{err}"
    );
}

/// S5: a rule file adds; reusing a core id is an error, not an override.
#[test]
fn a_rule_file_cannot_replace_a_core_rule() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::create_dir_all(p.path().join(".evnx/rules")).unwrap();
    fs::write(
        p.path().join(".evnx/rules/a.toml"),
        rule_file("aws-access-key", "X-[0-9]{8}"),
    )
    .unwrap();
    let (code, _, err) = run(p.path(), h.path(), &["scan", "."]);
    assert_eq!(code, 2);
    assert!(err.contains("evnx's own rules"), "{err}");
}

#[cfg(unix)]
#[test]
fn an_unreadable_personal_folder_is_a_warning_not_an_abort() {
    use std::os::unix::fs::PermissionsExt;
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let dir = h.path().join("rules");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads anything; the case cannot be built there.
    let readable = fs::read_dir(&dir).is_ok();
    fs::write(p.path().join("a.txt"), "nothing here\n").unwrap();
    let (code, _, err) = run(p.path(), h.path(), &["scan", "a.txt"]);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("personal rules skipped"), "{err}");
}

#[test]
fn a_rule_file_over_5000_rules_is_refused() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::create_dir_all(p.path().join(".evnx/rules")).unwrap();
    let mut body = String::from("version = 1\n");
    for i in 0..5001 {
        body.push_str(&format!(
            "[[rules]]\nid = \"r-{i}\"\nname = \"r{i}\"\nregex = 'R{i}-[0-9]{{8}}'\n"
        ));
    }
    fs::write(p.path().join(".evnx/rules/big.toml"), body).unwrap();
    let (code, _, err) = run(p.path(), h.path(), &["scan", "."]);
    assert_eq!(code, 2);
    assert!(err.contains("at most 5000"), "{err}");
}

#[test]
fn rules_new_then_test_then_list() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (code, _, err) = run(p.path(), h.path(), &["scan", "rules", "new", "acme-token"]);
    assert_eq!(code, 0, "{err}");
    let f = p.path().join(".evnx/rules/acme-token.toml");
    let text = fs::read_to_string(&f)
        .unwrap()
        .replace("REPLACE-ME-[A-Z0-9]{32}", "ACME-[A-Z0-9]{16}")
        .replace("positive = []", "positive = [\"ACME-K3X9Q2M7V1ZPAB4C\"]")
        .replace(
            "near_miss = []",
            "near_miss = [\"ACME-K3X9Q2M7\", \"ACME-K3X9Q2M7V1ZPAB4C\"]",
        );
    fs::write(&f, text).unwrap();
    // The second near miss matches: a failure, reported, exit 1.
    let (code, _, err) = run(p.path(), h.path(), &["scan", "rules", "test"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("acme-token: reports near miss"), "{err}");

    let fixed = fs::read_to_string(&f)
        .unwrap()
        .replace(", \"ACME-K3X9Q2M7V1ZPAB4C\"]", "]");
    fs::write(&f, fixed).unwrap();
    let (code, out, err) = run(p.path(), h.path(), &["scan", "rules", "test"]);
    assert_eq!(code, 0, "{out}{err}");

    let (code, out, _) = run(p.path(), h.path(), &["scan", "rules", "list", "--json"]);
    assert_eq!(code, 0);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    let mine = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "acme-token")
        .unwrap();
    assert_eq!(mine["source"], ".evnx/rules/acme-token.toml");
    assert_eq!(mine["severity"], "high");

    // `new` never writes over a file, and refuses a core id.
    assert_eq!(
        run(p.path(), h.path(), &["scan", "rules", "new", "acme-token"]).0,
        2
    );
    assert_eq!(
        run(
            p.path(),
            h.path(),
            &["scan", "rules", "new", "aws-access-key"]
        )
        .0,
        2
    );
}

/// `rules` is a subcommand; a folder of that name is scanned as `./rules`.
#[test]
fn a_folder_named_rules_is_scanned_as_dot_slash_rules() {
    let (p, h) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fs::create_dir_all(p.path().join("rules")).unwrap();
    let key = format!("sk_live_{}", "51Habc1234567890ABCDEFGHq");
    fs::write(p.path().join("rules/x.env"), format!("K={key}\n")).unwrap();
    let (code, out, _) = run(p.path(), h.path(), &["scan", "--format", "json", "./rules"]);
    assert_eq!(code, 1);
    assert_eq!(ids(&out), ["stripe-live-secret-key"]);
}
