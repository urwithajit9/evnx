// Converter trait and options for format conversion
//
// Provides the core trait and configuration for converting environment variables
// to different output formats.

use anyhow::Result;
use std::collections::HashMap;

/// Key transformation options
#[derive(Debug, Clone)]
pub enum KeyTransform {
    /// Transform keys to UPPERCASE
    Uppercase,
    /// Transform keys to lowercase
    Lowercase,
    /// Transform keys to camelCase
    CamelCase,
    /// Transform keys to snake_case
    SnakeCase,
}

/// Options for format conversion
///
/// ✅ CLIPPY FIX: Uses `#[derive(Default)]` instead of manual implementation
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Include only variables matching this glob pattern
    pub include_pattern: Option<String>,

    /// Exclude variables matching this glob pattern
    pub exclude_pattern: Option<String>,

    /// Base64-encode all values
    pub base64: bool,

    /// Prefix to add to all keys
    pub prefix: Option<String>,

    /// Key transformation to apply
    pub transform: Option<KeyTransform>,
}

// ✅ CLIPPY FIX: Removed manual impl Default (using #[derive(Default)])

impl ConvertOptions {
    /// Create new default options
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter variables based on include/exclude patterns
    pub fn filter_vars(&self, vars: &HashMap<String, String>) -> HashMap<String, String> {
        vars.iter()
            .filter(|(key, _)| self.should_include(key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Check if a variable should be included
    fn should_include(&self, key: &str) -> bool {
        // Check exclude pattern first
        if let Some(ref exclude) = self.exclude_pattern {
            if glob_match(key, exclude) {
                return false;
            }
        }

        // Check include pattern
        if let Some(ref include) = self.include_pattern {
            return glob_match(key, include);
        }

        // Include by default if no patterns specified
        true
    }

    /// Transform a key according to the specified transformation
    pub fn transform_key(&self, key: &str) -> String {
        let key = if let Some(ref prefix) = self.prefix {
            format!("{}{}", prefix, key)
        } else {
            key.to_string()
        };

        match self.transform {
            Some(KeyTransform::Uppercase) => key.to_uppercase(),
            Some(KeyTransform::Lowercase) => key.to_lowercase(),
            Some(KeyTransform::CamelCase) => to_camel_case(&key),
            Some(KeyTransform::SnakeCase) => to_snake_case(&key),
            None => key,
        }
    }

    /// Transform a value (base64 encode if enabled)
    pub fn transform_value(&self, value: &str) -> String {
        if self.base64 {
            use base64::{engine::general_purpose, Engine as _};
            general_purpose::STANDARD.encode(value.as_bytes())
        } else {
            value.to_string()
        }
    }
}

/// Converter trait for format conversion
///
/// Implement this trait to add support for a new output format.
///
/// # Example
///
/// ```rust,ignore
/// use dotenv_space::core::converter::{Converter, ConvertOptions};
/// use anyhow::Result;
/// use std::collections::HashMap;
///
/// pub struct MyFormatConverter;
///
/// impl Converter for MyFormatConverter {
///     fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String> {
///         let filtered = options.filter_vars(vars);
///         // ... format conversion logic
///         Ok(output)
///     }
///
///     fn name(&self) -> &str {
///         "my-format"
///     }
///
///     fn description(&self) -> &str {
///         "My custom format description"
///     }
/// }
/// ```
pub trait Converter {
    /// Convert environment variables to the target format
    ///
    /// # Arguments
    ///
    /// * `vars` - Environment variables to convert
    /// * `options` - Conversion options (filtering, transformations, etc.)
    ///
    /// # Returns
    ///
    /// Formatted output as a string
    fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String>;

    /// Get the name of this format
    ///
    /// Used for format selection and error messages.
    fn name(&self) -> &str;

    /// Get a description of this format
    ///
    /// Used in help text and interactive selection.
    fn description(&self) -> &str;
}

// Helper functions

/// Simple glob pattern matching
///
/// Supports `*` wildcard matching.
fn glob_match(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with('*') && pattern.ends_with('*') {
        let substring = &pattern[1..pattern.len() - 1];
        return text.contains(substring);
    }

    // if pattern.starts_with('*') {
    //     let suffix = &pattern[1..];
    //     return text.ends_with(suffix);
    // }

    // if pattern.ends_with('*') {
    //     let prefix = &pattern[..pattern.len() - 1];
    //     return text.starts_with(prefix);
    // }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return text.ends_with(suffix);
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return text.starts_with(prefix);
    }

    text == pattern
}

/// Convert string to camelCase
fn to_camel_case(s: &str) -> String {
    let parts: Vec<&str> = s.split('_').collect();
    if parts.is_empty() {
        return String::new();
    }

    let mut result = parts[0].to_lowercase();
    for part in &parts[1..] {
        if !part.is_empty() {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                result.push_str(&first.to_uppercase().to_string());
                result.push_str(&chars.as_str().to_lowercase());
            }
        }
    }
    result
}

/// Convert string to snake_case
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_is_upper = false;

    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 && !prev_is_upper {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap());
            prev_is_upper = true;
        } else {
            result.push(ch);
            prev_is_upper = false;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = ConvertOptions::default();
        assert!(opts.include_pattern.is_none());
        assert!(opts.exclude_pattern.is_none());
        assert!(!opts.base64);
        assert!(opts.prefix.is_none());
        assert!(opts.transform.is_none());
    }

    #[test]
    fn test_new_options() {
        let opts = ConvertOptions::new();
        assert!(opts.include_pattern.is_none());
    }

    #[test]
    fn test_filter_vars_no_pattern() {
        let mut vars = HashMap::new();
        vars.insert("KEY1".to_string(), "value1".to_string());
        vars.insert("KEY2".to_string(), "value2".to_string());

        let opts = ConvertOptions::default();
        let filtered = opts.filter_vars(&vars);

        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains_key("KEY1"));
        assert!(filtered.contains_key("KEY2"));
    }

    #[test]
    fn test_filter_vars_include() {
        let mut vars = HashMap::new();
        vars.insert("AWS_KEY".to_string(), "value1".to_string());
        vars.insert("DB_KEY".to_string(), "value2".to_string());

        let mut opts = ConvertOptions::default();
        opts.include_pattern = Some("AWS_*".to_string());
        let filtered = opts.filter_vars(&vars);

        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("AWS_KEY"));
        assert!(!filtered.contains_key("DB_KEY"));
    }

    #[test]
    fn test_filter_vars_exclude() {
        let mut vars = HashMap::new();
        vars.insert("KEY1".to_string(), "value1".to_string());
        vars.insert("KEY2_LOCAL".to_string(), "value2".to_string());

        let mut opts = ConvertOptions::default();
        opts.exclude_pattern = Some("*_LOCAL".to_string());
        let filtered = opts.filter_vars(&vars);

        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("KEY1"));
        assert!(!filtered.contains_key("KEY2_LOCAL"));
    }

    #[test]
    fn test_transform_key_prefix() {
        let mut opts = ConvertOptions::default();
        opts.prefix = Some("APP_".to_string());

        assert_eq!(opts.transform_key("DATABASE_URL"), "APP_DATABASE_URL");
    }

    #[test]
    fn test_transform_key_uppercase() {
        let mut opts = ConvertOptions::default();
        opts.transform = Some(KeyTransform::Uppercase);

        assert_eq!(opts.transform_key("database_url"), "DATABASE_URL");
    }

    #[test]
    fn test_transform_key_lowercase() {
        let mut opts = ConvertOptions::default();
        opts.transform = Some(KeyTransform::Lowercase);

        assert_eq!(opts.transform_key("DATABASE_URL"), "database_url");
    }

    #[test]
    fn test_transform_value_base64() {
        let mut opts = ConvertOptions::default();
        opts.base64 = true;

        let result = opts.transform_value("hello");
        assert_eq!(result, "aGVsbG8="); // base64("hello")
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("AWS_KEY", "AWS_*"));
        assert!(glob_match("MY_AWS_KEY", "*_AWS_*"));
        assert!(glob_match("KEY_LOCAL", "*_LOCAL"));
        assert!(glob_match("anything", "*"));
        assert!(!glob_match("AWS_KEY", "DB_*"));
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("database_url"), "databaseUrl");
        assert_eq!(to_camel_case("secret_key"), "secretKey");
        assert_eq!(to_camel_case("aws_access_key_id"), "awsAccessKeyId");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("DatabaseURL"), "database_u_r_l");
        assert_eq!(to_snake_case("SecretKey"), "secret_key");
    }
}
