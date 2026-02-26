//! Environment variable domain model.
//!
//! This module defines the `EnvVar` struct used to represent
//! individual environment variables with metadata for documentation,
//! validation, and template generation.

use std::fmt;

/// Represents a single environment variable configuration.
///
/// # Example
///
/// ```
/// use dotenv_space::generators::env_var::EnvVar;
///
/// let var = EnvVar::new("DATABASE_URL", "postgresql://localhost/db")
///     .with_description("Primary database connection string")
///     .required()
///     .with_category("Database");
///
/// assert_eq!(var.key, "DATABASE_URL");
/// assert!(var.required);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    /// The variable name (e.g., `"DATABASE_URL"`).
    pub key: String,
    /// Default/placeholder value for `.env.example`.
    pub example_value: String,
    /// Human-readable description for documentation.
    pub description: Option<String>,
    /// Whether this variable is required for the application to run.
    pub required: bool,
    /// Optional category for grouping in output (e.g., `"Database"`, `"Auth"`).
    pub category: Option<String>,
}

impl EnvVar {
    /// Create a new `EnvVar` with key and example value.
    ///
    /// # Arguments
    ///
    /// * `key` - The environment variable name (conventionally SCREAMING_SNAKE_CASE)
    /// * `example_value` - Placeholder value for `.env.example`
    #[must_use]
    pub fn new(key: impl Into<String>, example_value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            example_value: example_value.into(),
            description: None,
            required: false,
            category: None,
        }
    }

    /// Add a human-readable description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Mark this variable as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Assign to a category for grouped output.
    #[must_use]
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Format as a single line for `.env.example` output.
    ///
    /// If a description exists, it's rendered as a comment above the variable.
    #[must_use]
    pub fn to_example_line(&self) -> String {
        let mut lines = Vec::new();
        if let Some(desc) = &self.description {
            lines.push(format!("# {}", desc));
        }
        lines.push(format!("{}={}", self.key, self.example_value));
        lines.join("\n")
    }

    /// Format as a shell export statement.
    #[must_use]
    pub fn to_shell_export(&self) -> String {
        format!("export {}=\"{}\"", self.key, self.example_value)
    }
}

impl fmt::Display for EnvVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.key, self.example_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_var() {
        let var = EnvVar::new("KEY", "value");
        assert_eq!(var.key, "KEY");
        assert_eq!(var.example_value, "value");
        assert!(!var.required);
    }

    #[test]
    fn test_with_description() {
        let var = EnvVar::new("K", "v").with_description("desc");
        assert_eq!(var.description, Some("desc".to_string()));
    }

    #[test]
    fn test_to_example_line_with_desc() {
        let var = EnvVar::new("K", "v").with_description("My description");
        assert_eq!(var.to_example_line(), "# My description\nK=v");
    }

    #[test]
    fn test_to_example_line_without_desc() {
        let var = EnvVar::new("K", "v");
        assert_eq!(var.to_example_line(), "K=v");
    }

    #[test]
    fn test_display_trait() {
        let var = EnvVar::new("PORT", "8080");
        assert_eq!(format!("{}", var), "PORT=8080");
    }
}
