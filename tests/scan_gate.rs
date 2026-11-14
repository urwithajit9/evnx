//! A scan result can be trusted: what the configuration left out is said, a
//! symlink is not followed out of the tree, an unreadable folder is not a
//! clean one, and a bad exclusion stops the run.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// An AWS-format key, built at run time.
fn aws_key() -> String {
    format!("{}{}", "AKIA", "4OZRMFJ3VREALKEY")
}

fn project(files: &[(&str, String)]) -> TempDir {
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
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn json(dir: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    let mut a = vec!["scan", ".", "--format", "json"];
    a.extend_from_slice(args);
    let (code, stdout, stderr) = run(dir, &a);
    let v = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}\n{stdout}\n{stderr}"));
    (code, v)
}

fn key_project_with_disable() -> TempDir {
    project(&[
        (".env", format!("AWS_ACCESS_KEY_ID={}\n", aws_key())),
        (
            ".evnx.toml",
            "[scan]\ndisable = [\"aws-access-key\"]\n".into(),
        ),
    ])
}

// ── effective configuration ────────────────────────────────────────────────

#[test]
fn a_disabled_rule_is_said_in_pretty_output() {
    let d = key_project_with_disable();
    let (code, stdout, _) = run(d.path(), &["scan", "."]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("1 rule disabled by .evnx.toml: aws-access-key"),
        "{stdout}"
    );
}

#[test]
fn a_disabled_rule_is_recorded_in_json() {
    let d = key_project_with_disable();
    let (code, v) = json(d.path(), &[]);
    assert_eq!(code, 0);
    let cfg = &v["effective_config"];
    assert_eq!(cfg["disabled_rules"], serde_json::json!(["aws-access-key"]));
    assert_eq!(cfg["severity_floor"], "low");
    assert!(cfg["baseline"].is_null());
    assert_eq!(cfg["config_files"], serde_json::json!([".evnx.toml"]));
}

#[test]
fn a_disabled_rule_is_recorded_in_valid_sarif() {
    let d = key_project_with_disable();
    let (_, stdout, _) = run(d.path(), &["scan", ".", "--format", "sarif"]);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        doc["runs"][0]["properties"]["effective_config"]["disabled_rules"],
        serde_json::json!(["aws-access-key"]),
        "{stdout}"
    );
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("schemas/sarif-schema-2.1.0.json")).unwrap();
    let validator = jsonschema::draft4::new(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&doc)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn a_raised_severity_floor_is_said() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let (code, stdout, _) = run(d.path(), &["scan", ".", "--severity", "high"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("severity floor high"), "{stdout}");
    let (_, v) = json(d.path(), &["--severity", "high"]);
    assert_eq!(v["effective_config"]["severity_floor"], "high");
}

#[test]
fn a_default_scan_says_nothing_about_configuration() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let (code, stdout, _) = run(d.path(), &["scan", "."]);
    assert_eq!(code, 0);
    assert!(
        !stdout.contains("disabled") && !stdout.contains("floor") && !stdout.contains("baseline"),
        "{stdout}"
    );
}

#[test]
fn a_baseline_in_force_is_said() {
    let d = project(&[(".env", format!("AWS_ACCESS_KEY_ID={}\n", aws_key()))]);
    let (code, _, _) = run(d.path(), &["scan", ".", "--update-baseline"]);
    assert_eq!(code, 0);
    let (code, stdout, _) = run(d.path(), &["scan", "."]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("baseline") && stdout.contains("accepts 1 finding"),
        "{stdout}"
    );
    let (_, v) = json(d.path(), &[]);
    assert_eq!(v["effective_config"]["baseline"]["entries"], 1);
}

// ── --no-config ────────────────────────────────────────────────────────────

