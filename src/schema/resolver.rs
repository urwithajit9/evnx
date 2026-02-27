// src/schema/resolver.rs

use anyhow::Result;
// use std::collections::HashMap;

use super::loader::schema;
use super::models::{
    FrameworkConfig, InfrastructureConfig, ServiceConfig, StackBlueprint, VarCollection,
    VarMetadata, VarSource,
};

/// Resolve a blueprint into a collection of environment variables
pub fn resolve_blueprint(blueprint: &StackBlueprint) -> Result<VarCollection> {
    let schema = schema()?;
    let mut collection = VarCollection::default();

    // 1. Add framework variables
    if let Some(lang) = schema.languages.get(&blueprint.components.language) {
        if let Some(fw) = lang.frameworks.get(&blueprint.components.framework) {
            add_framework_vars(&mut collection, &blueprint.components.framework, fw);
        }
    }

    // 2. Add service variables
    for service_id in &blueprint.components.services {
        if let Some((_, service)) = super::loader::find_service(service_id) {
            add_service_vars(&mut collection, service_id, service);
        }
    }

    // 3. Add infrastructure variables
    for infra_id in &blueprint.components.infrastructure {
        if let Some(infra) = schema.infrastructure.get(infra_id) {
            add_infra_vars(&mut collection, infra_id, infra);
        }
    }

    Ok(collection)
}

/// Resolve variables from architect selections
pub fn resolve_architect_selection(
    language_id: &str,
    framework_id: &str,
    service_ids: &[String],
    infra_ids: &[String],
) -> Result<VarCollection> {
    let schema = schema()?;
    let mut collection = VarCollection::default();

    // Framework vars
    if let Some(lang) = schema.languages.get(language_id) {
        if let Some(fw) = lang.frameworks.get(framework_id) {
            add_framework_vars(&mut collection, framework_id, fw);
        }
    }

    // Service vars
    for service_id in service_ids {
        if let Some((_, service)) = super::loader::find_service(service_id) {
            add_service_vars(&mut collection, service_id, service);
        }
    }

    // Infrastructure vars
    for infra_id in infra_ids {
        if let Some(infra) = schema.infrastructure.get(infra_id) {
            add_infra_vars(&mut collection, infra_id, infra);
        }
    }

    Ok(collection)
}

/// NEW: Resolve variables for a single service
pub fn resolve_service(service_id: &str, service: &ServiceConfig) -> Result<VarCollection> {
    let mut collection = VarCollection::default();
    add_service_vars(&mut collection, service_id, service);
    Ok(collection)
}

/// NEW: Resolve variables for a single framework
pub fn resolve_framework(
    _language_id: &str,
    framework_id: &str,
    framework: &FrameworkConfig,
) -> Result<VarCollection> {
    let mut collection = VarCollection::default();
    add_framework_vars(&mut collection, framework_id, framework);
    Ok(collection)
}

/// Add framework variables to collection (with deduplication)
pub fn add_framework_vars(
    collection: &mut VarCollection,
    framework_id: &str,
    fw: &FrameworkConfig,
) {
    for var_name in &fw.vars {
        collection.vars.entry(var_name.clone()).or_insert_with(|| {
            VarMetadata {
                example_value: fw
                    .defaults
                    .get(var_name)
                    .cloned()
                    .unwrap_or_else(|| format!("your_{}_value", var_name.to_lowercase())),
                description: fw.descriptions.get(var_name).cloned(),
                category: fw.categories.get(var_name).cloned().or_else(|| {
                    // Infer category from var name patterns
                    infer_category(var_name)
                }),
                required: true,
                source: VarSource::Framework(framework_id.to_string()),
            }
        });
    }
}

/// Add service variables to collection
pub fn add_service_vars(collection: &mut VarCollection, service_id: &str, svc: &ServiceConfig) {
    let category = svc
        .category
        .clone()
        .or_else(|| infer_category_from_service(service_id));

    for var_name in &svc.vars {
        collection
            .vars
            .entry(var_name.clone())
            .or_insert_with(|| VarMetadata {
                example_value: svc
                    .defaults
                    .get(var_name)
                    .cloned()
                    .unwrap_or_else(|| format!("your_{}_value", var_name.to_lowercase())),
                description: svc.descriptions.get(var_name).cloned(),
                category: category.clone(),
                required: svc.required.contains(var_name),
                source: VarSource::Service(service_id.to_string()),
            });
    }
}

/// Add infrastructure variables
pub fn add_infra_vars(
    collection: &mut VarCollection,
    infra_id: &str,
    infra: &InfrastructureConfig,
) {
    let category = infra
        .category
        .clone()
        .or(Some("Infrastructure".to_string()));

    for var_name in &infra.vars {
        collection
            .vars
            .entry(var_name.clone())
            .or_insert_with(|| VarMetadata {
                example_value: infra
                    .defaults
                    .get(var_name)
                    .cloned()
                    .unwrap_or_else(|| format!("your_{}_value", var_name.to_lowercase())),
                description: infra.descriptions.get(var_name).cloned(),
                category: category.clone(),
                required: false,
                source: VarSource::Infrastructure(infra_id.to_string()),
            });
    }
}

/// Helper: Infer category from variable name patterns
fn infer_category(var_name: &str) -> Option<String> {
    let name_lower = var_name.to_lowercase();

    if name_lower.contains("secret") || name_lower.contains("key") || name_lower.contains("token") {
        Some("Security".to_string())
    } else if name_lower.contains("database")
        || name_lower.contains("db_")
        || name_lower.contains("postgres")
        || name_lower.contains("mysql")
    {
        Some("Database".to_string())
    } else if name_lower.contains("redis") {
        Some("Cache".to_string())
    } else if name_lower.contains("auth") || name_lower.contains("oauth") {
        Some("Auth".to_string())
    } else if name_lower.contains("url")
        || name_lower.contains("host")
        || name_lower.contains("port")
    {
        Some("Connection".to_string())
    } else if name_lower.contains("aws") || name_lower.contains("s3") {
        Some("Cloud".to_string())
    } else {
        Some("Application".to_string())
    }
}

/// Helper: Infer category from service ID
fn infer_category_from_service(service_id: &str) -> Option<String> {
    if service_id.contains("postgres")
        || service_id.contains("mysql")
        || service_id.contains("mongo")
    {
        Some("Database".to_string())
    } else if service_id.contains("redis") {
        Some("Cache".to_string())
    } else if service_id.contains("s3")
        || service_id.contains("storage")
        || service_id.contains("cloudinary")
    {
        Some("Storage".to_string())
    } else if service_id.contains("auth")
        || service_id.contains("oauth")
        || service_id.contains("clerk")
    {
        Some("Auth".to_string())
    } else if service_id.contains("stripe")
        || service_id.contains("payment")
        || service_id.contains("razorpay")
    {
        Some("Payments".to_string())
    } else if service_id.contains("sentry")
        || service_id.contains("datadog")
        || service_id.contains("log")
    {
        Some("Monitoring".to_string())
    } else if service_id.contains("openai")
        || service_id.contains("anthropic")
        || service_id.contains("ai")
    {
        Some("AI/ML".to_string())
    } else {
        Some("Service".to_string())
    }
}
