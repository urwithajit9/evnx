//! tests/migrate_integration.rs — Integration tests for the migrate command
//!
//! Run with: `cargo test --features migrate --test migrate_integration`

use std::fs;
use std::io::Write;
use tempfile::NamedTempFile;

use evnx::commands::migrate::{
    destination::{MigrationDestination, MigrationOptions},
    filtering::apply_filters,
    sources::load_secrets,
};

// ─── Helper ───────────────────────────────────────────────────────────────────

fn write_env_file(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(contents.as_bytes()).expect("write");
    f
}

// ─── sources ─────────────────────────────────────────────────────────────────

#[test]
fn test_load_env_file() {
    let f = write_env_file("DB_URL=postgres://localhost/test\nSECRET=abc123\n");
    let secrets = load_secrets("env-file", f.path().to_str().unwrap(), false).unwrap();
    assert_eq!(
        secrets.get("DB_URL").map(String::as_str),
        Some("postgres://localhost/test")
    );
    assert_eq!(secrets.get("SECRET").map(String::as_str), Some("abc123"));
}

#[test]
fn test_load_unsupported_source_errors() {
    let result = load_secrets("consul", "", false);
    assert!(result.is_err());
}

// ─── filtering ───────────────────────────────────────────────────────────────

#[test]
fn test_filter_include_and_strip() {
    use indexmap::IndexMap;
    let mut s = IndexMap::new();
    s.insert("APP_DB_URL".into(), "pg://".into());
    s.insert("APP_SECRET".into(), "xyz".into());
    s.insert("UNRELATED".into(), "val".into());

    let result = apply_filters(&s, Some(&["APP_*".into()]), None, Some("APP_"), None);
    assert_eq!(result.len(), 2);
    assert!(result.contains_key("DB_URL"));
    assert!(result.contains_key("SECRET"));
    assert!(!result.contains_key("UNRELATED"));
}

// ─── AWS dry-run ──────────────────────────────────────────────────────────────

#[test]
fn test_aws_dry_run_no_side_effects() {
    use evnx::commands::migrate::destinations::aws::AwsDestination;
    use indexmap::IndexMap;

    let dest = AwsDestination {
        secret_name: "prod/test".into(),
        profile: None,
    };

    let mut secrets = IndexMap::new();
    secrets.insert("KEY".into(), "value".into());

    let opts = MigrationOptions {
        dry_run: true,
        ..Default::default()
    };
    let result = dest.migrate(&secrets, &opts).unwrap();

    assert_eq!(result.uploaded, 0);
    assert_eq!(result.skipped, 1);
    assert_eq!(result.failed, 0);
}

// ─── Azure key rename ─────────────────────────────────────────────────────────

#[test]
fn test_azure_underscore_to_hyphen() {
    let key = "DB_URL_PRIMARY".to_string();
    let akv = key.replace('_', "-");
    assert_eq!(akv, "DB-URL-PRIMARY");
}

// ─── Heroku dry-run ───────────────────────────────────────────────────────────

#[test]
fn test_heroku_dry_run() {
    use evnx::commands::migrate::destinations::heroku::HerokuDestination;
    use indexmap::IndexMap;

    let dest = HerokuDestination {
        app: "my-app".into(),
    };
    let mut secrets = IndexMap::new();
    secrets.insert("PORT".into(), "3000".into());

    let opts = MigrationOptions {
        dry_run: true,
        ..Default::default()
    };
    let result = dest.migrate(&secrets, &opts).unwrap();
    assert_eq!(result.skipped, 1);
}
