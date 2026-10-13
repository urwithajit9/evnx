//! The files `evnx add` writes are a contract. These tests pin them byte for byte.
//!
//! # Why this exists
//!
//! `add` appends to two files people commit and edit (`.env.example`) or run
//! with (`.env`). The 0.10.2 work changes what it appends on purpose (skipping
//! names that exist, I-016; the category line, N-17; line endings, N-16); every
//! such change has to show up here as a reviewed diff, and nothing else may.
//!
//! What is pinned: both files after `add service` (a service, an infra id, a
//! framework id), `add framework`, and **every** blueprint, each run once on
//! the same small project. `add custom` is pinned in `add_pty.rs`. Terminal
//! output is not pinned here; specific messages are pinned where a defect is
//! known, in the `known_*` tests.
//!
//! # Updating
//!
//! ```bash
//! UPDATE_GOLDEN=1 cargo test --all-features --test add_golden
//! git diff tests/golden/add/      # read every line before committing
//! ```
//!
//! ⚠️ A diff here is either a bug or a decision. If it is a decision, name it
//! (the `CH-ADD-nn` entry or the issue id) in the commit message.

mod support;

use support::snapshot::{check_golden, normalise};
use support::{run, Project};

/// Every blueprint id, in the order `evnx add blueprint <unknown>` lists them.
/// `blueprint_list_matches_the_binary` keeps this in step with the schema.
const BLUEPRINTS: &[&str] = &[
    "t3_modern",
    "mern_v2",
    "fastapi_ai",
    "rust_high_perf",
    "laravel_monolith",
    "go_microservice",
    "supabase_fullstack",
    "django_enterprise",
    "firebase_mobile",
    "content_marketing",
];

/// The project every scenario starts from: one variable in each file, `.env`
/// ignored.
fn project() -> Project {
    Project::new()
        .example("# Minimal project\nAPP_NAME=test\n")
        .env("APP_NAME=my-app\n")
        .file(".gitignore", ".env\n")
}

/// Run `evnx add <args> --yes` on a fresh project and pin both files as
/// `tests/golden/add/<name>.env.example` and `<name>.env`.
fn pin(name: &str, args: &[&str]) {
    let p = project();
    let mut full = vec!["add"];
    full.extend_from_slice(args);
    full.push("--yes");
    run(p.path(), &full).assert_code(0);
    for file in [".env.example", ".env"] {
        check_golden(
            &format!("add/{name}{file}"),
            &normalise(&p.read(file), &[p.path()]),
        );
    }
}

#[test]
fn service_postgresql() {
    pin("service-postgresql", &["service", "postgresql"]);
}

/// An infra id through `add service` (I-018: accepted, by decision).
#[test]
fn service_vercel() {
    pin("service-vercel", &["service", "vercel"]);
}

/// A framework id through `add service`.
#[test]
fn service_nextjs() {
    pin("service-nextjs", &["service", "nextjs"]);
}

#[test]
fn framework_django() {
    pin(
        "framework-django",
        &["framework", "--language", "python", "django"],
    );
}

#[test]
fn every_blueprint() {
    for id in BLUEPRINTS {
        pin(&format!("blueprint-{id}"), &["blueprint", id]);
    }
}

/// A new blueprint in the schema fails here until it has golden files.
#[test]
fn blueprint_list_matches_the_binary() {
    let p = Project::new();
    let out = run(
        p.path(),
        &["add", "blueprint", "no_such_blueprint", "--yes"],
    );
    out.assert_code(2);
    let listed: Vec<&str> = out
        .stderr
        .lines()
        .skip_while(|l| !l.starts_with("Available blueprints:"))
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .filter(|w| {
            w.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        })
        .collect();
    assert_eq!(listed, BLUEPRINTS, "{}", out.stderr);
}