#[test]
fn no_config_ignores_the_repository_config() {
    let d = key_project_with_disable();
    let (code, _, stderr) = run(d.path(), &["scan", "."]);
    assert_eq!(code, 0, "the config silences the rule: {stderr}");

    let (code, v) = json(d.path(), &["--no-config"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["summary"]["total"], 1);
    let cfg = &v["effective_config"];
    assert_eq!(cfg["disabled_rules"], serde_json::json!([]));
    assert_eq!(cfg["config_files"], serde_json::json!([]));
}

#[test]
fn no_config_leaves_out_the_default_baseline() {
    let d = project(&[(".env", format!("AWS_ACCESS_KEY_ID={}\n", aws_key()))]);
    run(d.path(), &["scan", ".", "--update-baseline"]);
    assert_eq!(run(d.path(), &["scan", "."]).0, 0);
    assert_eq!(run(d.path(), &["scan", ".", "--no-config"]).0, 1);
}

// ── symlinks ───────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_outside_the_tree_is_not_read() {
    let outside = project(&[("creds", format!("AWS_ACCESS_KEY_ID={}\n", aws_key()))]);
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    std::os::unix::fs::symlink(outside.path().join("creds"), d.path().join("x.env")).unwrap();

    for extra in [&[][..], &["--follow-symlinks"][..]] {
        let (code, v) = json(d.path(), extra);
        assert_eq!(code, 0, "{extra:?}: {v}");
        assert_eq!(v["summary"]["total"], 0);
        assert_eq!(v["summary"]["files_skipped"], 1, "{extra:?}: {v}");
        assert!(
            !v.to_string().contains(&aws_key()[4..]),
            "a value from outside the tree reached the output"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_inside_the_tree_is_read_only_when_asked() {
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        (
            "real/data.txt",
            format!("AWS_ACCESS_KEY_ID={}\n", aws_key()),
        ),
    ]);
    // The real file is excluded, so the link is the only way to reach it.
    std::os::unix::fs::symlink(d.path().join("real/data.txt"), d.path().join("link.txt")).unwrap();

    let (code, v) = json(d.path(), &["--exclude", "real"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"]["files_skipped"], 1);

    let (code, v) = json(d.path(), &["--exclude", "real", "--follow-symlinks"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["summary"]["total"], 1);
}

#[cfg(unix)]
#[test]
fn a_broken_symlink_followed_is_unreadable_not_clean() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    std::os::unix::fs::symlink(d.path().join("missing"), d.path().join("dangling.txt")).unwrap();
    let (code, v) = json(d.path(), &["--follow-symlinks"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["summary"]["files_unreadable"], 1);
}

// ── walk errors ────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_read_is_counted_and_blocks_the_all_clear() {
    use std::os::unix::fs::PermissionsExt;
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        ("locked/a.txt", "x\n".into()),
    ]);
    let locked = d.path().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let can_read = fs::read_dir(&locked).is_ok();
    let out = (!can_read).then(|| (run(d.path(), &["scan", "."]), json(d.path(), &[])));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let Some(((code, stdout, _), (_, v))) = out else {
        // Not a pass: root reads a mode-000 folder. The same counting path is
        // covered without permissions by
        // `a_broken_symlink_followed_is_unreadable_not_clean`.
        eprintln!("skipped: running as root, permissions are not enforced");
        return;
    };
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains("No secrets detected in the files that could be read; 1 could not be read"),
        "{stdout}"
    );
    assert!(!stdout.contains("✓  No secrets detected"), "{stdout}");
    assert_eq!(v["summary"]["files_unreadable"], 1);
    assert_eq!(v["unreadable"][0]["path"], "./locked");
}

// ── exclude patterns ───────────────────────────────────────────────────────

#[test]
fn an_invalid_exclude_glob_is_exit_2_naming_the_pattern() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let (code, _, stderr) = run(d.path(), &["scan", ".", "--exclude", "[abc"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("[abc"), "{stderr}");
    assert!(
        stderr.to_lowercase().contains("invalid exclude pattern"),
        "{stderr}"
    );
    assert!(stderr.contains("Pattern syntax error"), "{stderr}");
}

