//! WP-15 VALIDATE-AND-REST: the cheap `validate` checks on by default,
//! `doctor --all`, `template INPUT OUTPUT`, `parse` facts and the UTF-16 hint
//! (validate and parse side; `backup`, `restore` and `update` are in the
//! feature-gated modules below).
//!
//! The assertions on secrets are the point of several tests: a value never
//! reaches an output that was not asked for it.

mod support;

use serde_json::Value;
use support::{run, run_with, Project};

fn json(out: &support::Output) -> Value {
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", out.stdout))
}

fn types(v: &Value) -> Vec<String> {
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["type"].as_str().unwrap().to_string())
        .collect()
}

const BAD: &str = "API_URL=nope\nPORT=abc\nADMIN_EMAIL=x\nNAME=ok\n";
const KEYS: &str = "API_URL=\nPORT=\nADMIN_EMAIL=\nNAME=\n";

fn bad_project() -> Project {
    Project::new().env(BAD).example(KEYS).private(".env")
}

// ── validate: VAL-3, GEN-22 ──────────────────────────────────────────────────

#[test]
fn wp15_default_run_warns_on_bad_url_port_email() {
    let p = bad_project();
    let out = run(p.path(), &["validate", "--format", "json"]);
    out.assert_code(0);
    let v = json(&out);
    let t = types(&v);
    for want in ["invalid_url", "invalid_port", "invalid_email"] {
        assert!(t.iter().any(|x| x == want), "{want} missing: {t:?}");
    }
    assert!(v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["severity"] == "warning"));
}

#[test]
fn wp15_port_heuristic_is_warning_not_error() {
    let p = Project::new()
        .env("PORT=99999\n")
        .example("PORT=\n")
        .private(".env");
    let out = run(p.path(), &["validate", "--format", "json"]);
    out.assert_code(0);
    let v = json(&out);
    let port = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["type"] == "invalid_port")
        .expect("a port finding");
    assert_eq!(port["severity"], "warning");
    assert_eq!(v["summary"]["errors"], 0);
}

