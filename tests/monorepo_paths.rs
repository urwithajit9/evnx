//! Monorepos (F14-10): paths in `.evnx.toml` are read from the folder that
//! holds it (I-333), and `doctor` notices env files in subfolders (I-331).
//!
//! ⚠️ Secret-shaped values are assembled at run time.

mod support;

use std::fs;
use support::{run, Project};

const IGNORE: &str = ".env*\n!.env.example\n";

/// A repository with a private root `.env`, so doctor's own findings are quiet.
fn repo() -> Project {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .example("A=\n")
        .file("package.json", "{}\n")
        .env("A=1\n");
    p.track(".env.example");
    #[cfg(unix)]
    let p = p.mode(".env", 0o600);
    p
}

fn nested(p: &Project, dirs: &[&str]) {
    for d in dirs {
        let dir = p.path().join(d);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".env"), "B=2\n").unwrap();
    }
}

// ── I-331 ────────────────────────────────────────────────────────────────────

#[test]
fn doctor_notes_nested_env_files_as_info_with_a_hint() {
    let p = repo();
    nested(
        &p,
        &[
            "apps/web",
            "packages/api",
            "node_modules/pkg",
            "target/x",
            ".git/x",
        ],
    );
    let out = run(p.path(), &["doctor"]);
    out.assert_code(0);
    out.assert_contains("apps/web/.env");
    out.assert_contains("packages/api/.env");
    out.assert_contains("evnx doctor apps/web");
    out.assert_not_contains("node_modules");
    out.assert_not_contains("target/x");
    // Not checked is not an error: the root is healthy and so is the exit.
    let json = run(p.path(), &["doctor", "--format", "json"]);
    json.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let env = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "env_file")
        .unwrap_or_else(|| panic!("{doc}"));
    assert_eq!(env["severity"], "info", "{env}");
}

#[test]
fn doctor_lists_at_most_five_and_counts_the_rest() {
    let p = repo();
    let dirs: Vec<String> = (1..=8).map(|i| format!("apps/app{i}")).collect();
    nested(&p, &dirs.iter().map(String::as_str).collect::<Vec<_>>());
    let out = run(p.path(), &["doctor"]);
    out.assert_contains("apps/app1/.env");
    out.assert_contains("apps/app5/.env");
    out.assert_not_contains("apps/app6/.env");
    out.assert_contains("and 3 more");
}

#[test]
fn doctor_says_nothing_extra_without_nested_files() {
    let p = repo();
    run(p.path(), &["doctor"]).assert_not_contains("subfolders");
}

#[test]
fn doctor_names_nested_files_when_the_root_has_no_env() {
    let p = Project::new()
        .git()
        .file(".gitignore", IGNORE)
        .example("A=\n")
        .file("package.json", "{}\n");
    nested(&p, &["apps/web"]);
    run(p.path(), &["doctor"]).assert_contains("apps/web/.env");
}

#[test]
fn init_mentions_nested_env_files_and_leaves_them_alone() {
    let p = Project::new().git();
    nested(&p, &["apps/web"]);
    fs::write(p.path().join("apps/web/.env.example"), "B=\n").unwrap();
    let out = run(p.path(), &["init", "--yes"]);
    out.assert_code(0);
    out.assert_contains("apps/web/.env also exists");
    assert_eq!(p.read("apps/web/.env"), "B=2\n");
    assert_eq!(p.read("apps/web/.env.example"), "B=\n");
}

// ── I-333 ────────────────────────────────────────────────────────────────────

fn stripe() -> String {
    format!("sk_live_{}", "51Habc1234567890ABCDEFGH")
}

/// A root `.evnx.toml` naming `apps/web/fixtures/**` excludes it from a scan
/// run inside `apps/web`, as from the root.
#[test]
fn scan_exclude_is_read_from_the_config_folder() {
    let p = Project::new().toml("[scan]\nexclude = [\"apps/web/fixtures/**\"]\n");
    let key = stripe();
    for f in ["apps/web/fixtures/a.txt", "apps/web/src/b.txt"] {
        let path = p.path().join(f);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("key = {key}\n")).unwrap();
    }
    for dir in [p.path().to_path_buf(), p.path().join("apps").join("web")] {
        let out = run(&dir, &["scan", "--format", "json"]);
        let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        let locs: Vec<String> = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["location"].as_str().unwrap().replace('\\', "/"))
            .collect();
        assert_eq!(locs.len(), 1, "from {}: {locs:?}", dir.display());
        assert!(locs[0].contains("src/b.txt"), "{locs:?}");
    }
}

/// `[defaults] example` from a subfolder (I-332, kept under test here with the
/// rest of the rule).
#[test]
fn defaults_example_is_read_from_the_config_folder_in_a_subfolder() {
    let p = Project::new().toml("[defaults]\nexample = \"tpl/app.env\"\n");
    fs::create_dir_all(p.path().join("tpl")).unwrap();
    fs::write(p.path().join("tpl/app.env"), "A=x\nFROM_TEMPLATE=x\n").unwrap();
    fs::create_dir_all(p.path().join("sub")).unwrap();
    fs::write(p.path().join("sub/.env"), "A=1\n").unwrap();
    let out = run(&p.path().join("sub"), &["validate", "--exit-zero"]);
    out.assert_contains("FROM_TEMPLATE");
}
