// tests/schema_integration.rs - Top of file

mod common;

use evnx::schema::{formatter, loader, resolver};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────
// Schema Consistency Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn test_blueprint_components_exist() {
    use evnx::schema::{loader, resolver};

    let schema = loader::schema().expect("Schema should load");

    for (stack_id, blueprint) in &schema.stacks {
        // Verify language exists
        assert!(
            schema
                .languages
                .contains_key(&blueprint.components.language),
            "Blueprint '{}' references unknown language: {}",
            stack_id,
            blueprint.components.language
        );

        // Verify framework exists
        let lang = &schema.languages[&blueprint.components.language];
        assert!(
            lang.frameworks
                .contains_key(&blueprint.components.framework),
            "Blueprint '{}' references unknown framework: {} for language {}",
            stack_id,
            blueprint.components.framework,
            blueprint.components.language
        );

        // Verify services exist
        for service_id in &blueprint.components.services {
            assert!(
                loader::find_service(service_id).is_some(),
                "Blueprint '{}' references unknown service: {}",
                stack_id,
                service_id
            );
        }

        // Verify infrastructure exists
        for infra_id in &blueprint.components.infrastructure {
            assert!(
                schema.infrastructure.contains_key(infra_id),
                "Blueprint '{}' references unknown infrastructure: {}",
                stack_id,
                infra_id
            );
        }

        // Test that blueprint actually resolves to variables
        let vars = resolver::resolve_blueprint(blueprint)
            .expect(&format!("Blueprint '{}' should resolve", stack_id));

        assert!(
            !vars.vars.is_empty(),
            "Blueprint '{}' should produce at least one variable",
            stack_id
        );
    }
}

#[test]
fn test_service_categories_are_consistent() {
    let schema = loader::schema().expect("Schema should load");

    // Collect all service categories
    let mut categories = HashMap::new();

    for (id, config) in &schema.services.databases {
        categories.insert(id.as_str(), config.category.as_deref());
    }
    for (id, config) in &schema.services.auth_providers {
        categories.insert(id.as_str(), config.category.as_deref());
    }
    // ... add other categories as needed

    // Verify no service has conflicting category assignments
    // (This is more of a schema validation check)
    for (id, cat) in &categories {
        assert!(
            cat.is_some() || id.contains("test"),
            "Service {} should have a category (or be a test service)",
            id
        );
    }
}

// ─────────────────────────────────────────────────────────────
// Resolution Round-Trip Tests
// ─────────────────────────────────────────────────────────────

#[test]
fn test_resolve_then_format_roundtrip() {
    let blueprint = loader::get_blueprint("supabase_fullstack").unwrap();
    let vars = resolver::resolve_blueprint(blueprint).expect("Should resolve");
    let formatted = formatter::format_env_example(&vars, true).expect("Should format");

    // FIX: Use common:: not crate::common::
    let parsed = common::parse_env_vars(&formatted);

    for var_name in vars.vars.keys() {
        assert!(
            parsed.contains_key(var_name),
            "Formatted output should contain variable: {}",
            var_name
        );
    }
}

