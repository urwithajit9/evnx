//! `validate`'s placeholder check after the shared classifier (U3 part 2).
//!
//! The check now comes from `core::classify` at the Strict level, with the
//! variable's name (DEC-G11, DEC-G13): `API_KEY=dev` is a placeholder and
//! `ENVIRONMENT=development` is not; `*****` always is (DEC-G10). On 0.10.2,
//! neither `API_KEY=dev` nor `TOKEN=*****` was reported as a placeholder.

mod support;

use support::{Project, run};

fn placeholder_vars(stdout: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(stdout).unwrap();
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "placeholder_value")
        .map(|i| i["variable"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn t_dec_g11_the_name_decides_for_context_words() {
    let p = Project::new()
        .env("API_KEY=dev\nENVIRONMENT=development\nTOKEN=*****\nDB_PASSWORD=test_db_pass\n")
        .example("API_KEY=\nENVIRONMENT=\nTOKEN=\nDB_PASSWORD=\n");
    let out = run(p.path(), &["validate", "--format", "json"]);
    let mut vars = placeholder_vars(&out.stdout);
    vars.sort();
    assert_eq!(
        vars,
        ["API_KEY", "TOKEN"],
        "ENVIRONMENT=development is a real value; test_db_pass stays real for --fix (DEC-G13)"
    );
}

/// The quick-start run: five placeholders, and the next steps never named the
/// command that fills them.
#[test]
fn placeholders_point_to_set_fill() {
    let p = Project::new()
        .env("APP_NAME=YOUR_VALUE_HERE\nPORT=8080\n")
        .example("APP_NAME=\nPORT=\n");
    let out = run(p.path(), &["validate"]);
    out.assert_code(1);
    out.assert_contains("Fill in the placeholder values: evnx set --fill");

    let p = Project::new()
        .file(".env.staging", "APP_NAME=YOUR_VALUE_HERE\n")
        .example("APP_NAME=\n");
    let out = run(p.path(), &["validate", "--env", ".env.staging"]);
    out.assert_contains("Fill in the placeholder values: evnx set --fill --env .env.staging");
}

/// No placeholder, no `set --fill`.
#[test]
fn no_placeholder_no_set_fill() {
    let p = Project::new().env("PORT=8080\n").example("PORT=\nB=\n");
    let out = run(p.path(), &["validate"]);
    out.assert_not_contains("set --fill");
}
