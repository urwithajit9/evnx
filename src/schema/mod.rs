//! Schema module: reusable infrastructure for all commands.

pub mod formatter;
pub mod loader;
pub mod models;
pub mod query;
pub mod resolver;

// Re-export commonly used types
pub use formatter::{format_addition, format_env_example, format_env_template, generate_preview};
pub use loader::{
    find_framework, find_service, get_frameworks_for_language, get_services_grouped,
    list_blueprints, schema,
};
pub use models::{Schema, VarCollection, VarMetadata, VarSource};
pub use query::{filter_by_tag, list_tags, search_frameworks, search_services};
pub use resolver::{
    resolve_architect_selection, resolve_blueprint, resolve_framework, resolve_service,
};
