//! Generator traits and module exports.
//!
//! This module defines the core traits for stack and service generators,
//! and re-exports submodules for convenient access.

pub mod env_var;
pub mod service;
pub mod stack;
pub mod template;

pub use env_var::EnvVar;

use anyhow::Result;

/// Trait for framework/stack-specific environment variable generation.
///
/// Implementors provide default environment variables for a given technology stack.
pub trait StackGenerator: Send + Sync {
    /// Machine-readable identifier (used in CLI flags and config).
    fn id(&self) -> &'static str;

    /// Human-readable name for UI/display purposes.
    fn display_name(&self) -> &'static str;

    /// Default environment variables for this stack.
    fn default_env_vars(&self) -> Vec<EnvVar>;

    /// Optional: Interactive prompts for stack-specific configuration.
    ///
    /// Default implementation returns empty; override if custom prompts are needed.
    fn interactive_setup(&self) -> Result<Vec<EnvVar>> {
        Ok(Vec::new())
    }
}

/// Trait for third-party service environment variable generation.
///
/// Implementors provide environment variables required to integrate with external services.
pub trait ServiceGenerator: Send + Sync {
    /// Machine-readable identifier.
    fn id(&self) -> &'static str;

    /// Human-readable name for UI/display purposes.
    fn display_name(&self) -> &'static str;

    /// Environment variables required for this service.
    fn env_vars(&self) -> Vec<EnvVar>;
}

/// Helper to normalize service names from display format to machine ID.
///
/// Converts "PostgreSQL" → "postgresql", "AWS S3" → "aws_s3", etc.
#[must_use]
pub fn normalize_service_name(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_service_name() {
        assert_eq!(normalize_service_name("PostgreSQL"), "postgresql");
        assert_eq!(normalize_service_name("AWS S3"), "aws_s3");
        assert_eq!(normalize_service_name("SendGrid"), "sendgrid");
    }
}
