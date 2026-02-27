// src/schema/loader.rs

use anyhow::Result;
use std::sync::OnceLock;

use super::models::{FrameworkConfig, Schema, ServiceConfig, StackBlueprint};

/// Embedded schema JSON (compiled into binary)
/// Path: src/schema/loader.rs → ../assets/schema.json
const SCHEMA_JSON: &str = include_str!("../assets/schema.json");

/// Global singleton for loaded schema
static _SCHEMA: OnceLock<Schema> = OnceLock::new();

/// Get the global schema instance, loading if necessary
/// Global singleton for loaded schema (stable Rust compatible)
pub fn schema() -> Result<&'static Schema> {
    use std::sync::OnceLock;

    static SCHEMA: OnceLock<Schema> = OnceLock::new();

    SCHEMA.get_or_init(|| {
        serde_json::from_str(SCHEMA_JSON)
            .expect("schema.json must be valid JSON - check assets/schema.json")
    });

    SCHEMA
        .get()
        .ok_or_else(|| anyhow::anyhow!("Schema failed to initialize"))
}

/// Fallback for stable Rust (if get_or_try_init is unstable)
// #[cfg(not(feature = "unstable_once_cell"))]
pub fn schema_fallback() -> Result<&'static Schema> {
    static FALLBACK: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    FALLBACK
        .get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("schema.json must be valid JSON"));
    FALLBACK
        .get()
        .ok_or_else(|| anyhow::anyhow!("Schema not initialized"))
}

/// Find a service by ID across all categories
pub fn find_service(service_id: &str) -> Option<(&str, &ServiceConfig)> {
    let schema = schema().ok()?;

    // Search all service categories
    schema
        .services
        .databases
        .get(service_id)
        .or_else(|| schema.services.messaging_queues.get(service_id))
        .or_else(|| schema.services.auth_providers.get(service_id))
        .or_else(|| schema.services.storage.get(service_id))
        .or_else(|| schema.services.monitoring_logging.get(service_id))
        .or_else(|| schema.services.payments.get(service_id))
        .or_else(|| schema.services.ai_ml.get(service_id))
        .or_else(|| schema.services.email_sms.get(service_id))
        .map(|svc| (service_id, svc))
}

/// Find a framework by language_id and framework_id
pub fn find_framework<'a>(
    language_id: &'a str,
    framework_id: &'a str,
) -> Option<(&'a str, &'a FrameworkConfig)> {
    let schema = schema().ok()?;
    let lang = schema.languages.get(language_id)?;
    let fw = lang.frameworks.get(framework_id)?;
    Some((framework_id, fw))
}

/// Get blueprint by ID
pub fn get_blueprint(id: &str) -> Option<&'static StackBlueprint> {
    schema().ok()?.stacks.get(id)
}

/// List all blueprints as (id, name) pairs
pub fn list_blueprints() -> Vec<(&'static str, &'static str)> {
    let Ok(schema) = schema() else { return vec![] };

    schema
        .stacks
        .iter()
        .map(|(id, bp)| (id.as_str(), bp.name.as_str()))
        .collect()
}

/// Get frameworks for a language as (id, display_name) pairs
pub fn get_frameworks_for_language(language_id: &str) -> Option<Vec<(&'static str, String)>> {
    let schema = schema().ok()?;
    let lang = schema.languages.get(language_id)?;

    Some(
        lang.frameworks
            .iter()
            .map(|(id, fw)| {
                let display = fw.display_name.as_deref().unwrap_or(id).to_string();
                (id.as_str(), display)
            })
            .collect(),
    )
}

/// Get all services grouped by category for MultiSelect
pub fn get_services_grouped() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let Ok(schema) = schema() else { return vec![] };

    vec![
        (
            "Databases",
            extract_service_names(&schema.services.databases),
        ),
        (
            "Messaging",
            extract_service_names(&schema.services.messaging_queues),
        ),
        (
            "Auth",
            extract_service_names(&schema.services.auth_providers),
        ),
        ("Storage", extract_service_names(&schema.services.storage)),
        (
            "Monitoring",
            extract_service_names(&schema.services.monitoring_logging),
        ),
        ("Payments", extract_service_names(&schema.services.payments)),
        ("AI/ML", extract_service_names(&schema.services.ai_ml)),
        (
            "Email/SMS",
            extract_service_names(&schema.services.email_sms),
        ),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .collect()
}

fn extract_service_names(
    services: &std::collections::HashMap<String, ServiceConfig>,
) -> Vec<(&str, String)> {
    services
        .iter()
        .map(|(id, cfg)| {
            let display = cfg.display_name.as_deref().unwrap_or(id).to_string();
            (id.as_str(), display)
        })
        .collect()
}
