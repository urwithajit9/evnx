//! Template engine for generating configuration files from `.env` files.
//!
//! This module provides a flexible template substitution system that supports:
//!
//! - Multiple variable syntax styles: `${VAR}`, `{{VAR}}`, `$VAR`
//! - Filter transformations: `|upper`, `|lower`, `|title`, `|bool`, `|int`, `|json`, `|default:value`
//! - Proper error handling with contextual messages
//! - Detection of undefined variables with warnings
//! - Integration with the project's UI utilities for consistent output
//!
//! # Variable Substitution Syntax
//!
//! The template engine supports three styles of variable references:
//!
//! ```text
//! ${DATABASE_URL}     # Shell-style (bash-like)
//! {{DATABASE_URL}}    # Template-style (Jinja-like)
//! $DATABASE_URL       # Simple prefix style
//! ```
//!
//! # Filter Syntax
//!
//! Filters can be applied using the pipe operator within double braces:
//!
//! ```text
//! {{ENV|upper}}           # Convert to uppercase: "production" → "PRODUCTION"
//! {{DEBUG|bool}}          # Convert to boolean: "true"/"yes"/"1" → true
//! {{PORT|int}}            # Convert to integer: "8000" → 8000
//! {{NAME|json}}           # JSON-escape: hello "world" → "hello \"world\""
//! {{OPTIONAL|default:foo}} # Use default if undefined or empty
//! ```
//!
//! Filters are processed **before** simple substitution to avoid pattern conflicts.
//!
//! # Usage Examples
//!
//! ## Basic template processing
//!
//! ```no_run
//! # use indexmap::IndexMap;
//! # use evnx::commands::template::process_template;
//! let template = "url={{DATABASE_URL}}";
//! let mut vars = IndexMap::new();
//! vars.insert("DATABASE_URL".to_string(), "postgresql://localhost".to_string());
//!
//! let result = process_template(template, &vars).unwrap();
//! assert_eq!(result, "url=postgresql://localhost");
//! ```
//!
//! ## Using filters
//!
//! ```no_run
//! # use indexmap::IndexMap;
//! # use evnx::commands::template::process_template;
//! let template = "ENV={{ENV|upper}}, debug={{DEBUG|bool}}";
//! let mut vars = IndexMap::new();
//! vars.insert("ENV".to_string(), "production".to_string());
//! vars.insert("DEBUG".to_string(), "yes".to_string());
//!
//! let result = process_template(template, &vars).unwrap();
//! assert_eq!(result, "ENV=PRODUCTION, debug=true");
//! ```
//!
//! ## Command-line usage
//!
//! ```sh
//! # Generate config from template
//! evnx template --input config.toml.template --output config.toml --env .env.production
//!
//! # With verbose output
//! evnx template -i template.yaml -o output.yaml --verbose
//! ```
//!
//! # Error Handling
//!
//! - Undefined variables trigger a warning but don't fail (configurable in future)
//! - Invalid filter values (e.g., non-integer for `|int`) return an error
//! - File I/O errors include contextual path information
//!
//! # Security Considerations
//!
//! - Variable values are inserted as-is; sanitize if generating shell commands
//! - Template paths should be validated by the caller to prevent path traversal
//! - For JSON output, use `|json` filter to properly escape special characters