#[test]
fn test_format_preserves_required_metadata() {
    let (_, service) = loader::find_service("stripe").unwrap();
    let vars = resolver::resolve_service("stripe", service).unwrap();

    let formatted = formatter::format_env_example(&vars, false).unwrap();

    // Required vars should have "(required)" marker
    for (name, meta) in &vars.vars {
        if meta.required {
            assert!(
                formatted.contains(name) && formatted.contains("(required)"),
                "Required var {} should have (required) marker",
                name
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Deduplication Edge Cases
// ─────────────────────────────────────────────────────────────

// tests/schema_integration.rs

#[test]
fn test_deduplication_framework_vs_service_priority() {
    // Current behavior: FIRST value added wins (regardless of source)
    let (_, django) = loader::find_framework("python", "django").unwrap();
    let (_, postgres) = loader::find_service("postgresql").unwrap();

    // Test 1: Framework added first → framework value wins
    let mut collection1 = evnx::schema::models::VarCollection::default();
    resolver::add_framework_vars(&mut collection1, "django", django);
    resolver::add_service_vars(&mut collection1, "postgresql", postgres);

    let framework_first_value = collection1.vars["DATABASE_URL"].example_value.clone();
    let framework_first_source = collection1.vars["DATABASE_URL"].source.clone();

    // Test 2: Service added first → service value wins
    let mut collection2 = evnx::schema::models::VarCollection::default();
    resolver::add_service_vars(&mut collection2, "postgresql", postgres);
    resolver::add_framework_vars(&mut collection2, "django", django);

    let service_first_value = collection2.vars["DATABASE_URL"].example_value.clone();
    let service_first_source = collection2.vars["DATABASE_URL"].source.clone();

    // Verify first-wins behavior
    assert_ne!(
        framework_first_value, service_first_value,
        "Values should differ based on add order (first-wins)"
    );

    assert_eq!(
        framework_first_source,
        evnx::schema::models::VarSource::Framework("django".to_string()),
        "When framework added first, source should be framework"
    );

    assert_eq!(
        service_first_source,
        evnx::schema::models::VarSource::Service("postgresql".to_string()),
        "When service added first, source should be service"
    );

    // Verify the variable still exists (deduplication worked - only one entry)
    assert_eq!(
        collection1.vars.len(),
        collection2.vars.len(),
        "Both collections should have same number of unique vars"
    );
}

#[test]
fn test_deduplication_same_service_twice() {
    let (_, redis) = loader::find_service("redis").unwrap();

    let mut collection = evnx::schema::models::VarCollection::default();

    // Add same service twice
    resolver::add_service_vars(&mut collection, "redis", redis);
    let first_add_count = collection.vars.len();

    resolver::add_service_vars(&mut collection, "redis", redis);
    let second_add_count = collection.vars.len();

    // Count should not increase (deduplication)
    assert_eq!(
        first_add_count, second_add_count,
        "Adding same service twice should not duplicate vars"
    );
}

// ─────────────────────────────────────────────────────────────
// Formatter Output Validation
// ─────────────────────────────────────────────────────────────

#[test]
fn test_formatter_output_is_valid_env_syntax() {
    let blueprint = loader::get_blueprint("fastapi_ai").unwrap();
    let vars = resolver::resolve_blueprint(blueprint).unwrap();

    let formatted = formatter::format_env_example(&vars, true).unwrap();

    // Each non-comment, non-empty line should be KEY=value
    for line in formatted.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Should contain exactly one '='
        let eq_count = trimmed.matches('=').count();
        assert_eq!(eq_count, 1, "Line should have exactly one '=': {}", trimmed);

        // Key should be non-empty and uppercase/snake_case
        let key = trimmed.split('=').next().unwrap();
        assert!(!key.is_empty(), "Key should not be empty");
        assert!(
            key.chars().all(|c| c.is_alphanumeric() || c == '_'),
            "Key should be alphanumeric with underscores: {}",
            key
        );
    }
}

#[test]
fn test_formatter_section_ordering() {
    // Create a VarCollection with multiple categories
    let mut vars = evnx::schema::models::VarCollection::default();

    // Add vars in random category order
    use evnx::schema::models::{VarMetadata, VarSource};

    vars.vars.insert(
        "Z_VAR".to_string(),
        VarMetadata {
            example_value: "z".to_string(),
            description: None,
            category: Some("Zebra".to_string()),
            required: false,
            source: VarSource::Service("test".to_string()),
        },
    );

    vars.vars.insert(
        "A_VAR".to_string(),
        VarMetadata {
            example_value: "a".to_string(),
            description: None,
            category: Some("Application".to_string()),
            required: false,
            source: VarSource::Framework("test".to_string()),
        },
    );

    vars.vars.insert(
        "D_VAR".to_string(),
        VarMetadata {
            example_value: "d".to_string(),
            description: None,
            category: Some("Database".to_string()),
            required: false,
            source: VarSource::Service("test".to_string()),
        },
    );

    let formatted = formatter::format_env_example(&vars, false).unwrap();

    // Find line numbers of section headers
    let lines: Vec<_> = formatted.lines().collect();

    let app_idx = lines
        .iter()
        .position(|l| l.contains("── Application ──"))
        .unwrap();
    let db_idx = lines
        .iter()
        .position(|l| l.contains("── Database ──"))
        .unwrap();
    let zebra_idx = lines
        .iter()
        .position(|l| l.contains("── Zebra ──"))
        .unwrap();

    // Application should come before Database, which should come before Zebra
    assert!(app_idx < db_idx, "Application should come before Database");
    assert!(
        db_idx < zebra_idx,
        "Database should come before Zebra (unknown category)"
    );
}