#[test]
fn wp15_declared_name_skips_heuristic() {
    // `API_URL` is declared (with no format): the declared entry is
    // authoritative, so the name heuristic does not second-guess it.
    let p = bad_project()
        .toml("[vars.API_URL]\nrequired = true\n[vars.PORT]\n[vars.ADMIN_EMAIL]\n[vars.NAME]\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    out.assert_code(0);
    assert!(types(&json(&out)).is_empty(), "{}", out.stdout);
}

#[test]
fn wp15_declared_format_checked_without_flag() {
    let p = Project::new()
        .env("HOMEPAGE=nope\n")
        .example("HOMEPAGE=\n")
        .toml("[vars.HOMEPAGE]\nformat = \"url\"\n")
        .private(".env");
    let out = run(p.path(), &["validate", "--format", "json", "--no-formats"]);
    out.assert_code(1);
    assert!(types(&json(&out)).contains(&"format_mismatch".to_string()));
}

#[test]
fn wp15_no_formats_turns_heuristics_off() {
    let p = bad_project();
    let out = run(p.path(), &["validate", "--format", "json", "--no-formats"]);
    out.assert_code(0);
    assert!(types(&json(&out)).is_empty(), "{}", out.stdout);
    // The old spelling is accepted and changes nothing (it is the default).
    let out = run(
        p.path(),
        &["validate", "--format", "json", "--validate-formats"],
    );
    out.assert_code(0);
    assert_eq!(types(&json(&out)).len(), 3);
}

#[test]
fn wp15_config_validate_formats_false_respected() {
    let p = bad_project().toml("[validate]\nvalidate_formats = false\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    out.assert_code(0);
    assert!(types(&json(&out)).is_empty(), "{}", out.stdout);
}

#[test]
fn wp15_fail_on_warning_exits_1_on_url_warning() {
    let p = bad_project();
    run(p.path(), &["validate", "--fail-on", "warning"]).assert_code(1);
    run(p.path(), &["validate"]).assert_code(0);
}

// ── validate and parse: RUST-10, PARSE-2 ────────────────────────────────────

fn utf16(text: &str) -> Vec<u8> {
    let mut b = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b
}

fn with_binary(p: Project, name: &str, bytes: &[u8]) -> Project {
    std::fs::write(p.join(name), bytes).unwrap();
    p
}

const UTF16_LINE: &str =
    "u16.env is UTF-16. Save it as UTF-8 (in PowerShell: `Set-Content -Encoding utf8`).";

#[test]
fn wp15_utf16_file_hint_exit_2() {
    let p = with_binary(
        Project::new().example("A=\n"),
        "u16.env",
        &utf16("A=topsecretvalue\n"),
    );
    for args in [
        &["parse", "--env", "u16.env"][..],
        &["validate", "--env", "u16.env"],
        &["validate", "--env", "u16.env", "--input-format", "json"],
    ] {
        let out = run(p.path(), args);
        out.assert_code(2).assert_contains(UTF16_LINE);
        out.assert_not_contains("topsecretvalue");
    }
    // Without a BOM: ASCII text with a NUL after every character.
    let mut raw = Vec::new();
    for b in "KEY=value\nOTHER=1\n".bytes() {
        raw.extend_from_slice(&[b, 0]);
    }
    let p = with_binary(Project::new(), "u16.env", &raw);
    run(p.path(), &["parse", "--env", "u16.env"])
        .assert_code(2)
        .assert_contains(UTF16_LINE);
}

#[test]
fn wp15_utf8_bom_still_parses() {
    let p = Project::new().file("b.env", "\u{feff}A=1\nB=2\n");
    let out = run(p.path(), &["parse", "--env", "b.env", "--format", "json"]);
    out.assert_code(0);
    let keys: Vec<_> = json(&out)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys, ["A", "B"]);
}

// ── parse facts: PARSE-1 ─────────────────────────────────────────────────────

const FACTS: &str = "A=1\nexport B=\"two words\"\nA=3\nC=${A}tail\nD='$A'\nE=\"l1\nl2\"\nF=`bt`\n";

#[test]
fn wp15_parse_facts_quoted_exported_duplicate_expands() {
    let p = Project::new().file("f.env", FACTS);
    let out = run(p.path(), &["parse", "--env", "f.env", "--format", "json"]);
    out.assert_code(0);
    let v = json(&out);
    let e = |key: &str, line: u64| {
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key && e["line"] == line)
            .unwrap_or_else(|| panic!("{key}@{line} in {v}"))
            .clone()
    };
    assert_eq!(e("A", 1)["duplicate"], true);
    assert_eq!(e("A", 3)["duplicate"], false);
    assert_eq!(e("B", 2)["exported"], true);
    assert_eq!(e("B", 2)["quoted"], "double");
    assert_eq!(e("B", 2)["length"], 9);
    assert_eq!(e("C", 4)["expands"], true);
    assert_eq!(e("D", 5)["quoted"], "single");
    assert_eq!(e("D", 5)["expands"], false, "single quotes do not expand");
    assert_eq!(e("E", 6)["multiline"], true);
    assert_eq!(e("F", 8)["quoted"], "backtick");
    assert_eq!(e("A", 1)["quoted"], "none");
    // The pretty form names only the flags that are true.
    let pretty = run(p.path(), &["parse", "--env", "f.env"]);
    pretty
        .assert_contains("duplicate")
        .assert_contains("exported");
    pretty
        .assert_contains("expands")
        .assert_contains("multiline");
}

#[test]
fn wp15_parse_json_schema_2_additive() {
    let p = Project::new().file("f.env", "A=1\n");
    let v = json(&run(
        p.path(),
        &["parse", "--env", "f.env", "--format", "json"],
    ));
    assert_eq!(v["run"]["schema_version"], 2);
    // Every key schema 1 had is still there.
    let entry = &v["entries"][0];
    for k in ["key", "line", "empty", "placeholder", "secret"] {
        assert!(entry.get(k).is_some(), "{k} missing: {entry}");
    }
    assert!(v.get("problems").is_some() && v.get("file").is_some());
}

