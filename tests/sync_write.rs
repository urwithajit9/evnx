//! What `sync` writes and says when it writes (R-sync, 0.10.12).
//!
//! - I-120, DEC-S10, DEC-S19: placeholders are chosen by whole `_`-separated
//!   words, URL placeholders follow the value's scheme, and `DEBUG` keeps its
//!   own value. `add custom` shares the generator, so it is fixed with it.
//! - I-124, DEC-S9: with no terminal, sync uses placeholders on an existing
//!   template and in reverse, and says so; `--placeholder` is a deprecated
//!   alias of `--force`.
//! - I-125: messages name the files actually in play.
//! - I-126: "1 variable", modes as `644`; I-276: one permission warning.
//! - I-129, DEC-S20: a new `.env` does not carry the template's
//!   "# Generated from .env" header.
//! - The dry-run preview names the file a write would really go to.

mod support;

use support::{run, run_with, Project};

// ── I-120: placeholder rules ────────────────────────────────────────────────

/// T-SYNC-26..28, through the binary: the demo's S5 table.
#[test]
fn t_i120_placeholders_follow_whole_words_and_schemes() {
    let p = Project::new().env(
        "SUPPORT_EMAIL=help@acme.io\nMONKEY_COUNT=3\nBYPASS_CACHE=1\nGHOST_MODE=on\n\
         DEBUG=false\nDB_URL=postgres://u:p@h/db\nMONGO_URL=mongodb://u:p@h/db\n\
         API_PORT=80\nSTRIPE_KEY=x\nDB_PASSWORD=x\n",
    );
    run(p.path(), &["sync", "-f"]).assert_code(0);
    let t = p.read(".env.example");
    let value = |k: &str| {
        t.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("{k} not written:\n{t}"))
            .to_string()
    };
    for k in [
        "SUPPORT_EMAIL",
        "MONKEY_COUNT",
        "BYPASS_CACHE",
        "GHOST_MODE",
    ] {
        assert_eq!(value(k), "YOUR_VALUE_HERE", "{k}:\n{t}");
    }
    assert_eq!(value("DEBUG"), "false", "DEBUG keeps its value");
    assert!(value("DB_URL").starts_with("postgres://"), "{t}");
    assert!(value("MONGO_URL").starts_with("mongodb://"), "{t}");
    for k in ["DB_URL", "MONGO_URL"] {
        assert!(!value(k).contains("u:p@h"), "{k} copied the credential");
    }
    // The rules that were right stay as they were (T-SYNC-29).
    assert_eq!(value("API_PORT"), "8000");
    assert_eq!(value("STRIPE_KEY"), "YOUR_KEY_HERE");
    assert_eq!(value("DB_PASSWORD"), "YOUR_PASSWORD_HERE");
}

/// DEC-S19: `add custom` writes sync's placeholder, so the same fix reaches it.
#[test]
fn t_i120_add_custom_uses_the_same_placeholder_rules() {
    let p = Project::new().env("").example("");
    run(
        p.path(),
        &["add", "custom", "SUPPORT_EMAIL=help@acme.io", "--yes"],
    )
    .assert_code(0);
    let t = p.read(".env.example");
    assert!(
        t.lines().any(|l| l == "SUPPORT_EMAIL=YOUR_VALUE_HERE"),
        "not `8000` because the name contains PORT:\n{t}"
    );
}

// ── I-124, DEC-S9: no terminal ──────────────────────────────────────────────

/// T-SYNC-45: an existing template, drift, no terminal, no flag.
#[test]
fn t_i124_no_terminal_uses_placeholders_on_an_existing_template() {
    let p = Project::new().env("A=1\nNEW_KEY=abc\n").example("A=\n");
    let out = run(p.path(), &["sync"]);
    out.assert_code(0);
    out.assert_not_contains("not a terminal");
    out.assert_contains("no terminal");
    assert!(p.read(".env.example").contains("NEW_KEY=YOUR_KEY_HERE"));
}

/// Reverse onto an existing `.env`, no terminal.
#[test]
fn t_i124_no_terminal_reverse_adds_the_templates_values() {
    let p = Project::new().env("A=1\n").example("A=\nPORT=8080\n");
    let out = run(p.path(), &["sync", "--direction", "reverse"]);
    out.assert_code(0);
    out.assert_contains("no terminal");
    assert!(p.read(".env").contains("PORT=8080"));
}

