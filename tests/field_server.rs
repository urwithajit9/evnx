//! The field test of 2026-11-15 (review file 31), replayed: a server project
//! with only named environment files — `.env.prod` and its own template
//! `.env.prod.example`, beside the shared `.env.example` — and no `.env`.
//!
//! Every test here failed on 0.14.1; WP-5 (`core::resolve`) is what makes
//! them pass. Each names the finding it covers.

mod support;

use support::{Project, run};

/// The server's shape. `.env.example` is the development template; production
/// keeps its own names in `.env.prod.example`.
fn server() -> Project {
    Project::new()
        .file(
            ".env.prod",
            "DATABASE_URL=postgres://prod\nPADDLE_KEY=pdl_live_x\n",
        )
        .file(".env.prod.example", "DATABASE_URL=\nPADDLE_KEY=\n")
        .example("DATABASE_URL=\n")
        .file(".gitignore", ".env*\n!.env*.example\n")
        .private(".env.prod")
        .git()
}

/// FLD-3: the file's name is accepted as the environment's name.
#[test]
fn fld3_env_name_takes_the_file_name() {
    let p = server();
    let out = run(
        p.path(),
        &[
            "add",
            "custom",
            "K1=v1",
            "--env-name=.env.prod",
            "--dry-run",
        ],
    );
    out.assert_code(0);
    out.assert_contains(".env.prod");
    out.assert_not_contains(".env..env");
    run(p.path(), &["sync", "--env-name", ".env.prod", "--check"]).assert_code(0);
}

/// FLD-4: with `--env-name prod` the template is `.env.prod.example`, and the
/// shared `.env.example` is never touched.
#[test]
fn fld4_the_environment_pairs_with_its_own_template() {
    let p = server();
    let shared = p.read(".env.example");

    let out = run(
        p.path(),
        &[
            "add",
            "custom",
            "SERVER_DATA_KEY=abc",
            "--env-name",
            "prod",
            "--yes",
        ],
    );
    out.assert_code(0);
    assert!(p.read(".env.prod.example").contains("SERVER_DATA_KEY="));
    assert!(p.read(".env.prod").contains("SERVER_DATA_KEY=abc"));
    assert_eq!(
        p.read(".env.example"),
        shared,
        "add wrote the shared template"
    );

    let out = run(p.path(), &["sync", "--env-name", "prod", "--yes"]);
    out.assert_code(0);
    assert_eq!(
        p.read(".env.example"),
        shared,
        "sync copied production names into it"
    );
    out.assert_contains(".env.prod ↔ .env.prod.example");

    // validate compares the pair; the template is not a sibling env file.
    // --exit-zero: the fixture's values are weak on purpose; the pairing is
    // what is checked here.
    let out = run(p.path(), &["validate", "--env-name", "prod", "--exit-zero"]);
    out.assert_code(0);
    out.assert_not_contains("not in .env.prod.example");
    out.assert_contains("template matched to --env-name prod");
}

/// FLD-6: a command that needs `.env` names `.env.prod`, the one-off flag
/// and the `[defaults]` block, and does not say "run evnx init".
#[test]
fn fld6_the_missing_env_message_names_what_is_there() {
    let p = server();
    for args in [
        &["sync", "--check"][..],
        &["sync"][..],
        &["validate"][..],
        &["remove", "PADDLE_KEY", "--dry-run"][..],
    ] {
        let out = run(p.path(), args);
        out.assert_code(2);
        out.assert_contains("Found: .env.prod");
        out.assert_contains("--env-name prod");
        out.assert_contains("env_name = \"prod\"");
        out.assert_contains("example  = \".env.prod.example\"");
        out.assert_not_contains("Getting started");
        out.assert_not_contains("hasn't been initialized");
    }
}

/// The setting the review recommended: with `[defaults]`, every command uses
/// the pair with no flags, and says so.
#[test]
fn defaults_make_every_command_use_the_pair() {
    let p = server().toml("[defaults]\nenv_name = \"prod\"\n");
    run(p.path(), &["sync", "--check"]).assert_code(0);
    let out = run(p.path(), &["validate", "--exit-zero"]);
    out.assert_code(0);
    out.assert_contains("template matched to [defaults] env_name = \"prod\"");
    let out = run(p.path(), &["add", "custom", "K=v", "--dry-run"]);
    out.assert_code(0);
    out.assert_contains(".env.prod.example");
    run(p.path(), &["remove", "PADDLE_KEY", "--dry-run"]).assert_code(0);
}

/// INIT-3, ADD-6: a misspelt environment is not created silently.
#[test]
fn init3_a_misspelt_environment_is_caught() {
    let p = server();
    let out = run(
        p.path(),
        &["add", "custom", "K=v", "--env-name", "prodution"],
    );
    // No terminal and no --yes to confirm a new environment: declined (WP-9 D3).
    out.assert_code(4);
    out.assert_contains("did you mean 'production'?");
    assert!(!p.exists(".env.prodution"));
    let out = run(p.path(), &["init", "--env-name", "stagng", "--yes"]);
    out.assert_contains("did you mean 'staging'?");
}

/// The `.gitignore` exception: a new environment template stays tracked.
#[test]
fn a_new_environment_template_is_not_ignored() {
    let p = Project::new().git();
    run(p.path(), &["init", "--yes"]).assert_code(0);
    std::fs::write(p.path().join(".env.staging.example"), "A=\n").unwrap();
    assert!(!p.is_ignored(".env.staging.example"));
    assert!(!p.is_ignored(".env.example"));
    assert!(p.is_ignored(".env.staging"));
}
