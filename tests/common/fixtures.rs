// tests/common/fixtures.rs

//! Test fixtures for schema and CLI tests.

use crate::common::{write_env, write_env_example};
use std::path::Path;

/// Create a minimal project structure for testing
pub fn setup_minimal_project(dir: &Path) -> anyhow::Result<()> {
    write_env_example(dir, "# Minimal project\nAPP_NAME=test\n")?;
    write_env(dir, "# TODO: Fill in values\nAPP_NAME=\n")?;
    Ok(())
}

/// Create a project with existing PostgreSQL vars (for conflict testing)
pub fn setup_postgres_project(dir: &Path) -> anyhow::Result<()> {
    write_env_example(
        dir,
        r#"
# Database
DATABASE_URL=postgresql://user:pass@localhost/prod_db
DB_HOST=prod-db.example.com
DB_NAME=production
"#,
    )?;
    Ok(())
}

/// Expected output for Python/Django framework
pub const DJANGO_VARS: &[&str] = &[
    "SECRET_KEY",
    "DEBUG",
    "ALLOWED_HOSTS",
    "DJANGO_SETTINGS_MODULE",
];

/// Expected output for PostgreSQL service
pub const POSTGRES_VARS: &[&str] = &[
    "DB_HOST",
    "DB_PORT",
    "DB_USER",
    "DB_PASSWORD",
    "DB_NAME",
    "DATABASE_URL",
];

/// Expected output for T3 blueprint (subset)
pub const T3_BLUEPRINT_VARS: &[&str] = &[
    "NEXTAUTH_SECRET",
    "NEXTAUTH_URL",
    "CLERK_SECRET_KEY",
    "DB_HOST",
    "AWS_ACCESS_KEY_ID",
    "STRIPE_SECRET_KEY",
];