/// T-SYNC-66: `CI` set is treated as no terminal (DEC-S9, DEC-S18). Without
/// a terminal the run cannot tell the two apart except by what it says.
#[test]
fn t_i124_ci_is_named_as_the_reason() {
    let p = Project::new().env("A=1\nB=2\n").example("A=\n");
    let out = run_with(p.path(), &["sync"], &[("CI", "true")], None);
    out.assert_code(0);
    out.assert_contains("CI");
    // `CI=false` is not CI.
    let p = Project::new().env("A=1\nB=2\n").example("A=\n");
    let out = run_with(p.path(), &["sync"], &[("CI", "false")], None);
    out.assert_code(0);
    out.assert_contains("no terminal");
}

/// T-SYNC-46 / T-SYNC-65: `--placeholder` works on an existing template and
/// says it is deprecated.
#[test]
fn t_i124_placeholder_flag_is_a_deprecated_alias_of_force() {
    let p = Project::new().env("A=1\nB=2\n").example("A=\n");
    let out = run(p.path(), &["sync", "--placeholder"]);
    out.assert_code(0);
    out.assert_contains("deprecated");
    out.assert_contains("--force");
    assert!(p.read(".env.example").contains("B=YOUR_VALUE_HERE"));
}

// ── I-125: real file names ──────────────────────────────────────────────────

/// T-SYNC-49.
#[test]
fn t_i125_messages_name_the_files_in_play() {
    let p = Project::new();
    let out = run(p.path(), &["sync", "--env", "nope.env"]);
    out.assert_code(2);
    out.assert_contains("nope.env");
    out.assert_not_contains("File not found: .env\n");

    let p = Project::new()
        .file("a.env", "A=1\nB=2\n")
        .file("tmpl", "A=\n");
    let out = run(
        p.path(),
        &["sync", "-f", "--env", "a.env", "--example", "tmpl"],
    );
    out.assert_code(0);
    out.assert_contains("to tmpl");
    out.assert_not_contains(".env.example");

    let p = Project::new().file("tmpl", "A=\n");
    let out = run(
        p.path(),
        &[
            "sync",
            "-f",
            "--direction",
            "reverse",
            "--env",
            "b.env",
            "--example",
            "tmpl",
        ],
    );
    out.assert_code(0);
    out.assert_contains("Created b.env");

    let p = Project::new()
        .file("bad.env", "X=\"open\n")
        .file("tmpl", "A=\n");
    let out = run(
        p.path(),
        &["sync", "-n", "--env", "bad.env", "--example", "tmpl"],
    );
    out.assert_code(2);
    out.assert_contains("Failed to parse bad.env");
}

// ── I-126, I-276: wording and one permission warning ────────────────────────

/// T-SYNC-50.
#[test]
fn t_i126_singular_and_plain_octal() {
    let p = Project::new().env("A=1\nB=2\n").example("A=\n");
    run(p.path(), &["sync", "-f"])
        .assert_code(0)
        .assert_contains("Added 1 variable to .env.example")
        .assert_not_contains("1 variables");
}

#[cfg(unix)]
#[test]
fn t_i276_one_permission_warning_in_plain_octal() {
    let p = Project::new()
        .env("A=1\n")
        .mode(".env", 0o644)
        .example("A=\nB=\n");
    let out = run(p.path(), &["sync", "-f", "--direction", "reverse"]);
    out.assert_code(0);
    out.assert_not_contains("0o644");
    let warnings = out.all().matches("mode 644").count();
    assert_eq!(warnings, 1, "one warning for one file:\n{}", out.all());
    out.assert_contains("chmod 600 .env");
}

// ── I-129, DEC-S20 ──────────────────────────────────────────────────────────

/// T-SYNC-18.
#[test]
fn t_i129_new_env_has_no_stale_header() {
    let p = Project::new().example(
        "# Generated from .env\n# Replace all placeholder values with real credentials\n\n\
         # Database\nA=1\n",
    );
    run(p.path(), &["sync", "-f", "--direction", "reverse"]).assert_code(0);
    let env = p.read(".env");
    assert!(!env.contains("Generated from .env"), "{env}");
    assert!(
        env.contains("# Created from .env.example by evnx sync"),
        "{env}"
    );
    assert!(env.contains("# Database\nA=1"), "the rest is kept:\n{env}");
}

// ── The dry-run preview names the real target ───────────────────────────────

#[cfg(unix)]
#[test]
fn t_preview_names_a_symlinks_target() {
    let p = Project::new()
        .env("A=1\nB=2\n")
        .dir("shared")
        .file("shared/real.example", "A=\n")
        .symlink("shared/real.example", ".env.example");
    run(p.path(), &["sync", "-n"])
        .assert_code(0)
        .assert_contains("shared/real.example");
}