#[test]
fn an_invalid_exclude_glob_in_config_is_exit_2() {
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        (".evnx.toml", "[scan]\nexclude = [\"a[\"]\n".into()),
    ]);
    let (code, _, stderr) = run(d.path(), &["scan", "."]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("a["), "{stderr}");
}

// ── default excludes ───────────────────────────────────────────────────────

fn key_line() -> String {
    format!("AWS_ACCESS_KEY_ID={}\n", aws_key())
}

#[test]
fn plain_default_folders_are_not_walked() {
    let mut files = vec![(".env", "APP_NAME=demo\n".to_string())];
    let dirs = [".next", ".nuxt", ".turbo", "__pycache__", ".terraform"];
    let names: Vec<String> = dirs.iter().map(|d| format!("{d}/k.txt")).collect();
    for n in &names {
        files.push((n.as_str(), key_line()));
    }
    let d = project(&files);
    let (code, v) = json(d.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"]["total"], 0);
    // A file named on the command line is still read.
    let (code, _, _) = run(d.path(), &["scan", ".next/k.txt"]);
    assert_eq!(code, 1);
}

#[test]
fn a_vendor_folder_beside_a_manifest_is_skipped_and_said() {
    for manifest in ["go.mod", "composer.json", "Gemfile"] {
        let d = project(&[
            (".env", "APP_NAME=demo\n".into()),
            (manifest, "x\n".into()),
            ("vendor/k.txt", key_line()),
        ]);
        let (code, v) = json(d.path(), &[]);
        assert_eq!(code, 0, "{manifest}: {v}");
        assert_eq!(
            v["skipped_dirs"],
            serde_json::json!([format!("vendor ({manifest})")])
        );
        let (_, _, stderr) = run(d.path(), &["scan", "."]);
        assert!(
            stderr.contains(&format!("skipped dependency folders: vendor ({manifest})")),
            "{stderr}"
        );
        let (_, _, quiet) = run(d.path(), &["-q", "scan", "."]);
        assert!(!quiet.contains("skipped dependency folders"), "{quiet}");
    }
}

#[test]
fn a_vendor_folder_with_modules_txt_is_skipped() {
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        ("vendor/modules.txt", "# pkg\n".into()),
        ("vendor/k.txt", key_line()),
    ]);
    let (code, v) = json(d.path(), &[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_vendor_folder_without_a_manifest_is_walked() {
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        ("vendor/k.txt", key_line()),
    ]);
    let (code, v) = json(d.path(), &[]);
    assert_eq!(code, 1, "{v}");
    assert!(v.get("skipped_dirs").is_none(), "{v}");
}

#[test]
fn a_virtualenv_is_skipped_only_with_pyvenv_cfg() {
    for name in ["venv", ".venv"] {
        let with = project(&[
            (".env", "APP_NAME=demo\n".into()),
            (&format!("{name}/pyvenv.cfg"), "home = /usr\n".into()),
            (&format!("{name}/k.txt"), key_line()),
        ]);
        let (code, v) = json(with.path(), &[]);
        assert_eq!(code, 0, "{name} with pyvenv.cfg: {v}");
        assert_eq!(v["skipped_dirs"], serde_json::json!([name]));

        let without = project(&[
            (".env", "APP_NAME=demo\n".into()),
            (&format!("{name}/k.txt"), key_line()),
        ]);
        let (code, v) = json(without.path(), &[]);
        assert_eq!(code, 1, "{name} without pyvenv.cfg: {v}");
    }
}

