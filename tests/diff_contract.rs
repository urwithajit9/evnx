//! `evnx diff` after R-diff (0.10.13).
//!
//! - I-078, DEC-DF1 (owner, 2026-10-21): against a template, key names only. A filled-in
//!   `.env` differs from its `.env.example` in every value by construction, so
//!   `evnx diff` exited 1 on every real project. The right-hand side decides:
//!   the template (default `.env.example`, `[defaults] example`, `--example`)
//!   compares keys; `--against NAME` compares two environments, values too;
//!   `--values` opts in against a template. `--fail-on` picks what fails.
//! - DEC-DF2 (owner, 2026-10-21): without `--show-values` a secret-named key prints its
//!   name only, like every other key — no `= ***`.
//! - I-076b: JSON `value_state` (`withheld | masked | shown`), `schema_version`.
//! - I-075: `--with-stats` without json and `--interactive` without patch are
//!   errors, not silent no-ops.
//! - I-077: the ignored count is printed; `-v` names ignore entries that
//!   matched nothing; JSON lists what was ignored.
//! - I-079: `-v` says which files were compared and how.
//! - I-080: an error is not preceded by a header announcing the comparison.
//! - D-39, DEC-DF3 (owner, 2026-10-21): the `evnx-diff` hook fails on missing keys only
//!   and skips a clone with no `.env`.

mod support;

use support::{run, Project};

/// Same keys, every value different: a `.env` filled in from its template.
fn filled_in() -> Project {
    Project::new()
        .example("JWT_SECRET=change-me\nDEBUG=false\nAPI_URL=http://localhost\n")
        .env("JWT_SECRET=real-jwt-AAAA\nDEBUG=true\nAPI_URL=https://api.example.test\n")
}

fn json(out: &support::Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}:\n{}", out.stdout))
}

// ─── I-078: what is compared ────────────────────────────────────────────────

#[test]
fn t_i078_a_template_compares_key_names_only() {
    let p = filled_in();
    let out = run(p.path(), &["diff"]);
    out.assert_code(0);
    out.assert_not_contains("Different values");
    assert!(
        out.stdout.contains("have the same keys"),
        "a key-only run must not claim the files are identical:\n{}",
        out.stdout
    );
    assert!(out.stderr.contains("--values"), "{}", out.stderr);
}

#[test]
fn t_i078_example_names_a_template_too() {
    let p = Project::new()
        .file("a.env", "A=1\nB=2\n")
        .file("b.env", "A=x\nB=y\n");
    run(p.path(), &["diff", "--env", "a.env", "--example", "b.env"]).assert_code(0);
}

#[test]
fn t_i078_values_compares_values_against_a_template() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "--values"]);
    out.assert_code(1);
    out.assert_contains("Different values");
    out.assert_contains("~ DEBUG");
}

#[test]
fn t_i078_against_compares_two_environments_with_values() {
    let p = Project::new()
        .file(".env.production", "A=1\nB=2\n")
        .file(".env.staging", "A=1\nB=3\n");
    let out = run(
        p.path(),
        &["diff", "--env-name", "production", "--against", "staging"],
    );
    out.assert_code(1);
    out.assert_contains("~ B");
}

#[test]
fn t_i078_fail_on_missing_passes_when_only_extra_keys() {
    let p = Project::new().example("A=x\n").env("A=1\nLOCAL_ONLY=1\n");
    let out = run(p.path(), &["diff", "--fail-on", "missing"]);
    out.assert_code(0);
    // The report is unchanged: --fail-on decides the exit code only.
    out.assert_contains("Extra in .env");

    let behind = Project::new().example("A=x\nNEW=y\n").env("A=1\n");
    run(behind.path(), &["diff", "--fail-on", "missing"]).assert_code(1);
}

#[test]
fn t_i078_fail_on_refuses_an_unknown_value() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "--fail-on", "missing,nope"]);
    out.assert_code(2);
    for valid in ["missing", "extra", "different"] {
        out.assert_contains(valid);
    }
}

#[test]
fn t_i078_fail_on_different_needs_values() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "--fail-on", "different"]);
    out.assert_code(2);
    out.assert_contains("--values");
    run(p.path(), &["diff", "--values", "--fail-on", "different"]).assert_code(1);
}

#[test]
fn t_i078_json_says_what_was_compared() {
    let p = filled_in();
    let keys = json(&run(p.path(), &["diff", "--format", "json"]));
    assert_eq!(keys["schema_version"], 1, "{keys}");
    assert_eq!(keys["compared"], "keys", "{keys}");
    assert_eq!(keys["different"], serde_json::json!([]), "{keys}");
    assert_eq!(keys["summary"]["total"], 0, "{keys}");

    let values = json(&run(p.path(), &["diff", "--format", "json", "--values"]));
    assert_eq!(values["compared"], "values", "{values}");
    assert_eq!(values["summary"]["different"], 3, "{values}");
}

#[test]
fn t_i078_patch_against_a_template_has_no_update_section() {
    let p = Project::new().example("A=x\nNEW=y\n").env("A=1\n");
    let out = run(p.path(), &["diff", "--format", "patch"]);
    out.assert_contains("+ NEW=");
    out.assert_not_contains("# Update these");
}

// ─── DEC-DF2: no mask line without --show-values ────────────────────────────

