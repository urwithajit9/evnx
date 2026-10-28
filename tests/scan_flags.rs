//! `evnx scan`'s walk and flags (R-scan, 0.10.14).
//!
//! * I-053 — `--exclude` is a glob, or a plain name matching a whole path
//!   component; it has help text.
//! * I-054 — a path given twice, or a file inside a folder also given, is
//!   scanned once.
//! * I-062 — files and findings come out sorted: path, line, rule.
//! * I-057 — `--no-ignore-placeholders` overrides `[scan] ignore_placeholders`.
//! * I-061 — `--max-file-size` and `[scan] max_file_size` (DEC-SC4).
//! * I-048, I-289 — the "nothing was scanned" error says what was skipped and
//!   why, and no longer advises a flag that does not exist.
//! * I-254 — `[vars]` applies to a file named `app.env`.
//! * DEC-SF12 — `path` and `exclude` have help text.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

const AWS: &str = "AKIA4OZRMFJ3VREALKEY";

fn project(files: &[(&str, &str)]) -> TempDir {
    let d = TempDir::new().unwrap();
    for (name, body) in files {
        let p = d.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }
    d
}

/// (exit code, stdout, stderr)
fn scan(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .arg("scan")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn locations(dir: &TempDir, args: &[&str]) -> Vec<String> {
    let mut all = vec!["--format", "json"];
    all.extend_from_slice(args);
    let (_, stdout, _) = scan(dir, &all);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}"));
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["location"].as_str().unwrap().to_string())
        .collect()
}

fn help() -> String {
    let out = cargo_bin_cmd!("evnx")
        .args(["scan", "--help"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ── I-053 ───────────────────────────────────────────────────────────────────

/// `--exclude env` dropped `.env`; `--exclude test` dropped `latest/` and
/// `contest.py`. A plain name now matches a whole path component.
#[test]
fn t_i053_a_plain_name_matches_a_whole_component_not_a_substring() {
    let key = format!("k = \"{AWS}\"\n");
    let d = project(&[
        (".env", &format!("AWS_ACCESS_KEY_ID={AWS}\n")),
        ("latest/notes.txt", &key),
        ("contest.py", &key),
        ("test/fixture.py", &key),
    ]);
    let found = locations(&d, &["--exclude", "env", "--exclude", "test"]);
    assert!(found.iter().any(|l| l.starts_with("./.env:")), "{found:?}");
    assert!(
        found.iter().any(|l| l.starts_with("./latest/notes.txt:")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|l| l.starts_with("./contest.py:")),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|l| l.contains("test/fixture.py")),
        "{found:?}"
    );
}

/// A plain value with a slash names consecutive components.
#[test]
fn t_i053_a_plain_path_names_consecutive_components() {
    let key = format!("k = \"{AWS}\"\n");
    let d = project(&[("vendor/cache/a.py", &key), ("vendor/src/b.py", &key)]);
    let found = locations(&d, &["--exclude", "vendor/cache"]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("vendor/src/b.py"), "{found:?}");
}

#[test]
fn t_i053_exclude_and_path_have_help_text() {
    let help = help();
    let exclude = help
        .split("--exclude <EXCLUDE>")
        .nth(1)
        .expect("--exclude is listed");
    let first = exclude.lines().nth(1).unwrap_or("").trim();
    assert!(!first.starts_with("[default"), "no help text:\n{help}");
    assert!(exclude.contains("glob"), "{help}");
    assert!(exclude.contains("whole path component"), "{help}");
    let path = help
        .split("[PATH]...")
        .nth(2)
        .expect("[PATH]... argument listed");
    let first = path.lines().nth(1).unwrap_or("").trim();
    assert!(
        !first.starts_with("[default"),
        "no help text for PATH:\n{help}"
    );
}

// ── I-054 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i054_the_same_path_twice_is_scanned_once() {
    let d = project(&[(".env", &format!("AWS_ACCESS_KEY_ID={AWS}\n"))]);
    for args in [
        &[".", "."][..],
        &[".", "./.env"],
        &[".env", ".env"],
        &[".", ".env"],
    ] {
        let found = locations(&d, args);
        assert_eq!(found.len(), 1, "{args:?}: {found:?}");
    }
    // Counted once in the summary too.
    let (_, stdout, _) = scan(&d, &[".", ".", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["summary"]["files_scanned"], 1, "{v:#}");
}

/// A folder and a folder inside it: each file once.
#[test]
fn t_i054_a_nested_folder_adds_nothing() {
    let d = project(&[("a/b/.env", &format!("AWS_ACCESS_KEY_ID={AWS}\n"))]);
    assert_eq!(locations(&d, &["a", "a/b"]).len(), 1);
}

// ── I-062 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i062_named_files_come_out_in_path_order() {
    let body = format!("AWS_ACCESS_KEY_ID={AWS}\n");
    let d = project(&[("b.env", &body), ("a.env", &body)]);
    let found = locations(&d, &["b.env", "a.env"]);
    assert_eq!(
        found,
        vec!["a.env:1 (AWS_ACCESS_KEY_ID)", "b.env:1 (AWS_ACCESS_KEY_ID)"]
    );
    let (_, stdout, _) = scan(&d, &["b.env", "a.env"]);
    assert!(stdout.contains("scanning a.env, b.env"), "{stdout}");
}

/// A commented-out key is found by a second pass over the file; it is still
/// reported in line order.
#[test]
fn t_i062_findings_in_a_file_come_out_in_line_order() {
    let d = project(&[(
        ".env",
        &format!("# OLD_KEY={AWS}\nGITHUB_TOKEN=ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8\n"),
    )]);
    let found = locations(&d, &[".env"]);
    assert_eq!(found, vec![".env:1 (OLD_KEY)", ".env:2 (GITHUB_TOKEN)"]);
}

// ── I-057 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i057_no_ignore_placeholders_overrides_the_config() {
    let d = project(&[
        // The Slack rule matches; `fake` makes it look like filler.
        (".env", "SLACK_BOT=xoxb-123456789012-fake-token-value\n"),
        (".evnx.toml", "[scan]\nignore_placeholders = true\n"),
    ]);
    assert!(locations(&d, &[".env"]).is_empty());
    assert_eq!(
        locations(&d, &[".env", "--no-ignore-placeholders"]).len(),
        1
    );
    // The last of the pair wins.
    assert!(locations(
        &d,
        &[".env", "--no-ignore-placeholders", "--ignore-placeholders"]
    )
    .is_empty());
}