#[test]
fn wp15_parse_facts_never_carry_value() {
    let p = Project::new().file(
        "f.env",
        "TOKEN=\"ghp_s3cr3tvalue\"\nexport PASS='hunter2hunter2'\nURL=${TOKEN}zz\n",
    );
    for args in [
        &["parse", "--env", "f.env", "--format", "json"][..],
        &["parse", "--env", "f.env"],
    ] {
        let out = run(p.path(), args);
        out.assert_not_contains("s3cr3t")
            .assert_not_contains("hunter2");
    }
    // Only `--values` asks for them.
    run(p.path(), &["parse", "--env", "f.env", "--values"]).assert_contains("hunter2hunter2");
}

// ── template: TPL-2/4/5 ──────────────────────────────────────────────────────

fn template_project() -> Project {
    Project::new()
        .env("NAME=world\n")
        .file("t.tpl", "hello {{NAME}}\n")
        .private(".env")
}

#[test]
fn wp15_template_positional_input_output() {
    let p = template_project();
    run(
        p.path(),
        &["template", "t.tpl", "out.txt", "--no-gitignore"],
    )
    .assert_code(0);
    assert_eq!(p.read("out.txt"), "hello world\n");
}

#[test]
fn wp15_template_flags_still_work() {
    let p = template_project();
    run(
        p.path(),
        &[
            "template",
            "--input",
            "t.tpl",
            "--output",
            "out.txt",
            "--no-gitignore",
        ],
    )
    .assert_code(0);
    assert_eq!(p.read("out.txt"), "hello world\n");
}

#[test]
fn wp15_template_both_forms_usage_2() {
    let p = template_project();
    for args in [
        &["template", "t.tpl", "out.txt", "--input", "t.tpl"][..],
        &["template", "t.tpl", "--output", "out.txt"],
        &["template", "--input", "t.tpl", "out.txt"],
    ] {
        run(p.path(), args).assert_code(2);
        assert!(!p.exists("out.txt"), "{args:?}");
    }
    // One of each form is missing a half.
    run(p.path(), &["template", "t.tpl"]).assert_code(2);
    run(p.path(), &["template", "--input", "t.tpl"]).assert_code(2);
}

#[test]
fn wp15_template_dash_output_stdout() {
    let p = template_project();
    let out = run(p.path(), &["template", "t.tpl", "-"]);
    out.assert_code(0);
    assert_eq!(out.stdout, "hello world\n");
}

// ── doctor --all: DOC-1, DOC-6 ───────────────────────────────────────────────

const SECRET: &str = "sk_live_wp15_topsecret_0123456789";

/// A project every part runs on: an env file, its template, a private mode and
/// a `.gitignore` that covers it.
fn healthy() -> Project {
    Project::new()
        .git()
        .env("NAME=ok\nAPI_KEY=live-key-value-77\n")
        .example("NAME=\nAPI_KEY=\n")
        .file(".gitignore", ".env*\n!.env.example\n")
        .private(".env")
}

fn doctor_all(p: &Project, extra: &[&str]) -> support::Output {
    let mut a = vec!["doctor", "--all"];
    a.extend_from_slice(extra);
    run(p.path(), &a)
}

#[test]
fn wp15_doctor_all_runs_four_sections() {
    let p = healthy();
    p.track(".env.example");
    let out = doctor_all(&p, &[]);
    out.assert_code(0)
        .assert_contains("evnx doctor")
        .assert_contains("evnx validate")
        .assert_contains("evnx sync")
        .assert_contains("evnx scan");
    out.assert_contains("doctor    clean");
    // The doctor part no longer says what it did not look at: the rest ran.
    out.assert_not_contains("Not examined here");
}

