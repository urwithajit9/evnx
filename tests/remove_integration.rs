//! `evnx remove` (R1, v0.13.0). Test ids are the plan's:
//! `specs/evnx-remove-command-plan.md` §8, as cut down in §13.4.
//!
//! Byte-level editing (A-1 … A-16, F-1, F-2, F-4) is unit-tested in
//! `src/commands/remove/edit.rs`, and the contract editor in `contract.rs`;
//! the two-phase apply (E-3, E-5) in `src/commands/remove/mod.rs`. This file
//! runs the binary.
//!
//! ⚠️ Secret-shaped fixture values are assembled at run time, so push
//! protection never sees one, and each is a sentinel the leak tests grep for.

mod support;

use support::pty::{Pty, ENTER};
use support::{run, run_with, Project};

/// The value every leak test looks for. Built, not written out.
fn sentinel() -> String {
    format!("SENTINEL_{}{}", "9f3a", "_c0ffee")
}

fn evnx(p: &Project, args: &[&str]) -> support::run::Output {
    run(p.path(), args)
}

/// A project where every checker is clean: `.env`, `.env.staging`,
/// `.env.production`, the template and a contract, all with `KEEP` and `DROP`.
fn clean_project() -> Project {
    let s = sentinel();
    Project::new()
        .git()
        .file(".gitignore", ".env*\n!.env.example\n")
        .env(&format!("KEEP=1\nDROP={s}\n"))
        .file(".env.staging", &format!("KEEP=2\nDROP={s}\n"))
        .file(".env.production", &format!("KEEP=2\nDROP={s}\n"))
        .example("KEEP=\nDROP=\n")
        .toml("[vars.KEEP]\nrequired = true\n\n[vars.DROP]\nrequired = true\n")
}

// ── Group A, through the binary ─────────────────────────────────────────────

#[test]
fn a1_removes_one_line_and_keeps_a_backup() {
    let p = Project::new().env("A=1\nB=2\nC=3\n");
    evnx(&p, &["remove", "B", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "A=1\nC=3\n");
    assert_eq!(p.read(".env.bak"), "A=1\nB=2\nC=3\n");
}

#[test]
fn a10_a_case_only_miss_is_exit_2_and_names_the_key() {
    let p = Project::new().env("PORT=1\n");
    evnx(&p, &["remove", "port", "--yes"])
        .assert_code(2)
        .assert_contains("Did you mean PORT?");
    assert_eq!(p.read(".env"), "PORT=1\n");
}

#[test]
fn a11_a_name_inside_a_comment_or_value_is_not_an_entry() {
    let p = Project::new().env("# DROP was removed\nX=\"DROP=1\"\n");
    evnx(&p, &["remove", "DROP", "--yes"]).assert_code(2);
    assert_eq!(p.read(".env"), "# DROP was removed\nX=\"DROP=1\"\n");
}

#[test]
fn a12_s12_the_last_entry_leaves_an_empty_file() {
    let p = Project::new().env("ONLY=1\n");
    evnx(&p, &["remove", "ONLY", "--yes"]).assert_code(0);
    assert!(p.exists(".env"));
    assert_eq!(p.read(".env"), "");
}

#[cfg(unix)]
fn mode(p: &Project, name: &str) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p.join(name))
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
#[test]
fn a17_a18_modes_are_kept_and_a_loose_one_is_reported() {
    let p = Project::new().env("X=1\nY=2\n").mode(".env", 0o600);
    evnx(&p, &["remove", "X", "--yes"]).assert_code(0);
    assert_eq!((mode(&p, ".env"), mode(&p, ".env.bak")), (0o600, 0o600));

    let p = Project::new().env("X=1\nY=2\n").mode(".env", 0o644);
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains("chmod 600");
    assert_eq!((mode(&p, ".env"), mode(&p, ".env.bak")), (0o644, 0o644));
}