#[test]
fn include_dir_walks_a_skipped_folder() {
    let d = project(&[
        (".env", "APP_NAME=demo\n".into()),
        ("go.mod", "module x\n".into()),
        ("vendor/k.txt", key_line()),
        (".next/k.txt", key_line()),
    ]);
    assert_eq!(json(d.path(), &[]).0, 0);
    let (code, v) = json(d.path(), &["--include-dir", "vendor"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["summary"]["total"], 1);
    let (_, v) = json(
        d.path(),
        &["--include-dir", "vendor", "--include-dir", ".next"],
    );
    assert_eq!(v["summary"]["total"], 2);
}

#[test]
fn scanning_a_default_folder_directly_reads_it() {
    let d = project(&[
        ("go.mod", "module x\n".into()),
        ("vendor/k.txt", key_line()),
        ("build/k.txt", key_line()),
        ("venv/pyvenv.cfg", "x\n".into()),
        ("venv/k.txt", key_line()),
    ]);
    for target in ["vendor/", "./vendor", "build", "venv"] {
        let (code, stdout, stderr) = run(d.path(), &["scan", target, "--format", "json"]);
        assert_eq!(code, 1, "{target}: {stdout}{stderr}");
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(v["summary"]["total"], 1, "{target}");
    }
}

#[test]
fn a_root_below_a_default_folder_name_is_scanned() {
    let d = project(&[("vendor/build/app/k.txt", key_line())]);
    let root = d.path().join("vendor/build/app");
    let (code, v) = json(&root, &[]);
    assert_eq!(code, 1, "{v}");
    // The same folder reached from outside by path.
    let (code, stdout, _) = run(d.path(), &["scan", "vendor/build/app", "--format", "json"]);
    assert_eq!(code, 1, "{stdout}");
}

// ── --exit-zero ────────────────────────────────────────────────────────────

#[test]
fn exit_zero_does_not_hide_a_run_that_could_not_start() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let (code, _, stderr) = run(d.path(), &["scan", ".", "--exclude", "[abc", "--exit-zero"]);
    assert_eq!(code, 2, "{stderr}");
}

#[cfg(unix)]
#[test]
fn exit_zero_lets_unreadable_files_pass_but_still_counts_them() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    std::os::unix::fs::symlink(d.path().join("missing"), d.path().join("dangling.txt")).unwrap();
    let (code, v) = json(d.path(), &["--follow-symlinks", "--exit-zero"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"]["files_unreadable"], 1);
}

// ── --no-config ────────────────────────────────────────────────────────────

#[test]
fn no_config_keeps_inline_allow_markers() {
    let d = project(&[(".env", key_line())]);
    let (_, v) = json(d.path(), &["--no-config"]);
    let rule = v["findings"][0]["rule_id"].as_str().unwrap().to_string();
    fs::write(
        d.path().join(".env"),
        format!(
            "AWS_ACCESS_KEY_ID={} # evnx:allow {rule} - test fixture\n",
            aws_key()
        ),
    )
    .unwrap();
    let (code, v) = json(d.path(), &["--no-config"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"]["total"], 0);
    assert_eq!(v["summary"]["allowed"], 1);
}

#[test]
fn a_rule_file_cannot_disable_a_builtin_rule() {
    let d = project(&[(".env", key_line())]);
    let (_, v) = json(d.path(), &[]);
    let rule = v["findings"][0]["rule_id"].as_str().unwrap().to_string();
    // Same id as evnx's own rule: refused, not an override.
    fs::create_dir_all(d.path().join(".evnx/rules")).unwrap();
    fs::write(
        d.path().join(".evnx/rules/x.toml"),
        format!("version = 1\n\n[[rules]]\nid = \"{rule}\"\nname = \"x\"\nregex = 'zzzzzz'\nseverity = \"high\"\n"),
    )
    .unwrap();
    let (code, _, stderr) = run(d.path(), &["scan", ".", "--no-config"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("cannot replace"), "{stderr}");
}

// ── outside a repository ───────────────────────────────────────────────────

#[test]
fn outside_a_repository_the_run_says_ignore_rules_do_not_apply() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let (_, _, stderr) = run(d.path(), &["scan", "."]);
    assert!(
        stderr.contains("not a git repository: git-ignore rules do not apply"),
        "{stderr}"
    );
}

#[test]
fn inside_a_repository_the_line_is_absent() {
    let d = project(&[(".env", "APP_NAME=demo\n".into())]);
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(d.path())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return;
    }
    let (_, _, stderr) = run(d.path(), &["scan", "."]);
    assert!(!stderr.contains("not a git repository"), "{stderr}");
}