// ── I-061 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i061_max_file_size_lowers_and_raises_the_walk_limit() {
    let big = format!("AWS_ACCESS_KEY_ID={AWS}\n{}", "A=1\n".repeat(300_000)); // ~1.2 MB
    let d = project(&[
        ("big.env", &big),
        ("small.env", &format!("AWS_ACCESS_KEY_ID={AWS}\n")),
    ]);

    // Default 1 MiB: big.env is skipped and said.
    let (_, _, stderr) = scan(&d, &["."]);
    assert!(stderr.contains("skipped 1 file"), "{stderr}");
    assert_eq!(locations(&d, &["."]).len(), 1);

    // Raised: both read.
    assert_eq!(locations(&d, &[".", "--max-file-size", "2MiB"]).len(), 2);
    // Lowered: small.env (40 bytes) still read, nothing else.
    assert_eq!(locations(&d, &[".", "--max-file-size", "100"]).len(), 1);
    // From config.
    fs::write(
        d.path().join(".evnx.toml"),
        "[scan]\nmax_file_size = \"2MB\"\n",
    )
    .unwrap();
    assert_eq!(locations(&d, &["."]).len(), 2);
}

#[test]
fn t_i061_a_bad_size_is_an_error_not_a_default() {
    let d = project(&[(".env", "A=1\n")]);
    for bad in ["huge", "0", "-5", "5XB"] {
        let (code, _, stderr) = scan(&d, &[".", "--max-file-size", bad]);
        assert_eq!(code, 2, "{bad}: {stderr}");
        assert!(stderr.contains("max-file-size"), "{bad}: {stderr}");
    }
}

