//! `evnx convert --only secrets|public` (RM-67): split a `.env` by the shared
//! classifier (`core::classify::holds_secret`, by name or by value).

mod support;

use support::{run, Project};

const ENV: &str = "APP_NAME=shop\n\
PORT=8080\n\
DB_PASSWORD=hunter22hunter\n\
DATABASE_URL=postgres://u:pw@h/db\n\
STRIPE_KEY=sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9\n";

/// Sorted: the test's `serde_json` map orders keys.
fn keys(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    v.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn rm67_secrets_and_public_split_the_file() {
    let p = Project::new().env(ENV);
    let secrets = run(p.path(), &["convert", "--to", "json", "--only", "secrets"]);
    secrets.assert_code(0);
    // By name, by a URL's password, by a provider key's shape.
    assert_eq!(
        keys(&secrets.stdout),
        ["DATABASE_URL", "DB_PASSWORD", "STRIPE_KEY"]
    );

    let public = run(p.path(), &["convert", "--to", "json", "--only", "public"]);
    public.assert_code(0);
    assert_eq!(keys(&public.stdout), ["APP_NAME", "PORT"]);
}

#[test]
fn rm67_only_combines_with_include_and_names_itself_when_nothing_is_left() {
    let p = Project::new().env(ENV);
    let out = run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "--only",
            "secrets",
            "--include",
            "DB_*",
        ],
    );
    out.assert_code(0);
    assert_eq!(keys(&out.stdout), ["DB_PASSWORD"]);

    let none = run(
        p.path(),
        &[
            "convert",
            "--to",
            "json",
            "--only",
            "secrets",
            "--include",
            "APP_*",
        ],
    );
    none.assert_code(2);
    assert!(none.stderr.contains("--only secrets"), "{}", none.stderr);
}
