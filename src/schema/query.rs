// src/schema/query.rs

use crate::schema::loader;

/// Search services by keyword (case-insensitive)
pub fn search_services(query: &str) -> Vec<(String, String, String)> {
    let Ok(schema) = loader::schema() else {
        return vec![];
    };
    let query_lower = query.to_lowercase();

    let mut results = Vec::new();

    for (category_name, services) in [
        ("Database", &schema.services.databases),
        ("Cache", &schema.services.databases),
        ("Auth", &schema.services.auth_providers),
        ("Storage", &schema.services.storage),
        ("Monitoring", &schema.services.monitoring_logging),
        ("Payments", &schema.services.payments),
        ("AI/ML", &schema.services.ai_ml),
        ("Email/SMS", &schema.services.email_sms),
    ] {
        for (id, config) in services {
            let display = config.display_name.as_deref().unwrap_or(id).to_string();
            if id.to_lowercase().contains(&query_lower)
                || display.to_lowercase().contains(&query_lower)
                || config
                    .category
                    .as_ref()
                    .is_some_and(|c| c.to_lowercase().contains(&query_lower))
            {
                results.push((id.clone(), display, category_name.to_string()));
            }
        }
    }

    results.sort_by_key(|(_, display, _)| display.clone());
    results
}

/// Search frameworks by language and keyword
pub fn search_frameworks(language_id: &str, query: &str) -> Vec<(String, String)> {
    let Ok(schema) = loader::schema() else {
        return vec![];
    };
    let Some(lang) = schema.languages.get(language_id) else {
        return vec![];
    };

    let query_lower = query.to_lowercase();

    lang.frameworks
        .iter()
        .filter(|(id, fw)| {
            let display = fw.display_name.as_deref().unwrap_or(id);
            id.to_lowercase().contains(&query_lower)
                || display.to_lowercase().contains(&query_lower)
        })
        .map(|(id, fw)| {
            let display = fw.display_name.as_deref().unwrap_or(id).to_string();
            (id.clone(), display)
        })
        .collect()
}

/// Filter blueprints by tags
pub fn filter_by_tag(tag: &str) -> Vec<(String, String, String)> {
    let Ok(schema) = loader::schema() else {
        return vec![];
    };

    schema
        .stacks
        .iter()
        .filter(|(_, bp)| bp.tags.iter().any(|t| t == tag))
        .map(|(id, bp)| (id.clone(), bp.name.clone(), bp.description.clone()))
        .collect()
}

/// Get all unique tags from blueprints
pub fn list_tags() -> Vec<String> {
    let Ok(schema) = loader::schema() else {
        return vec![];
    };

    let mut tags: Vec<_> = schema
        .stacks
        .values()
        .flat_map(|bp| bp.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}