#[test]
fn t_dec_df2_a_secret_name_prints_no_mask_without_show_values() {
    let p = Project::new()
        .example("JWT_SECRET=change-me\n")
        .env("JWT_SECRET=real-jwt-AAAA\nSECRET_KEY=django-BBBB\n");
    for args in [&["diff"][..], &["diff", "--values"][..]] {
        let out = run(p.path(), args);
        out.assert_not_contains("***");
        out.assert_contains("- SECRET_KEY");
    }
    // Asked for, the value is masked by its name.
    run(p.path(), &["diff", "--values", "--show-values"]).assert_contains("***");
}

// ─── I-076b: value_state ────────────────────────────────────────────────────

#[test]
fn t_i076b_value_state_says_why_a_value_is_or_is_not_there() {
    let p = Project::new()
        .example("PORT=8080\nAPI_KEY=example\n")
        .env("PORT=9090\nAPI_KEY=real-key-AAAA\n");
    let state = |args: &[&str], key: &str| -> serde_json::Value {
        let report = json(&run(p.path(), args));
        report["different"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["key"] == key)
            .unwrap_or_else(|| panic!("{key} not in {report}"))
            .clone()
    };
    let base = ["diff", "--format", "json", "--values"];
    for key in ["PORT", "API_KEY"] {
        let item = state(&base, key);
        assert_eq!(item["value_state"], "withheld", "{item}");
        assert_eq!(item["env_value"], serde_json::Value::Null, "{item}");
        // Kept for one release beside value_state.
        assert_eq!(item["values_redacted"], false, "{item}");
    }
    let shown = [&base[..], &["--show-values"]].concat();
    assert_eq!(state(&shown, "PORT")["value_state"], "shown");
    let masked = state(&shown, "API_KEY");
    assert_eq!(masked["value_state"], "masked", "{masked}");
    assert_eq!(masked["values_redacted"], true, "{masked}");
}

// ─── I-075: flags that do nothing in this mode are errors ───────────────────

#[test]
fn t_i075_with_stats_needs_json() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "--with-stats"]);
    out.assert_code(2);
    out.assert_contains("--format json");
    run(p.path(), &["diff", "--with-stats", "--format", "json"]).assert_code(0);
}

#[test]
fn t_i075_interactive_needs_patch() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "--interactive"]);
    out.assert_code(2);
    out.assert_contains("--format patch");
}

// ─── I-077: ignored keys are counted ────────────────────────────────────────

#[test]
fn t_i077_ignored_keys_are_counted_and_listed() {
    let p = Project::new().example("A=1\n").env("A=1\nB=2\n");
    let out = run(p.path(), &["diff", "--ignore-keys", "B,NOPE"]);
    out.assert_code(0);
    out.assert_contains("1 key ignored");
    // Names that matched nothing are listed at -v only.
    out.assert_not_contains("NOPE");
    run(p.path(), &["diff", "-v", "--ignore-keys", "B,NOPE"]).assert_contains("NOPE");

    let report = json(&run(
        p.path(),
        &["diff", "--format", "json", "--ignore-keys", "B,NOPE"],
    ));
    assert_eq!(report["ignored"], serde_json::json!(["B"]), "{report}");
}

// ─── I-079: -v says what was compared ───────────────────────────────────────

#[test]
fn t_i079_verbose_names_the_files_and_the_comparison() {
    let p = filled_in();
    let out = run(p.path(), &["diff", "-v"]);
    assert!(
        !out.stderr.contains("Running diff in verbose mode"),
        "{}",
        out.stderr
    );
    for needle in [".env.example", "template", "key names"] {
        assert!(out.stderr.contains(needle), "{needle}:\n{}", out.stderr);
    }
}

// ─── I-080: no header before an error ───────────────────────────────────────

#[test]
fn t_i080_an_error_is_not_preceded_by_a_header() {
    let p = Project::new().example("A=1\n");
    let out = run(p.path(), &["diff"]);
    out.assert_code(2);
    out.assert_contains("No verdict");
    out.assert_not_contains("evnx diff  .env");
}

// ─── D-39: the pre-commit hook ──────────────────────────────────────────────

#[test]
fn t_d39_skip_without_env_skips_a_fresh_clone() {
    let p = Project::new().example("A=1\n");
    let out = run(p.path(), &["diff", "--skip-without-env"]);
    out.assert_code(0);
    assert!(out.stderr.contains("skipped"), "{}", out.stderr);

    // Only a missing .env is skipped; a missing template is still an error.
    let no_template = Project::new().env("A=1\n");
    run(no_template.path(), &["diff", "--skip-without-env"]).assert_code(2);
}

/// The `entry` of the `evnx-diff` hook, run as pre-commit runs it.
fn hook_args() -> Vec<String> {
    let hooks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".pre-commit-hooks.yaml"),
    )
    .unwrap();
    let block = hooks
        .split("- id: ")
        .find(|b| b.starts_with("evnx-diff\n"))
        .expect("evnx-diff hook");
    let entry = block
        .lines()
        .find_map(|l| l.trim().strip_prefix("entry: "))
        .expect("entry");
    let mut words = entry.split_whitespace().map(str::to_string);
    assert_eq!(words.next().as_deref(), Some("evnx"));
    words.collect()
}

#[test]
fn t_d39_hook_fails_on_missing_keys_only() {
    let args = hook_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    // A fresh clone: a template and no .env.
    let fresh = Project::new().example("A=1\n");
    run(fresh.path(), &args).assert_code(0);

    // Filled in, plus a local-only key: nothing is missing.
    let ahead = Project::new().example("A=x\n").env("A=1\nLOCAL_ONLY=1\n");
    run(ahead.path(), &args).assert_code(0);

    // The template has a key .env does not.
    let behind = Project::new().example("A=x\nNEW=y\n").env("A=1\n");
    run(behind.path(), &args).assert_code(1);
}