#[test]
fn wp15_doctor_all_worst_verdict_wins() {
    // A secret in a tracked file: scan finds it, the others are fine.
    let p = healthy().file(
        "deploy.sh",
        &format!("export AWS_KEY=AKIAIOSFODNN7EXAMPLE\nTOKEN={SECRET}\n"),
    );
    p.track(".env.example");
    let out = doctor_all(&p, &["--json"]);
    let v = json(&out);
    assert_eq!(v["sections"]["scan"]["status"], "findings", "{v}");
    assert_eq!(v["verdict"], "findings");
    out.assert_code(1);

    // Not-a-directory is no verdict, whatever the parts would say.
    run(p.path(), &["doctor", "--all", "no-such-dir"]).assert_code(2);
}

#[test]
fn wp15_doctor_all_no_template_skips_sync_with_note() {
    let p = Project::new()
        .git()
        .env("NAME=ok\n")
        .file(".gitignore", ".env*\n")
        .private(".env");
    let out = doctor_all(&p, &["--json"]);
    let v = json(&out);
    assert_eq!(v["sections"]["sync"]["status"], "skipped", "{v}");
    assert!(v["sections"]["sync"]["note"]
        .as_str()
        .unwrap()
        .contains("no template"));
    assert_eq!(v["sections"]["validate"]["status"], "skipped");
    // Skipped is not a failure.
    assert!(out.code != Some(2), "{}", out.stderr);
    let text = doctor_all(&p, &[]);
    text.assert_contains("skipped");
}

#[test]
fn wp15_doctor_all_json_sections() {
    let p = healthy();
    p.track(".env.example");
    let out = doctor_all(&p, &["--json"]);
    out.assert_code(0);
    let v = json(&out);
    assert_eq!(v["run"]["schema_version"], 2);
    assert_eq!(v["run"]["tool"], "evnx");
    for name in ["doctor", "validate", "sync", "scan"] {
        let s = &v["sections"][name];
        assert!(s["status"].is_string(), "{name}: {v}");
        // Each part's own document, without its own `run` header.
        assert!(s["report"].is_object(), "{name}: {v}");
        assert!(s["report"].get("run").is_none(), "{name}");
    }
    assert!(v["sections"]["doctor"]["report"]["checks"].is_array());
    assert!(v["sections"]["validate"]["report"]["issues"].is_array());
}

#[test]
fn wp15_doctor_without_all_output_unchanged() {
    let p = healthy();
    p.track(".env.example");
    let out = run(p.path(), &["doctor", "--format", "json"]);
    let v = json(&out);
    assert_eq!(v["run"]["schema_version"], 1);
    assert!(v["checks"].is_array());
    assert!(v.get("sections").is_none());
    run(p.path(), &["doctor"]).assert_contains("Not examined here");
}