/// A named file is read whatever the limit says.
#[test]
fn t_i061_a_named_file_ignores_the_limit() {
    let big = format!("AWS_ACCESS_KEY_ID={AWS}\n{}", "A=1\n".repeat(300_000));
    let d = project(&[("big.env", &big)]);
    assert_eq!(locations(&d, &["big.env"]).len(), 1);
}

// ── I-048, I-289 ────────────────────────────────────────────────────────────

#[test]
fn t_i048_nothing_scanned_names_what_was_skipped_and_why() {
    let big = format!("AWS_ACCESS_KEY_ID={AWS}\n{}", "A=1\n".repeat(300_000));
    let d = project(&[("big.env", &big)]);
    let (code, _, stderr) = scan(&d, &["."]);
    assert_eq!(code, 2, "{stderr}");
    assert!(!stderr.contains("--include"), "{stderr}");
    assert!(stderr.contains("./big.env"), "{stderr}");
    assert!(stderr.contains("--max-file-size"), "{stderr}");
}

#[test]
fn t_i048_nothing_scanned_does_not_advise_include() {
    let d = project(&[("node_modules/x.env", "A=1\n")]);
    let (code, _, stderr) = scan(&d, &["."]);
    assert_eq!(code, 2, "{stderr}");
    assert!(!stderr.contains("--include"), "{stderr}");
    assert!(stderr.contains("Name a file"), "{stderr}");
}

// ── I-254 ───────────────────────────────────────────────────────────────────

#[test]
fn t_i254_vars_apply_to_a_file_named_app_env() {
    let d = project(&[
        ("app.env", "TENANT_REF=9f3a7c21b85e4d0fa62c\n"),
        (".evnx.toml", "[vars.TENANT_REF]\nsecret = true\n"),
    ]);
    let found = locations(&d, &["app.env"]);
    assert_eq!(found, vec!["app.env:1 (TENANT_REF)"], "{found:?}");
}

/// `*.env` reads as a `.env`; `*.env.yaml` is still YAML.
#[test]
fn t_i254_production_env_is_parsed_and_yaml_is_not() {
    let d = project(&[
        (
            "production.env",
            "# OLD={AWS}\n".replace("{AWS}", AWS).as_str(),
        ),
        ("config.env.yaml", &format!("key: {AWS}\n")),
    ]);
    let found = locations(&d, &["."]);
    assert!(
        found.contains(&"./production.env:1 (OLD)".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(&"./config.env.yaml:1".to_string()),
        "{found:?}"
    );
}

// ── RM-48: `evnx scan -` reads stdin ─────────────────────────────────────────

fn scan_stdin(input: &str, args: &[&str]) -> std::process::Output {
    let d = TempDir::new().unwrap();
    cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .arg("scan")
        .arg("-")
        .args(args)
        .write_stdin(input)
        .output()
        .unwrap()
}

#[test]
fn rm48_stdin_is_read_as_dotenv_when_it_parses_as_one() {
    let out = scan_stdin(
        "DB_PASSWORD=correct-horse-battery-staple\n",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["files"][0]["path"], "<stdin>");
    assert_eq!(json["files"][0]["read_as"], "dotenv");
    assert_eq!(json["findings"][0]["variable"], "DB_PASSWORD");
}

#[test]
fn rm48_a_diff_is_read_as_text_and_gets_no_history_advice() {
    let out = scan_stdin("+let k = \"sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9\";\n", &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<stdin>:1"), "{stdout}");
    assert!(!stdout.contains("filter-repo"), "{stdout}");
}

#[test]
fn rm48_empty_stdin_is_not_a_clean_scan() {
    let out = scan_stdin("", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing was piped"));
}

#[test]
fn rm48_stdin_beside_a_path_scans_both() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".env"), "SAFE=1\n").unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .args(["scan", ".", "-", "--format", "json"])
        .write_stdin("STRIPE_SECRET_KEY=sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9\n")
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["files_scanned"], 2, "{json}");
    assert_eq!(json["secrets_found"], 1, "{json}");
}
