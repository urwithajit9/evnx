/// Template command - generate configuration files from .env
///
/// Supports variable substitution with filters and transformations
use anyhow::{Context, Result};
use colored::*;
use std::collections::HashMap;
use std::fs;

use crate::core::Parser;

pub fn run(input: String, output: String, env: String, verbose: bool) -> Result<()> {
    if verbose {
        println!("{}", "Running template in verbose mode".dimmed());
    }

    println!(
        "\n{}",
        "┌─ Generate config from template ─────────────────────┐".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    // Parse .env file
    let parser = Parser::default();
    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse {}", env))?;

    println!(
        "{} Loaded {} variables from {}",
        "✓".green(),
        env_file.vars.len(),
        env
    );

    // Read template
    let template_content =
        fs::read_to_string(&input).with_context(|| format!("Failed to read template {}", input))?;

    println!("{} Read template from {}", "✓".green(), input);

    // Process template
    let result = process_template(&template_content, &env_file.vars)?;

    // Write output
    fs::write(&output, &result).with_context(|| format!("Failed to write to {}", output))?;

    println!("{} Generated config at {}", "✓".green(), output);

    Ok(())
}

/// Process template with variable substitution
fn process_template(template: &str, vars: &HashMap<String, String>) -> Result<String> {
    let mut result = template.to_string();

    // Simple variable substitution: ${VAR} or {{VAR}}
    for (key, value) in vars {
        // ${VAR} style
        let pattern = format!("${{{}}}", key);
        result = result.replace(&pattern, value);

        // {{VAR}} style
        let pattern = format!("{{{{{}}}}}", key);
        result = result.replace(&pattern, value);

        // $VAR style (simple)
        let pattern = format!("${}", key);
        result = result.replace(&pattern, value);
    }

    // Process filters
    result = process_filters(&result, vars)?;

    Ok(result)
}

type FilterFn = fn(&str) -> String;

fn filter_patterns() -> Vec<(&'static str, FilterFn)> {
    vec![
        ("|upper", |v| v.to_uppercase()),
        ("|lower", |v| v.to_lowercase()),
        ("|title", |v| {
            let mut c = v.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }),
    ]
}

/// Process template filters
/// Supports: {{VAR|upper}}, {{VAR|lower}}, {{VAR|bool}}, {{VAR|json}}
fn process_filters(template: &str, vars: &HashMap<String, String>) -> Result<String> {
    let mut result = template.to_string();

    for (key, value) in vars {
        // Apply upper/lower/title via filter_patterns()
        // BUG FIX: was `&filter_patterns` (variable) — must be `&filter_patterns()` (function call)
        for (filter, transform) in &filter_patterns() {
            let pattern = format!("{{{{{}{}}}}}", key, filter);
            if result.contains(&pattern) {
                let transformed = transform(value);
                result = result.replace(&pattern, &transformed);
            }
        }

        // Boolean filter
        let pattern = format!("{{{{{}|bool}}}}", key);
        if result.contains(&pattern) {
            let bool_val = value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("1");
            result = result.replace(&pattern, &bool_val.to_string());
        }

        // Integer filter
        let pattern = format!("{{{{{}|int}}}}", key);
        if result.contains(&pattern) {
            let int_val = value.parse::<i64>().unwrap_or(0);
            result = result.replace(&pattern, &int_val.to_string());
        }

        // JSON escape filter
        let pattern = format!("{{{{{}|json}}}}", key);
        if result.contains(&pattern) {
            let json_val = serde_json::to_string(value).unwrap_or_default();
            // Remove surrounding quotes
            let json_val = json_val.trim_matches('"');
            result = result.replace(&pattern, json_val);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_template_simple() {
        let template = "DATABASE_URL=${DATABASE_URL}";
        let mut vars = HashMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "DATABASE_URL=postgresql://localhost");
    }

    #[test]
    fn test_process_template_double_braces() {
        let template = "url: {{DATABASE_URL}}";
        let mut vars = HashMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "url: postgresql://localhost");
    }

    #[test]
    fn test_filter_upper() {
        let template = "ENV={{ENV|upper}}";
        let mut vars = HashMap::new();
        vars.insert("ENV".to_string(), "production".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "ENV=PRODUCTION");
    }

    #[test]
    fn test_filter_bool() {
        let template = "debug={{DEBUG|bool}}";
        let mut vars = HashMap::new();
        vars.insert("DEBUG".to_string(), "true".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "debug=true");
    }

    #[test]
    fn test_filter_int() {
        let template = "port={{PORT|int}}";
        let mut vars = HashMap::new();
        vars.insert("PORT".to_string(), "8000".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "port=8000");
    }
}
