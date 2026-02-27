//! evnx — Manage .env files with validation, secret scanning, and format conversion.
//!
//! # Library Usage
//!
//! ```rust
//! use evnx::schema::{loader, resolver, formatter};
//!
//! // Resolve variables for a service
//! let schema = loader::schema()?;
//! let pg = &schema.services.databases["postgresql"];
//! let vars = resolver::resolve_service("postgresql", pg)?;
//! let content = formatter::format_addition(&vars)?;
//! // Write `content` to .env.example
//! # Ok::<_, anyhow::Error>(())
//! ```
//!
//! # Architecture
//!
//! - `schema/` — Reusable core: JSON schema, resolver, formatter
//! - `commands/` — CLI handlers (init, add, validate, etc.)
//! - `utils/` — Shared utilities (file I/O, formatting)

// ─────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────

pub mod cli;
pub mod commands;
pub mod core;
pub mod formats;
pub mod schema;
pub mod utils;

pub use cli::{AddTarget, Cli, Commands};
pub use schema::{
    formatter::{format_addition, format_env_example, format_env_template, generate_preview},
    loader::{
        find_framework, find_service, get_frameworks_for_language, get_services_grouped,
        list_blueprints, schema as load_schema,
    },
    models::{Schema, VarCollection, VarMetadata, VarSource},
    query::{filter_by_tag, list_tags, search_frameworks, search_services},
    resolver::{
        resolve_architect_selection, resolve_blueprint, resolve_framework, resolve_service,
    },
};