#[cfg(unix)]
#[test]
fn wp15_all_fix_repairs_only_what_doctor_fix_does() {
    use std::os::unix::fs::PermissionsExt;
    // Loose mode and a placeholder value: `doctor --fix` fixes the mode;
    // `validate --fix` would also replace the placeholder, and must not run.
    let p = healthy().mode(".env", 0o644);
    p.track(".env.example");
    let before = p.read(".env");
    doctor_all(&p, &["--fix"]);
    let mode = std::fs::metadata(p.join(".env"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(p.read(".env"), before, "the file's contents were edited");
    assert!(!p.exists(".env.bak"));
}

#[test]
fn wp15_auto_fix_env_warns_and_does_not_repair() {
    let p = healthy().file(".gitignore", "").private(".env");
    p.track(".env.example");
    let out = run_with(p.path(), &["doctor"], &[("EVNX_AUTO_FIX", "1")], None);
    out.assert_contains("EVNX_AUTO_FIX is no longer read; pass --fix");
    assert_eq!(
        out.stderr.matches("EVNX_AUTO_FIX").count(),
        1,
        "said once: {}",
        out.stderr
    );
    // Nothing was repaired: `.gitignore` is as it was.
    assert_eq!(p.read(".gitignore"), "");
    out.assert_not_contains("auto-fix enabled");
    // `--fix` still repairs.
    run(p.path(), &["doctor", "--fix"]);
    assert!(p.read(".gitignore").contains(".env"));
    // JSON keeps stdout clean.
    let out = run_with(
        p.path(),
        &["doctor", "--format", "json"],
        &[("EVNX_AUTO_FIX", "1")],
        None,
    );
    json(&out);
}

#[test]
fn wp15_no_secret_value_in_all_output() {
    let p = Project::new()
        .git()
        .env(&format!("NAME=ok\nSECRET_KEY={SECRET}\nAPI_URL=nope\n"))
        .example("NAME=\nSECRET_KEY=\nAPI_URL=\n")
        .file(".gitignore", ".env*\n!.env.example\n")
        .private(".env");
    p.track(".env.example");
    for args in [&["doctor", "--all"][..], &["doctor", "--all", "--json"]] {
        let out = run(p.path(), args);
        out.assert_not_contains(SECRET);
    }
}

// ── backup / restore (feature backup) ────────────────────────────────────────

#[cfg(feature = "backup")]
mod backup_restore {
    use super::*;

    const ENV: &str = "DB_URL=postgres://user:pw@db.internal/app\nAPI_SECRET=sk_demo_000\n";
    const PW: &str = "demo-passphrase-correct-horse\n";

    fn backed_up() -> Project {
        let p = Project::new().env(ENV).file("pw", PW);
        run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
        std::fs::remove_file(p.join(".env")).unwrap();
        p
    }

    // ── RST-1, D5 ───────────────────────────────────────────────────────────

    #[test]
    fn wp15_restore_output_dash_writes_stdout_only() {
        let p = backed_up();
        let out = run(
            p.path(),
            &[
                "restore",
                ".env.backup",
                "--key-file",
                "pw",
                "--output",
                "-",
            ],
        );
        out.assert_code(0);
        assert_eq!(out.stdout, ENV, "stdout is the text and nothing else");
        assert!(!p.exists(".env"), "nothing was written");
        assert!(!p.exists("-"));
        // No `.bak`, no `.gitignore` question or edit.
        assert!(!p.exists(".env.bak"));
        assert!(!p.exists(".gitignore"));
    }

    #[test]
    fn wp15_restore_dash_status_on_stderr() {
        let p = backed_up();
        let out = run(
            p.path(),
            &[
                "restore",
                ".env.backup",
                "--key-file",
                "pw",
                "--output",
                "-",
            ],
        );
        out.assert_code(0);
        assert!(
            out.stderr.contains("Decryption successful"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("Restored 2 variable(s) to stdout"),
            "{}",
            out.stderr
        );
        assert!(!out.stdout.contains("Restored"), "{}", out.stdout);
        // The same flag by its hidden alias.
        let old = run(
            p.path(),
            &[
                "restore",
                ".env.backup",
                "--password-file",
                "pw",
                "--output",
                "-",
            ],
        );
        assert_eq!(old.stdout, ENV);
    }

    #[test]
    fn wp15_restore_dash_no_bak_no_gitignore() {
        let p = backed_up().env("OLD=1\n").git();
        let out = run(
            p.path(),
            &[
                "restore",
                ".env.backup",
                "--key-file",
                "pw",
                "--output",
                "-",
                "--yes",
            ],
        );
        out.assert_code(0);
        assert_eq!(out.stdout, ENV);
        assert_eq!(
            p.read(".env"),
            "OLD=1\n",
            "the existing file was left alone"
        );
        assert!(!p.exists(".env.bak"));
        assert!(!p.exists(".gitignore"));
    }

    #[cfg(unix)]
    #[test]
    fn wp15_restore_dash_terminal_refused_without_show_values() {
        use support::pty::Pty;
        let p = backed_up();
        let args = [
            "restore",
            ".env.backup",
            "--key-file",
            "pw",
            "--output",
            "-",
        ];
        let (text, code) = Pty::spawn(p.path(), &args).finish();
        assert_eq!(code, 2, "{text}");
        assert!(text.contains("--show-values"), "{text}");
        assert!(!text.contains("sk_demo_000"), "{text}");

        let mut shown = args.to_vec();
        shown.push("--show-values");
        let (text, code) = Pty::spawn(p.path(), &shown).finish();
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("sk_demo_000"), "{text}");
    }

    #[test]
    fn wp15_restore_inspect_and_dry_run_need_no_show_values() {
        let p = backed_up();
        for flag in ["--inspect", "--dry-run"] {
            let out = run(
                p.path(),
                &[
                    "restore",
                    ".env.backup",
                    "--key-file",
                    "pw",
                    "--output",
                    "-",
                    flag,
                ],
            );
            out.assert_code(0).assert_not_contains("sk_demo_000");
        }
    }

    // ── RST-3 ───────────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn wp15_restored_file_is_private_named_in_full_and_gitignore_checked() {
        use std::os::unix::fs::PermissionsExt;
        let p = Project::new()
            .git()
            .file("pw", PW)
            .file("note.txt", "plain words, no assignment\n");
        run(
            p.path(),
            &["backup", "note.txt", "--key-file", "pw", "--no-gitignore"],
        )
        .assert_code(0);
        // A leftover from an earlier run, readable by everyone.
        let stale = p.join("out.txt.restored");
        std::fs::write(&stale, "old").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();

        let out = run(
            p.path(),
            &[
                "restore",
                "note.txt.backup",
                "--key-file",
                "pw",
                "--output",
                "out.txt",
                "--gitignore",
                "--yes",
            ],
        );
        out.assert_code(5);
        let full = p.join("out.txt.restored");
        out.assert_contains(&format!("Full path: {}", full.display()));
        out.assert_contains("holds plaintext secrets");
        let mode = std::fs::metadata(&full).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(
            p.is_ignored("out.txt.restored"),
            "the same git-ignore check as the output"
        );
    }

    // ── BAK-3, BAK-5 ────────────────────────────────────────────────────────

    #[test]
    fn wp15_backup_says_encrypted_and_offers_gitignore() {
        let p = Project::new().git().env(ENV).file("pw", PW);
        let out = run(p.path(), &["backup", "--key-file", "pw"]);
        out.assert_code(0)
            .assert_contains("This file is encrypted. Committing it is as safe as its passphrase.");
        assert!(!p.exists(".gitignore"), "no flag: .gitignore is left alone");

        let out = run(p.path(), &["backup", "--key-file", "pw", "--gitignore"]);
        out.assert_code(0);
        assert!(p.is_ignored(".env.backup"));

        let p = Project::new().git().env(ENV).file("pw", PW);
        let out = run(p.path(), &["backup", "--key-file", "pw", "--no-gitignore"]);
        out.assert_code(0).assert_not_contains("Committing it");
        run(
            p.path(),
            &[
                "backup",
                "--key-file",
                "pw",
                "--gitignore",
                "--no-gitignore",
            ],
        )
        .assert_code(2);
    }

    #[test]
    fn wp15_key_file_is_the_shown_name_password_file_a_hidden_alias() {
        let p = Project::new();
        for cmd in ["backup", "restore"] {
            let help = run(p.path(), &[cmd, "--help"]).stdout;
            assert!(help.contains("--key-file"), "{cmd}: {help}");
            assert!(!help.contains("--password-file"), "{cmd}: {help}");
            assert!(!help.contains("password"), "{cmd} says password: {help}");
        }
        // Both spellings work, and neither is a removed spelling.
        let p = Project::new().env(ENV).file("pw", PW);
        run(p.path(), &["backup", "--password-file", "pw"]).assert_code(0);
        run(p.path(), &["backup", "--key-file", "pw"]).assert_code(0);
    }

    // ── BAK-2 ───────────────────────────────────────────────────────────────

    fn backup_with_env_passphrase(pw: &str) -> support::Output {
        let p = Project::new().env(ENV);
        run_with(p.path(), &["backup"], &[("EVNX_PASSWORD", pw)], None)
    }

    #[test]
    fn wp15_weak_passphrase_hint_names_reason_not_value() {
        for (pw, reason) in [
            ("12345678901234", "only digits"),
            ("zzzzzzzzzzzzzz", "one character repeated"),
            ("Password123", "common word"),
        ] {
            let out = backup_with_env_passphrase(pw);
            out.assert_code(0)
                .assert_contains("This passphrase is weak:")
                .assert_contains(reason)
                .assert_contains("4 random words is stronger");
            out.assert_not_contains(pw);
            assert_eq!(out.stderr.matches("This passphrase is weak").count(), 1);
        }
        let out = backup_with_env_passphrase("tr0ub4dor");
        out.assert_contains("under 12 characters");
    }

    #[test]
    fn wp15_strong_passphrase_silent() {
        let out = backup_with_env_passphrase("correct horse battery staple");
        out.assert_code(0).assert_not_contains("weak");
    }

    #[test]
    fn wp15_hint_never_changes_exit() {
        // Weak, and still a backup that restores.
        let p = Project::new().env(ENV);
        let out = run_with(
            p.path(),
            &["backup"],
            &[("EVNX_PASSWORD", "12345678")],
            None,
        );
        out.assert_code(0);
        assert!(p.exists(".env.backup"));
        // A key file is a script's input: no hint without a terminal.
        let p = Project::new().env(ENV).file("pw", "12345678\n");
        run(p.path(), &["backup", "--key-file", "pw"])
            .assert_code(0)
            .assert_not_contains("weak");
    }
}

// ── update (feature net, debug build) ────────────────────────────────────────

#[cfg(all(feature = "net", debug_assertions))]
mod update {
    use super::*;
    use mockito::Matcher;
    use support::install::Install;

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const NO_PROXY: (&str, &str) = ("NO_PROXY", "127.0.0.1,localhost");
    const TOKEN: &str = "ghp_wp15_not_for_output_0123456789";

    fn release(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/urwithajit9/evnx/releases/tag/{tag}"}}"#
        )
    }

    fn check(env: &[(&str, &str)], args: &[&str]) -> support::Output {
        let mut all = vec!["update", "--check"];
        all.extend_from_slice(args);
        let mut e = vec![NO_PROXY];
        e.extend_from_slice(env);
        Install::at(".cargo/bin").run_with(&all, &e)
    }

    #[test]
    fn wp15_check_sends_token_only_to_api_host() {
        let mut s = mockito::Server::new();
        let api = s
            .mock("GET", "/latest")
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_status(200)
            .with_body(release("v99.0.0"))
            .expect(1)
            .create();
        // The redirect fallback is another host in real life: it never gets
        // the token, and is not asked when the API answered.
        let redirect = s.mock("GET", "/redirect").expect(0).create();
        let url = format!("{}/latest", s.url());
        let redirect_url = format!("{}/redirect", s.url());
        let out = check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("EVNX_UPDATE_REDIRECT_URL", &redirect_url),
                ("GITHUB_TOKEN", TOKEN),
                ("GH_TOKEN", ""),
            ],
            &[],
        );
        out.assert_code(1);
        api.assert();
        redirect.assert();
        out.assert_not_contains(TOKEN);

        // No token: no Authorization header at all.
        let mut s = mockito::Server::new();
        let m = s
            .mock("GET", "/latest")
            .match_header("authorization", Matcher::Missing)
            .with_status(200)
            .with_body(release(&format!("v{VERSION}")))
            .expect(1)
            .create();
        let url = format!("{}/latest", s.url());
        check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("GITHUB_TOKEN", ""),
                ("GH_TOKEN", ""),
            ],
            &[],
        )
        .assert_code(0);
        m.assert();

        // GH_TOKEN is read when GITHUB_TOKEN is not set.
        let mut s = mockito::Server::new();
        let m = s
            .mock("GET", "/latest")
            .match_header("authorization", "Bearer from-gh-token")
            .with_status(200)
            .with_body(release(&format!("v{VERSION}")))
            .expect(1)
            .create();
        let url = format!("{}/latest", s.url());
        check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("GITHUB_TOKEN", ""),
                ("GH_TOKEN", "from-gh-token"),
            ],
            &[],
        )
        .assert_code(0);
        m.assert();
    }

    #[test]
    fn wp15_check_403_falls_back_to_redirect() {
        let mut s = mockito::Server::new();
        s.mock("GET", "/latest")
            .with_status(403)
            .with_body("{}")
            .create();
        let hop = s
            .mock("GET", "/redirect")
            // The fallback goes to github.com, not the API: no token.
            .match_header("authorization", Matcher::Missing)
            .with_status(302)
            .with_header(
                "location",
                "https://github.com/urwithajit9/evnx/releases/tag/v99.0.0",
            )
            .expect(1)
            .create();
        let url = format!("{}/latest", s.url());
        let redirect_url = format!("{}/redirect", s.url());
        let out = check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("EVNX_UPDATE_REDIRECT_URL", &redirect_url),
                ("GITHUB_TOKEN", TOKEN),
                ("GH_TOKEN", ""),
            ],
            &["--json", "-v"],
        );
        out.assert_code(1);
        hop.assert();
        let v = json(&out);
        assert_eq!(v["latest"], "99.0.0");
        assert_eq!(v["status"], "behind");
        // `-v` says which route answered; nothing prints the token.
        out.assert_contains("answered by the releases/latest redirect");
        out.assert_not_contains(TOKEN);

        // A redirect that names no tag is no answer: the API's error stands.
        let mut s = mockito::Server::new();
        s.mock("GET", "/latest")
            .with_status(429)
            .with_body("{}")
            .create();
        s.mock("GET", "/redirect")
            .with_status(302)
            .with_header("location", "https://github.com/urwithajit9/evnx/releases")
            .create();
        let url = format!("{}/latest", s.url());
        let redirect_url = format!("{}/redirect", s.url());
        check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("EVNX_UPDATE_REDIRECT_URL", &redirect_url),
                ("GITHUB_TOKEN", ""),
                ("GH_TOKEN", ""),
            ],
            &[],
        )
        .assert_code(2)
        .assert_contains("rate-limiting");
    }

    #[test]
    fn wp15_check_token_not_printed_in_verbose() {
        let mut s = mockito::Server::new();
        s.mock("GET", "/latest")
            .with_status(200)
            .with_body(release("v99.0.0"))
            .create();
        let url = format!("{}/latest", s.url());
        let out = check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("GITHUB_TOKEN", TOKEN),
                ("GH_TOKEN", ""),
            ],
            &["-v"],
        );
        out.assert_code(1).assert_not_contains(TOKEN);
        out.assert_contains("answered by the API");
        let out = check(
            &[
                ("EVNX_UPDATE_URL", &url),
                ("GITHUB_TOKEN", TOKEN),
                ("GH_TOKEN", ""),
            ],
            &["--json"],
        );
        out.assert_not_contains(TOKEN);
    }

    #[test]
    fn wp15_list_includes_running_copy_off_path() {
        // The binary is in a folder that is on no PATH and is no install folder.
        let install = Install::at("target/release");
        let out = install.run_with(
            &["update", "--list", "--json"],
            &[("PATH", "/usr/bin:/bin"), ("HOME", "/nonexistent-home")],
        );
        out.assert_code(0);
        let v = json(&out);
        let copies = v["copies"].as_array().unwrap();
        let this = copies
            .iter()
            .find(|c| c["this"] == true)
            .unwrap_or_else(|| panic!("the running copy is not listed: {v}"));
        assert_eq!(this["runs"], false);
        assert_eq!(this["version"], VERSION);
        assert!(this["path"]
            .as_str()
            .unwrap()
            .ends_with("target/release/evnx"));
        let pretty = install.run_with(
            &["update", "--list"],
            &[("PATH", "/usr/bin:/bin"), ("HOME", "/nonexistent-home")],
        );
        pretty.assert_contains("this one");
        pretty.assert_not_contains("No copy of evnx");
    }
}
