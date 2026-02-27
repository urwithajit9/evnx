// tests/schema_unit.rs - Top of file

mod common;
//use evnx::schema::{loader, resolver, formatter, query};
use evnx::schema::query;
// use evnx::schema::models::{VarCollection, VarSource};

#[test]
fn test_search_services_finds_match() {
    // FIX: Use query:: not loader::
    let results = query::search_services("postgres");
    assert!(!results.is_empty(), "Should find postgresql");

    let ids: Vec<_> = results.iter().map(|(id, _, _)| id.as_str()).collect();
    assert!(ids.contains(&"postgresql"), "Should include postgresql");
}

#[test]
fn test_search_services_case_insensitive() {
    // FIX: Use query:: not loader::
    let lower = query::search_services("redis");
    let upper = query::search_services("REDIS");

    assert_eq!(
        lower.len(),
        upper.len(),
        "Search should be case-insensitive"
    );
}

#[test]
fn test_filter_by_tag() {
    // FIX: Use query:: not loader::
    let results = query::filter_by_tag("fullstack");
    assert!(!results.is_empty(), "Should have fullstack blueprints");

    let ids: Vec<_> = results.iter().map(|(id, _, _)| id.as_str()).collect();
    assert!(
        ids.contains(&"t3_modern") || ids.contains(&"mern_v2"),
        "Should include a fullstack blueprint"
    );
}

#[test]
fn test_list_tags() {
    // FIX: Use query:: not loader::
    let tags = query::list_tags();
    assert!(!tags.is_empty(), "Should have at least one tag");

    assert!(
        tags.iter().any(|t| t == "fullstack"),
        "Should have 'fullstack' tag"
    );
    assert!(
        tags.iter().any(|t| t == "typescript"),
        "Should have 'typescript' tag"
    );
}