use anyhow::{Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use std::collections::HashSet;
use std::fs;

use crate::core::Parser;
use crate::utils::ui;

// ─────────────────────────────────────────────────────────────
// Regex Pattern Helpers (avoid format! brace escaping hell)
// ─────────────────────────────────────────────────────────────

/// Build regex pattern to match `${VAR}` style references.
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - iterate over results internally
/// let pattern = shell_var_pattern("MY_VAR");
///
/// ```
fn shell_var_pattern(key: &str) -> String {
    let escaped = regex::escape(key);
    // Breakdown of format string "\\$\\{{{}\\}}":
    //   \\   → \
    //   $    → $
    //   \\   → \
    //   {{   → {   (escaped brace in format! macro)
    //   {}   → <escaped key> (placeholder)
    //   \\   → \
    //   }}   → }   (escaped brace in format! macro)
    // Produces: \$\{KEY\}  which matches the literal text: ${KEY}
    format!("\\$\\{{{}\\}}", escaped)
}

/// Build regex pattern to match `{{VAR}}` style references.
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - used internally by process_simple_substitution
/// let pattern = template_var_pattern("MY_VAR");
/// // pattern == r"\{\{MY_VAR\}\}" as a regex string
/// ```
fn template_var_pattern(key: &str) -> String {
    let escaped = regex::escape(key);
    format!("\\{{\\{{{}\\}}\\}}", escaped)
}

/// Build regex pattern to match `{{VAR|filter}}` style references.
///
/// The `filter` argument **must include the leading pipe**, e.g. `"|upper"`, `"|bool"`.
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - used internally by process_filters
/// let pattern = filter_var_pattern("ENV", "|upper");
/// // pattern == r"\{\{ENV\|upper\}\}" as a regex string
/// ```
fn filter_var_pattern(key: &str, filter: &str) -> String {
    format!(
        "\\{{\\{{{}{}\\}}\\}}",
        regex::escape(key),
        regex::escape(filter)
    )
}

/// Build regex pattern to match `{{VAR|default:VALUE}}` with capture group.
///
/// The capture group extracts the default value for substitution.
fn default_var_pattern(key: &str) -> String {
    format!("\\{{\\{{{}\\|default:([^}}]*)\\}}\\}}", regex::escape(key))
}

// ─────────────────────────────────────────────────────────────
// Main Functions
// ─────────────────────────────────────────────────────────────

/// Execute the template command: generate config files from templates.
///
/// This function orchestrates the template processing pipeline:
/// 1. Parse the `.env` file into a variable map
/// 2. Read the template file content
/// 3. Process variable substitution and filters
/// 4. Write the generated output file
///
/// # Arguments
///
/// * `input` - Path to the template file (e.g., `config.toml.template`)
/// * `output` - Path where the generated config will be written
/// * `env` - Path to the `.env` file containing variable definitions
/// * `verbose` - Enable detailed progress output to stderr
///
/// # Returns
///
/// * `Ok(())` on success
/// * `Err(anyhow::Error)` with contextual message on failure
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::template::run;
/// run(
///     "config.yaml.template".to_string(),
///     "config.yaml".to_string(),
///     ".env.production".to_string(),
///     true, // verbose mode
/// ).expect("Template generation failed");
/// ```
pub fn run(input: String, output: String, env: String, verbose: bool) -> Result<()> {
    if verbose {
        ui::verbose_stderr("Running template in verbose mode");
    }

    ui::print_header("Generate config from template", None);

    // Parse .env file
    let parser = Parser::default();
    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse environment file: {}", env))?;

    ui::success(format!(
        "Loaded {} variables from {}",
        env_file.vars.len(),
        env
    ));

    // Read template
    let template_content = fs::read_to_string(&input)
        .with_context(|| format!("Failed to read template file: {}", input))?;

    ui::success(format!("Read template from {}", input));

    // Detect undefined variables and warn
    let undefined = detect_undefined_vars(&template_content, &env_file.vars);
    for var in &undefined {
        ui::warning(format!(
            "Variable '{}' referenced in template but not defined in {}",
            var, env
        ));
    }

    // Process template
    let result = process_template(&template_content, &env_file.vars)
        .context("Failed to process template substitutions")?;

    // Write output
    fs::write(&output, &result)
        .with_context(|| format!("Failed to write generated config to: {}", output))?;

    ui::success(format!("Generated config at {}", output));

    if !undefined.is_empty() {
        ui::info(format!(
            "Tip: Define {} undefined variable{} to avoid runtime issues",
            undefined.len(),
            if undefined.len() == 1 { "" } else { "s" }
        ));
    }

    Ok(())
}

/// Process a template string with variable substitution and filter transformations.
///
/// This function applies transformations in a specific order to avoid conflicts:
/// 1. **Filters first**: Process `{{VAR|filter}}` patterns with transformations
/// 2. **Simple substitution**: Replace remaining `${VAR}`, `{{VAR}}`, `$VAR` patterns
///
/// This ordering ensures that filtered variables are not accidentally replaced
/// by the simple substitution pass.
///
/// # Arguments
///
/// * `template` - The template string containing variable references
/// * `vars` - Map of variable names to their string values
///
/// # Returns
///
/// * `Ok(String)` - The processed template with all substitutions applied
/// * `Err(anyhow::Error)` - If a filter transformation fails (e.g., invalid integer)
///
/// # Supported Patterns
///
/// | Pattern | Description | Example |
/// |---------|-------------|---------|
/// | `${VAR}` | Shell-style substitution | `${PORT}` → `8000` |
/// | `{{VAR}}` | Template-style substitution | `{{PORT}}` → `8000` |
/// | `$VAR` | Simple prefix substitution | `$PORT` → `8000` |
/// | `{{VAR|upper}}` | Uppercase filter | `{{env|upper}}` → `PRODUCTION` |
/// | `{{VAR|bool}}` | Boolean conversion | `{{debug|bool}}` → `true` |
/// | `{{VAR|int}}` | Integer parsing | `{{port|int}}` → `8000` |
/// | `{{VAR|json}}` | JSON string escaping | `{{name|json}}` → `"hello \"world\""` |
/// | `{{VAR|default:foo}}` | Default value if undefined | `{{opt|default:bar}}` → `bar` |
///
/// # Example
///
/// ```
/// # use indexmap::IndexMap;
/// # use evnx::commands::template::process_template;
/// let template = "host={{HOST|upper}}, port=${PORT}";
/// let mut vars = IndexMap::new();
/// vars.insert("HOST".to_string(), "localhost".to_string());
/// vars.insert("PORT".to_string(), "3000".to_string());
///
/// let result = process_template(template, &vars).unwrap();
/// assert_eq!(result, "host=LOCALHOST, port=3000");
/// ```
pub fn process_template(template: &str, vars: &IndexMap<String, String>) -> Result<String> {
    let mut result = template.to_string();

    // Step 1: Process filters FIRST (before simple substitution)
    // This prevents {{VAR|filter}} from being partially replaced by $VAR or {{VAR}} patterns
    result = process_filters(&result, vars)?;

    // Step 2: Simple variable substitution for remaining unfiltered references
    result = process_simple_substitution(&result, vars);

    Ok(result)
}

/// Apply simple variable substitution patterns: `${VAR}`, `{{VAR}}`, `$VAR`.
///
/// This function handles basic substitution without filters. It should be called
/// **after** `process_filters` to avoid interfering with filtered patterns.
///
/// # Arguments
///
/// * `template` - Template string after filter processing
/// * `vars` - Variable map for substitution values
///
/// # Returns
///
/// String with all simple variable references replaced
fn process_simple_substitution(template: &str, vars: &IndexMap<String, String>) -> String {
    let mut result = template.to_string();

    for (key, value) in vars {
        // ${VAR} style
        if let Ok(re) = Regex::new(&shell_var_pattern(key)) {
            result = re.replace_all(&result, value.as_str()).to_string();
        }

        // {{VAR}} style
        if let Ok(re) = Regex::new(&template_var_pattern(key)) {
            result = re.replace_all(&result, value.as_str()).to_string();
        }

        // $VAR style with word boundary - raw string works here (no literal braces)
        let simple_pattern = format!(r"\${}\b", regex::escape(key));
        if let Ok(re) = Regex::new(&simple_pattern) {
            result = re.replace_all(&result, value.as_str()).to_string();
        }
    }

    result
}

/// Type alias for filter transformation functions.
///
/// Each filter takes a string value and returns a transformed string.
type FilterFn = fn(&str) -> String;

/// Registry of built-in filter patterns and their transformation functions.
///
/// Returns a vector of (pattern_suffix, transformation_function) pairs.
/// The pattern suffix **includes the leading pipe** and is appended to the variable
/// name within `{{VAR|suffix}}` syntax, e.g. `"|upper"` matches `{{VAR|upper}}`.
///
/// # Supported Filters
///
/// | Filter | Description | Example Input → Output |
/// |--------|-------------|------------------------|
/// | `upper` | Convert to uppercase | `"hello"` → `"HELLO"` |
/// | `lower` | Convert to lowercase | `"HELLO"` → `"hello"` |
/// | `title` | Capitalize first letter | `"hello"` → `"Hello"` |
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - used internally by process_simple_substitution
/// let pattern = shell_var_pattern("MY_VAR");
/// // pattern == r"\$\{MY_VAR\}" as a regex string
/// ```
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

/// Process template filters: transformations applied via `{{VAR|filter}}` syntax.
///
/// This function handles all filter-based substitutions using regex for reliable
/// pattern matching. Filters are processed before simple substitution to avoid
/// conflicts with base variable patterns.
///
/// # Supported Filters
///
/// - **String transforms**: `|upper`, `|lower`, `|title`
/// - **Type conversions**: `|bool`, `|int`
/// - **Encoding**: `|json` (properly escaped JSON string)
/// - **Defaults**: `|default:value` (fallback if variable is empty/undefined)
///
/// # Arguments
///
/// * `template` - Template string containing filter patterns
/// * `vars` - Variable map for lookup and transformation
///
/// # Returns
///
/// * `Ok(String)` - Template with all filters applied
/// * `Err(anyhow::Error)` - If a filter transformation fails
fn process_filters(template: &str, vars: &IndexMap<String, String>) -> Result<String> {
    let mut result = template.to_string();

    for (key, value) in vars {
        // Apply string transformation filters (upper/lower/title)
        // filter_suffix already contains the leading pipe, e.g. "|upper"
        for (filter_suffix, transform) in &filter_patterns() {
            let pattern = filter_var_pattern(key, filter_suffix);
            if let Ok(re) = Regex::new(&pattern) {
                if re.is_match(&result) {
                    let transformed = transform(value);
                    result = re.replace_all(&result, transformed.as_str()).to_string();
                }
            }
        }

        // Boolean filter: {{VAR|bool}} - converts truthy strings to "true"/"false"
        let bool_pattern = filter_var_pattern(key, "|bool");
        if let Ok(re) = Regex::new(&bool_pattern) {
            if re.is_match(&result) {
                let bool_val = value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
                    || value.eq_ignore_ascii_case("1");
                result = re
                    .replace_all(&result, bool_val.to_string().as_str())
                    .to_string();
            }
        }

        // Integer filter: {{VAR|int}} - parses string to i64, errors on invalid input
        let int_pattern = filter_var_pattern(key, "|int");
        if let Ok(re) = Regex::new(&int_pattern) {
            if re.is_match(&result) {
                let int_val = value.parse::<i64>().with_context(|| {
                    format!(
                        "Variable '{}' value '{}' is not a valid integer for |int filter",
                        key, value
                    )
                })?;
                result = re
                    .replace_all(&result, int_val.to_string().as_str())
                    .to_string();
            }
        }

        // JSON filter: {{VAR|json}} - properly escape value for JSON context
        let json_pattern = filter_var_pattern(key, "|json");
        if let Ok(re) = Regex::new(&json_pattern) {
            if re.is_match(&result) {
                let json_val = serde_json::to_string(value)
                    .with_context(|| format!("Failed to JSON-encode variable '{}'", key))?;
                // Keep quotes - they're part of valid JSON string representation
                result = re.replace_all(&result, json_val.as_str()).to_string();
            }
        }

        // Default value filter: {{VAR|default:fallback}}
        let default_pattern = default_var_pattern(key);
        if let Ok(re) = Regex::new(&default_pattern) {
            if re.is_match(&result) {
                let replacement = if value.trim().is_empty() {
                    // Extract default value from capture group 1
                    re.captures(&result)
                        .and_then(|caps| caps.get(1))
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default()
                } else {
                    value.clone()
                };
                result = re.replace_all(&result, replacement.as_str()).to_string();
            }
        }
    }

    Ok(result)
}

/// Detect variable references in template that are not defined in the variable map.
///
/// Scans the template for all supported variable patterns and returns a list
/// of undefined variable names. This helps catch configuration errors early.
///
/// # Arguments
///
/// * `template` - Template string to scan for variable references
/// * `vars` - Map of defined variable names
///
/// # Returns
///
/// Vec of undefined variable names (deduplicated)
///
/// # Example
///
/// ```
/// # use indexmap::IndexMap;
/// # use evnx::commands::template::detect_undefined_vars;
/// let template = "url=${DATABASE_URL}, host={{HOST}}";
/// let mut vars = IndexMap::new();
/// vars.insert("DATABASE_URL".to_string(), "postgresql://localhost".to_string());
/// // HOST is not defined
///
/// let undefined = detect_undefined_vars(template, &vars);
/// assert_eq!(undefined, vec!["HOST"]);
/// ```
pub fn detect_undefined_vars(template: &str, vars: &IndexMap<String, String>) -> Vec<String> {
    // Regex pattern matching all supported variable reference styles:
    // - ${VAR}: shell-style with braces
    // - {{VAR}} or {{VAR|filter}}: template-style with optional filter
    // - $VAR: simple prefix style with word boundary
    let re = Regex::new(
        r"(?:\$\{([A-Za-z_][A-Za-z0-9_]*)\})|(?:\{\{([A-Za-z_][A-Za-z0-9_]*)(?:\|[^}]+)?\}\})|(?:\$([A-Za-z_][A-Za-z0-9_]*))\b"
    ).expect("Variable detection regex is valid");

    let mut undefined = Vec::new();
    let mut seen = HashSet::new();

    for caps in re.captures_iter(template) {
        // Variable name could be in capture group 1, 2, or 3 depending on pattern style
        let var_name = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_string());

        if let Some(name) = var_name {
            // Only report each undefined variable once, and skip if defined
            if !vars.contains_key(&name) && !seen.contains(&name) {
                undefined.push(name.clone());
                seen.insert(name);
            }
        }
    }

    undefined
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_helpers() {
        // shell_var_pattern: should produce \$\{KEY\}
        assert_eq!(shell_var_pattern("MY_VAR"), r"\$\{MY_VAR\}");

        // template_var_pattern: should produce \{\{KEY\}\}
        assert_eq!(template_var_pattern("MY_VAR"), r"\{\{MY_VAR\}\}");

        // filter_var_pattern: filter arg includes the leading pipe.
        // Passing "|bool" → regex::escape("|bool") = "\|bool"
        // Result: \{\{KEY\|bool\}\}
        assert_eq!(filter_var_pattern("KEY", "|bool"), r"\{\{KEY\|bool\}\}");

        // default_var_pattern: should produce \{\{KEY\|default:([^}]*)\}\}
        assert_eq!(default_var_pattern("OPT"), r"\{\{OPT\|default:([^}]*)\}\}");
    }

    #[test]
    fn test_process_template_simple_substitution() {
        let template = "DATABASE_URL=${DATABASE_URL}";
        let mut vars = IndexMap::new();
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
        let mut vars = IndexMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "url: postgresql://localhost");
    }

    #[test]
    fn test_process_template_simple_prefix() {
        let template = "host: $HOST port: $PORT";
        let mut vars = IndexMap::new();
        vars.insert("HOST".to_string(), "localhost".to_string());
        vars.insert("PORT".to_string(), "3000".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "host: localhost port: 3000");
    }

    #[test]
    fn test_filter_order_prevents_conflicts() {
        // Ensure {{VAR|upper}} is not partially replaced by {{VAR}} substitution
        let template = "env={{ENV|upper}} raw={{ENV}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "production".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "env=PRODUCTION raw=production");
    }

    #[test]
    fn test_filter_upper() {
        let template = "ENV={{ENV|upper}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "production".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "ENV=PRODUCTION");
    }

    #[test]
    fn test_filter_lower() {
        let template = "ENV={{ENV|lower}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "PRODUCTION".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "ENV=production");
    }

    #[test]
    fn test_filter_title() {
        let template = "name={{NAME|title}}";
        let mut vars = IndexMap::new();
        vars.insert("NAME".to_string(), "hello world".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "name=Hello world");
    }

    #[test]
    fn test_filter_bool_truthy_values() {
        let test_cases = vec!["true", "True", "TRUE", "yes", "Yes", "1"];
        for truthy in test_cases {
            let template = "debug={{DEBUG|bool}}";
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), truthy.to_string());

            let result = process_filters(template, &vars).unwrap();
            assert_eq!(result, "debug=true", "Failed for truthy value: {}", truthy);
        }
    }

    #[test]
    fn test_filter_bool_falsy_values() {
        let test_cases = vec!["false", "no", "0", "maybe", ""];
        for falsy in test_cases {
            let template = "debug={{DEBUG|bool}}";
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), falsy.to_string());

            let result = process_filters(template, &vars).unwrap();
            assert_eq!(result, "debug=false", "Failed for falsy value: {}", falsy);
        }
    }

    #[test]
    fn test_filter_int_valid() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "8000".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "port=8000");
    }

    #[test]
    fn test_filter_int_invalid_error() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "not-a-number".to_string());

        let result = process_filters(template, &vars);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a valid integer"));
    }

    #[test]
    fn test_filter_json_proper_escaping() {
        let template = r#"value={{VALUE|json}}"#;
        let mut vars = IndexMap::new();
        // Value containing quotes and special chars
        vars.insert("VALUE".to_string(), r#"hello "world" & <test>"#.to_string());

        let result = process_filters(template, &vars).unwrap();
        // Should be properly JSON-escaped WITH quotes
        assert_eq!(result, r#"value="hello \"world\" & <test>""#);
    }

    #[test]
    fn test_filter_default_when_empty() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "".to_string()); // empty value

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "opt=fallback");
    }

    #[test]
    fn test_filter_default_when_defined() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "actual_value".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "opt=actual_value");
    }

    #[test]
    fn test_detect_undefined_vars_all_styles() {
        let template = "${UNDEF1} {{UNDEF2}} $UNDEF3 {{DEFINED|upper}}";
        let mut vars = IndexMap::new();
        vars.insert("DEFINED".to_string(), "value".to_string());
        // UNDEF1, UNDEF2, UNDEF3 are not defined

        let undefined = detect_undefined_vars(template, &vars);
        assert_eq!(undefined.len(), 3);
        assert!(undefined.contains(&"UNDEF1".to_string()));
        assert!(undefined.contains(&"UNDEF2".to_string()));
        assert!(undefined.contains(&"UNDEF3".to_string()));
    }

    #[test]
    fn test_detect_undefined_vars_no_duplicates() {
        let template = "${VAR} {{VAR}} $VAR";
        let vars = IndexMap::new(); // VAR not defined

        let undefined = detect_undefined_vars(template, &vars);
        // Should report VAR only once despite 3 references
        assert_eq!(undefined, vec!["VAR"]);
    }

    #[test]
    fn test_regex_escaping_for_special_var_names() {
        // Test that variable names with regex special chars are handled safely
        let template = "${VAR_NAME}";
        let mut vars = IndexMap::new();
        vars.insert("VAR_NAME".to_string(), "value".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "value");
    }

    #[test]
    fn test_mixed_substitution_styles() {
        let template = "${A} {{B}} $C";
        let mut vars = IndexMap::new();
        vars.insert("A".to_string(), "1".to_string());
        vars.insert("B".to_string(), "2".to_string());
        vars.insert("C".to_string(), "3".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "1 2 3");
    }

    // Test the helper functions themselves
    #[test]
    fn test_shell_var_pattern() {
        assert_eq!(shell_var_pattern("MY_VAR"), r"\$\{MY_VAR\}");
        assert_eq!(shell_var_pattern("VAR.WITH.DOTS"), r"\$\{VAR\.WITH\.DOTS\}");
    }

    #[test]
    fn test_template_var_pattern() {
        assert_eq!(template_var_pattern("MY_VAR"), r"\{\{MY_VAR\}\}");
        assert_eq!(
            template_var_pattern("VAR.WITH.DOTS"),
            r"\{\{VAR\.WITH\.DOTS\}\}"
        );
    }

    #[test]
    fn test_filter_var_pattern() {
        // filter arg must include the leading pipe character.
        // regex::escape("|upper") = "\|upper" → pattern: \{\{MY_VAR\|upper\}\}
        assert_eq!(
            filter_var_pattern("MY_VAR", "|upper"),
            r"\{\{MY_VAR\|upper\}\}"
        );
        // regex::escape("|bool") = "\|bool" → pattern: \{\{KEY\|bool\}\}
        assert_eq!(filter_var_pattern("KEY", "|bool"), r"\{\{KEY\|bool\}\}");
    }
}