#[cfg(unix)]
#[test]
fn a19_a_symlink_is_written_through() {
    let p = Project::new()
        .file("real.env", "X=1\nY=2\n")
        .symlink("real.env", ".env");
    evnx(&p, &["remove", "X", "--yes"]).assert_code(0);
    assert!(std::fs::symlink_metadata(p.join(".env"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(p.read("real.env"), "Y=2\n");
}

#[cfg(unix)]
#[test]
fn a20_a_read_only_file_is_refused() {
    let p = Project::new().env("X=1\n").mode(".env", 0o444);
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(2)
        .assert_contains("read-only");
    assert_eq!(p.read(".env"), "X=1\n");
    assert!(!p.exists(".env.bak"));
}

#[test]
fn a21_r13_invalid_utf8_and_a_directory_are_refused() {
    let p = Project::new();
    std::fs::write(p.join(".env"), b"X=\xff\xfe\n").unwrap();
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(2)
        .assert_contains("not UTF-8");
    assert_eq!(std::fs::read(p.join(".env")).unwrap(), b"X=\xff\xfe\n");

    let p = Project::new().dir(".env");
    evnx(&p, &["remove", "X", "--yes"]).assert_code(2);
}

#[test]
fn a_line_the_parser_refuses_stops_everything() {
    let s = sentinel();
    let p = Project::new().env(&format!("X=1\n{s}\n"));
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(2)
        .assert_contains("Line 2")
        .assert_not_contains(&s);
    assert_eq!(p.read(".env"), format!("X=1\n{s}\n"));
}

// ── Group B: which file ─────────────────────────────────────────────────────

#[test]
fn b1_no_defaults_targets_dot_env_and_prints_it() {
    let p = Project::new().env("X=1\n");
    let abs = std::path::absolute(p.join(".env")).unwrap();
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains(&format!("{} (default)", abs.display()));
}

#[test]
fn b2_b3_defaults_env_name_is_followed_and_said_and_the_flag_wins() {
    let p = Project::new()
        .env("X=0\n")
        .file(".env.staging", "X=1\n")
        .file(".env.development", "X=2\n")
        .toml("[defaults]\nenv_name = \"staging\"\n");
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains("(from [defaults] env_name)");
    assert_eq!(p.read(".env.staging"), "");
    assert_eq!(p.read(".env"), "X=0\n");

    evnx(&p, &["remove", "X", "--env-name", "development", "--yes"])
        .assert_code(0)
        .assert_contains("(--env-name)");
    assert_eq!(p.read(".env.development"), "");
}

#[test]
fn b4_s17_a_default_that_resolves_to_production_still_asks() {
    let p = Project::new()
        .file(".env.production", "X=1\n")
        .toml("[defaults]\nenv_name = \"production\"\n");
    evnx(&p, &["remove", "X"])
        .assert_code(2)
        .assert_contains("--yes");
    assert_eq!(p.read(".env.production"), "X=1\n");

    let mut s = Pty::spawn(p.path(), &["remove", "X"]);
    s.wait_for(".env.production, from [defaults] env_name?");
    s.press(&["n", ENTER]);
    let (_, code) = s.finish();
    assert_eq!(code, 4);
}

#[test]
fn b6_b7_unknown_and_invalid_env_names() {
    let p = Project::new().env("X=1\n").file(".env.staging", "X=1\n");
    evnx(&p, &["remove", "X", "--env-name", "nope", "--yes"])
        .assert_code(2)
        .assert_contains("staging");
    evnx(&p, &["remove", "X", "--env-name", "../x", "--yes"])
        .assert_code(2)
        .assert_contains("invalid environment name");
}

#[test]
fn b8_b9_b10_the_template_is_the_projects_one() {
    let p = Project::new().env("X=1\n").file(".env.sample", "X=\nY=\n");
    evnx(&p, &["remove", "X", "--template", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env.sample"), "Y=\n");

    let p = Project::new()
        .env("X=1\n")
        .file("tpl.env", "X=\nY=\n")
        .toml("[defaults]\nexample = \"tpl.env\"\n");
    evnx(&p, &["remove", "X", "--template", "--yes"]).assert_code(0);
    assert_eq!(p.read("tpl.env"), "Y=\n");

    let p = Project::new().env("X=1\n");
    evnx(&p, &["remove", "X", "--template", "--yes"])
        .assert_code(2)
        .assert_contains(".env.example does not exist");
    assert_eq!(p.read(".env"), "X=1\n");
}

#[test]
fn b11_all_leaves_backups_alone_and_edits_the_template_as_the_template() {
    let p = Project::new()
        .env("X=1\n")
        .file(".env.bak", "X=old\n")
        .file(".env.backup", "X=old\n")
        .example("X=\n");
    evnx(&p, &["remove", "X", "--all", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env.bak"), "X=old\n");
    assert_eq!(p.read(".env.backup"), "X=old\n");
    assert_eq!(p.read(".env.example"), "");
    // The template is committed: git is its backup.
    assert!(!p.exists(".env.example.bak"));
}

// ── Group C: the contract ───────────────────────────────────────────────────

#[test]
fn c1_c3_c8_the_rest_of_the_contract_is_byte_identical() {
    let toml = "\
[defaults]
env_name = \"staging\"

[vars.A]
environments = [\"production\"]

# about X
[vars.X]
required = true

[[scan.patterns]]
name = \"mine\"
regex = \"zz[0-9]{8}\"
";
    let p = Project::new().file(".env.staging", "A=1\nX=2\n").toml(toml);
    evnx(&p, &["remove", "X", "--contract", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env.staging"), "A=1\n");
    assert_eq!(
        p.read(".evnx.toml"),
        toml.replace("# about X\n[vars.X]\nrequired = true\n\n", "")
    );
}

#[test]
fn c2_the_inline_form() {
    let p = Project::new()
        .env("X=1\nY=2\n")
        .toml("[vars]\nX = { format = \"port\" }\nY = { required = false }\n");
    evnx(&p, &["remove", "X", "--contract", "--yes"]).assert_code(0);
    assert_eq!(p.read(".evnx.toml"), "[vars]\nY = { required = false }\n");
}

#[test]
fn c4_a_required_name_left_in_the_contract_is_warned_about() {
    let p = clean_project();
    evnx(&p, &["remove", "DROP", "--yes"])
        .assert_code(0)
        .assert_contains("DROP is required in .evnx.toml [vars]");
}

#[test]
fn c5_the_question_names_a_required_entry() {
    let p = clean_project();
    let mut s = Pty::spawn(p.path(), &["remove", "DROP", "--contract"]);
    s.wait_for("(the contract marks DROP required)");
    s.press(&["n", ENTER]);
    let (_, code) = s.finish();
    assert_eq!(code, 4);
}

#[test]
fn c6_no_contract() {
    let p = Project::new().env("X=1\n");
    evnx(&p, &["remove", "X", "--contract", "--yes"])
        .assert_code(2)
        .assert_contains("no .evnx.toml");
    assert_eq!(p.read(".env"), "X=1\n");
    evnx(&p, &["remove", "X", "--all", "--yes"])
        .assert_code(0)
        .assert_contains("no .evnx.toml, skipped");
}

#[test]
fn c7_s4_a_malformed_contract_changes_no_file() {
    let p = Project::new()
        .env("X=1\n")
        .example("X=\n")
        .toml("[vars.X\nrequired = true\n");
    // From another folder, so the run reaches `remove` rather than stopping
    // at the config every command reads first; both are exit 2.
    let out = run(
        p.path().parent().unwrap(),
        &[
            "remove",
            "X",
            "--all",
            "--yes",
            "-p",
            &p.path().to_string_lossy(),
        ],
    );
    out.assert_code(2);
    assert_eq!(p.read(".env"), "X=1\n");
    assert_eq!(p.read(".env.example"), "X=\n");
    assert!(!p.exists(".env.bak"));
}

// ── Group D: safety and interface ───────────────────────────────────────────

/// Every file in the project, with its bytes, `.git` left out.
fn snapshot(p: &Project) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(p.path()).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        out.push((name, std::fs::read(e.path()).unwrap_or_default()));
    }
    out.sort();
    out
}

#[test]
fn d1_s8_no_terminal_and_no_yes_refuses() {
    let p = Project::new().env("X=1\n");
    evnx(&p, &["remove", "X"])
        .assert_code(2)
        .assert_contains("--yes");
    assert_eq!(p.read(".env"), "X=1\n");
}

#[test]
fn d2_s9_a_dry_run_writes_nothing() {
    let p = clean_project();
    let before = snapshot(&p);
    evnx(&p, &["remove", "DROP", "--all", "--dry-run"])
        .assert_code(0)
        .assert_contains("Dry run: nothing was written.");
    evnx(&p, &["remove", "DROP", "--all", "--dry-run", "--json"]).assert_code(0);
    assert_eq!(snapshot(&p), before);
}

#[test]
fn d4_answering_no_is_exit_4_and_changes_nothing() {
    let p = Project::new().env("X=1\n");
    let mut s = Pty::spawn(p.path(), &["remove", "X"]);
    s.wait_for("Remove X from 1 place?");
    s.press(&["n", ENTER]);
    let (rest, code) = s.finish();
    assert_eq!(code, 4, "{rest}");
    assert!(rest.contains("Cancelled; nothing was changed."), "{rest}");
    assert_eq!(p.read(".env"), "X=1\n");
    assert!(!p.exists(".env.bak"));

    let mut s = Pty::spawn(p.path(), &["remove", "X"]);
    s.wait_for("Remove X from 1 place?");
    s.press(&["y", ENTER]);
    let (_, code) = s.finish();
    assert_eq!(code, 0);
    assert_eq!(p.read(".env"), "");
}

/// D-5, D-6, S-1: no value anywhere — not in any stream, mode or error, and
/// not in a backup's name.
#[test]
fn d5_d6_s1_no_value_leaks() {
    let s = sentinel();
    let p = clean_project();
    let mut outputs = Vec::new();
    for args in [
        &["remove", "DROP", "--all", "--dry-run"][..],
        &["remove", "DROP", "--all", "--dry-run", "--json"],
        &["-v", "remove", "DROP", "--all", "--dry-run"],
        &["remove", "DRP", "--yes"],
        &["remove", "drop", "--all", "--yes"],
        &["remove", "DROP"],
        &["-v", "remove", "DROP", "--all", "--yes"],
    ] {
        outputs.push(evnx(&p, args).all());
    }
    let p2 = Project::new()
        .env(&format!("A={s}\nB=2\n"))
        .mode(".env", 0o444);
    outputs.push(evnx(&p2, &["remove", "A", "--yes"]).all());
    let p3 = Project::new().env(&format!("A={s}\n")).toml("[vars.A\n");
    outputs.push(
        run(
            p3.path().parent().unwrap(),
            &[
                "remove",
                "A",
                "--all",
                "--yes",
                "-p",
                &p3.path().to_string_lossy(),
            ],
        )
        .all(),
    );
    for o in &outputs {
        assert!(!o.contains(&s), "value leaked:\n{o}");
    }
    for (name, _) in snapshot(&p) {
        assert!(!name.contains(&s));
    }
    // The run above removed it: no checked-in or live file holds it any more.
    for f in [".env", ".env.staging", ".env.production", ".env.example"] {
        assert!(!p.read(f).contains(&s), "{f}");
    }
}

#[test]
fn d7_quiet_prints_nothing_on_success() {
    let p = Project::new().env("X=1\nY=2\n");
    let out = evnx(&p, &["-q", "remove", "X", "--yes"]);
    out.assert_code(0);
    assert_eq!(out.stdout, "");
    let out = evnx(&p, &["-q", "remove", "NOPE", "--yes"]);
    out.assert_code(2).assert_contains("NOPE is not in");
}

#[test]
fn d8_d9_s10_globs_and_wide_matches() {
    let p = Project::new().env("TEST_A=1\nTEST_B=2\nKEEP=3\n");
    evnx(&p, &["remove", "TEST_*", "--dry-run"])
        .assert_code(0)
        .assert_contains("TEST_A (line 1), TEST_B (line 2)");
    evnx(&p, &["remove", "TEST_*", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "KEEP=3\n");

    let p = Project::new().env("A=1\nB=2\n");
    let mut s = Pty::spawn(p.path(), &["remove", "*"]);
    let (rest, code) = {
        s.wait_for("matches every variable");
        s.finish()
    };
    assert_eq!(code, 2, "{rest}");
    assert_eq!(p.read(".env"), "A=1\nB=2\n");
    evnx(&p, &["remove", "*", "--dry-run"])
        .assert_code(0)
        .assert_contains("A (line 1), B (line 2)");
    evnx(&p, &["remove", "*", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "");
}

#[test]
fn d10_s11_a_shell_expanded_file_name_is_refused() {
    let p = Project::new().env("A=1\n").file("foo.txt", "");
    evnx(&p, &["remove", "foo.txt", "--yes"])
        .assert_code(2)
        .assert_contains("is a file here");
    evnx(&p, &["remove", "a/b", "--yes"])
        .assert_code(2)
        .assert_contains("not a variable name");
    evnx(&p, &["remove", "A=1", "--yes"])
        .assert_code(2)
        .assert_contains("not a value");
    // D12: a dotted name that is not a path is a name, and simply not here.
    evnx(&p, &["remove", "app.port", "--yes"])
        .assert_code(2)
        .assert_contains("app.port is not in");
}

#[test]
fn d12_d13_d14_git_warnings_and_no_backup() {
    let p = Project::new().git().env("X=1\nY=2\n");
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains("not covered by .gitignore");

    let p = Project::new().git().env("X=1\nY=2\n");
    p.force_add(".env");
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains("tracked by git");
    evnx(&p, &["remove", "Y", "--yes", "--no-backup"])
        .assert_code(2)
        .assert_contains("--no-backup is refused");

    let p = Project::new().env("X=1\nY=2\n");
    evnx(&p, &["remove", "X", "--no-backup"])
        .assert_code(2)
        .assert_contains("--no-backup needs --yes");
    evnx(&p, &["remove", "X", "--no-backup", "--yes"]).assert_code(0);
    assert!(!p.exists(".env.bak"));
}

#[test]
fn d15_s18_a_second_run() {
    let p = Project::new().env("X=1\nY=2\n");
    evnx(&p, &["remove", "X", "--yes"]).assert_code(0);
    let mtime = std::fs::metadata(p.join(".env"))
        .unwrap()
        .modified()
        .unwrap();
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(2)
        .assert_contains("--ignore-missing");
    evnx(&p, &["remove", "X", "--yes", "--ignore-missing"])
        .assert_code(0)
        .assert_contains("Nothing to remove.");
    assert!(!p.exists(".env.bak.2"));
    assert_eq!(
        std::fs::metadata(p.join(".env"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}

#[test]
fn d16_s20_removal_is_not_rotation() {
    let p = Project::new().env("X=1\n");
    evnx(&p, &["remove", "X", "--yes"])
        .assert_code(0)
        .assert_contains("Rotate it at the provider");
}

#[test]
fn d17_json_never_asks() {
    let p = Project::new().env("X=1\n");
    evnx(&p, &["remove", "X", "--json"])
        .assert_code(2)
        .assert_contains("--json needs --yes or --dry-run");
    let out = evnx(&p, &["remove", "X", "--json", "--yes"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect(&out.stdout);
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["removed"][0]["name"], "X");
    assert_eq!(doc["removed"][0]["lines"], serde_json::json!([1]));
    assert_eq!(doc["backups"], serde_json::json!(["./.env.bak"]));
}

// ── Group E: many files ─────────────────────────────────────────────────────

#[test]
fn e1_e4_all_edits_where_present_and_says_where_not() {
    let p = Project::new()
        .env("X=1\n")
        .file(".env.staging", "X=2\nY=1\n")
        .file(".env.production", "Y=1\n")
        .example("X=\nY=\n")
        .toml("[vars.X]\n[vars.Y]\n");
    evnx(&p, &["remove", "X", "--all", "--dry-run"])
        .assert_code(0)
        .assert_contains("./.env.production  not present");
    evnx(&p, &["remove", "X", "--all", "--yes"]).assert_code(0);
    assert_eq!(p.read(".env"), "");
    assert_eq!(p.read(".env.staging"), "Y=1\n");
    assert_eq!(p.read(".env.production"), "Y=1\n");
    assert_eq!(p.read(".env.example"), "Y=\n");
    assert_eq!(p.read(".evnx.toml"), "[vars.Y]\n");
}

#[test]
fn e2_one_file_that_cannot_be_planned_stops_every_file() {
    let p = Project::new()
        .env("X=1\n")
        .file(".env.staging", "X=2\nnot an assignment\n")
        .example("X=\n");
    let before = snapshot(&p);
    evnx(&p, &["remove", "X", "--all", "--yes"])
        .assert_code(2)
        .assert_contains(".env.staging has a line evnx cannot read");
    assert_eq!(snapshot(&p), before);
}

// ── Group H: every checker agrees afterwards (§1.1) ─────────────────────────

#[test]
fn h1_to_h5_after_remove_all_every_checker_is_clean() {
    let p = clean_project();
    for args in [
        &["validate"][..],
        &["spec", "check"],
        &["sync", "--check"],
        &["diff", "--env-name", "staging", "--against", "production"],
    ] {
        evnx(&p, args).assert_code(0);
    }
    let scan_before = evnx(&p, &["scan", "--format", "json"]);
    evnx(&p, &["remove", "DROP", "--all", "--yes"]).assert_code(0);
    for args in [
        &["validate"][..],
        &["spec", "check"],
        &["sync", "--check"],
        &["diff", "--env-name", "staging", "--against", "production"],
    ] {
        evnx(&p, args).assert_code(0);
    }
    // H-5: no new findings. The run made `.bak` copies, which are not
    // findings either: `.gitignore` covers them.
    let scan_after = evnx(&p, &["scan", "--format", "json"]);
    let findings = |o: &support::run::Output| {
        serde_json::from_str::<serde_json::Value>(&o.stdout)
            .map(|d| d["findings"].as_array().map(Vec::len).unwrap_or(0))
            .unwrap_or(0)
    };
    assert!(
        findings(&scan_after) <= findings(&scan_before),
        "{}",
        scan_after.stdout
    );
}

#[test]
fn h6_a_narrow_removal_leaves_drift_that_the_hint_names() {
    let p = clean_project();
    evnx(&p, &["remove", "DROP", "--yes"])
        .assert_code(0)
        .assert_contains("./.env.example     DROP             add --template");
    evnx(&p, &["sync", "--check"]).assert_code(1);
}

// ── Group I: surface ────────────────────────────────────────────────────────

#[test]
fn i1_commands_json_lists_remove_and_rm() {
    let p = Project::new();
    let out = evnx(&p, &["commands", "--json"]);
    out.assert_code(0);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let remove = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "remove")
        .expect("remove listed");
    assert!(remove["visible_aliases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "rm"));
    assert_eq!(
        remove["docs_url"],
        "https://docs.evnx.dev/cli/commands/remove"
    );
}

#[test]
fn i2_help_shows_exit_codes_and_examples() {
    let p = Project::new();
    evnx(&p, &["remove", "--help"])
        .assert_code(0)
        .assert_contains("4 you answered no")
        .assert_contains("evnx rm 'TEST_*'")
        .assert_contains("--ignore-missing");
}

#[test]
fn i4_sync_points_at_remove() {
    let p = Project::new().env("A=1\n").example("A=\nSTALE=\n");
    run_with(p.path(), &["sync"], &[], None).assert_contains("evnx remove STALE --template");
}
