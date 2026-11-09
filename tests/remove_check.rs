//! `evnx remove --check NAME…` (N14-11, plan §9 phase 2): read-only, exit 1 when
//! a name is still in any file `remove` would touch, 0 when it is gone, 2 when
//! it cannot tell. Fixture values are sentinels that no output may contain.

mod support;

use support::{run, Project};

fn sentinel() -> String {
    format!("SENTINEL_{}{}", "4d1c", "_beefed")
}

fn evnx(p: &Project, args: &[&str]) -> support::run::Output {
    run(p.path(), args)
}

fn project() -> Project {
    let s = sentinel();
    Project::new()
        .git()
        .file(".gitignore", ".env*\n!.env.example\n")
        .env(&format!("KEEP=1\nDROP={s}\n"))
        .file(
            ".env.staging",
            &format!("# c\nDROP={s}\nKEEP=2\nDROP={s}\n"),
        )
        .example("KEEP=\nDROP=\n")
        .toml("[vars.KEEP]\nrequired = true\n\n[vars.DROP]\nrequired = true\n")
}

fn snapshot(p: &Project) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(p.path()).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        if name != ".git" {
            out.push((name, std::fs::read(e.path()).unwrap_or_default()));
        }
    }
    out.sort();
    out
}

#[test]
fn a_name_still_anywhere_is_exit_1_and_every_place_is_listed() {
    let p = project();
    let before = snapshot(&p);
    let out = evnx(&p, &["remove", "DROP", "--check"]);
    out.assert_code(1)
        .assert_contains("still present")
        .assert_contains("./.env")
        .assert_contains(".env.staging (lines 2, 4)")
        .assert_contains(".env.example")
        .assert_contains("[vars.DROP]")
        .assert_not_contains(&sentinel());
    assert_eq!(snapshot(&p), before, "--check wrote something");
}

#[test]
fn a_name_gone_from_every_place_is_exit_0() {
    let p = project();
    evnx(&p, &["remove", "DROP", "--all", "--yes"]).assert_code(0);
    evnx(&p, &["remove", "DROP", "--check"])
        .assert_code(0)
        .assert_contains("gone from");
    // KEEP is untouched, so it is still found.
    evnx(&p, &["remove", "KEEP", "--check"]).assert_code(1);
}

#[test]
fn one_place_is_enough_to_fail() {
    for (file, content) in [
        (".env", "DROP=1\n"),
        (".env.production", "DROP=1\n"),
        (".env.example", "DROP=\n"),
    ] {
        let p = Project::new().file(file, content);
        evnx(&p, &["remove", "DROP", "--check"]).assert_code(1);
    }
    let p = Project::new().toml("[vars]\nDROP = { required = false }\n");
    evnx(&p, &["remove", "DROP", "--check"])
        .assert_code(1)
        .assert_contains("[vars.DROP]");
}

#[test]
fn globs_and_several_names_are_checked() {
    let p = Project::new().env("TEST_A=1\nTEST_B=2\nOTHER=3\n");
    evnx(&p, &["remove", "TEST_*", "--check"]).assert_code(1);
    evnx(&p, &["remove", "TEST_*", "NOPE", "--check"])
        .assert_code(1)
        .assert_contains("TEST_A")
        .assert_contains("TEST_B")
        .assert_not_contains("OTHER");
    evnx(&p, &["remove", "NOPE", "ALSO_NOPE", "--check"]).assert_code(0);
}

#[test]
fn it_reads_a_read_only_file() {
    let p = Project::new().env("DROP=1\n");
    let mut perms = std::fs::metadata(p.join(".env")).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(p.join(".env"), perms).unwrap();
    evnx(&p, &["remove", "DROP", "--check"]).assert_code(1);
}

#[test]
fn a_file_it_cannot_read_whole_is_no_verdict_not_a_pass() {
    let p = Project::new()
        .env("A=1\n")
        .file(".env.broken", "not an assignment\n");
    evnx(&p, &["remove", "GONE", "--check"])
        .assert_code(2)
        .assert_contains(".env.broken")
        .assert_contains("No verdict");
    let p = Project::new().env("A=1\n").toml("[vars.A\n");
    evnx(&p, &["remove", "GONE", "--check"]).assert_code(2);
}

#[test]
fn it_cannot_be_combined_with_what_it_never_does() {
    let p = project();
    for extra in [
        "--all",
        "--template",
        "--contract",
        "--dry-run",
        "--yes",
        "--ignore-missing",
        "--no-backup",
    ] {
        evnx(&p, &["remove", "DROP", "--check", extra])
            .assert_code(2)
            .assert_contains("cannot be used with");
    }
    evnx(&p, &["remove", "DROP", "--check", "--env-name", "staging"]).assert_code(2);
}

#[test]
fn json_names_places_and_never_a_value() {
    let p = project();
    let out = evnx(&p, &["remove", "DROP", "--check", "--json"]);
    out.assert_code(1).assert_not_contains(&sentinel());
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["check"], true);
    assert_eq!(doc["clean"], false);
    let present = doc["present"].as_array().unwrap();
    assert_eq!(present.len(), 4, "{doc:#}");
    let staging = present
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with(".env.staging"))
        .unwrap();
    assert_eq!(staging["kind"], "env");
    assert_eq!(staging["lines"], serde_json::json!([2, 4]));
    let contract = present.iter().find(|f| f["kind"] == "contract").unwrap();
    assert_eq!(contract["table"], "vars.DROP");

    evnx(&p, &["remove", "DROP", "--all", "--yes"]).assert_code(0);
    let out = evnx(&p, &["remove", "DROP", "--check", "--json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(doc["clean"], true);
    assert_eq!(doc["present"], serde_json::json!([]));
}

#[test]
fn help_documents_check() {
    let p = Project::new();
    evnx(&p, &["remove", "--help"])
        .assert_code(0)
        .assert_contains("--check")
        .assert_contains("1 only with --check");
}
