//! Generic generator for unspecified/other stacks.

use crate::generators::{EnvVar, StackGenerator};

/// Generic generator for unspecified or custom stacks.
pub struct OtherGenerator;

impl StackGenerator for OtherGenerator {
    fn id(&self) -> &'static str {
        "other"
    }

    fn display_name(&self) -> &'static str {
        "Other / Custom"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("APP_ENV", "development")
                .with_description("Application environment")
                .with_category("Application"),
            EnvVar::new("APP_PORT", "8080")
                .with_description("Application port")
                .with_category("Server"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_other_generator() {
        assert_eq!(OtherGenerator.id(), "other");
        assert_eq!(OtherGenerator.default_env_vars().len(), 2);
    }
}
