//! Generator for Go frameworks (Gin/Fiber).

use crate::generators::{EnvVar, StackGenerator};

/// Generator for Go frameworks (Gin/Fiber).
pub struct GoGenerator;

impl StackGenerator for GoGenerator {
    fn id(&self) -> &'static str {
        "go"
    }

    fn display_name(&self) -> &'static str {
        "Go (Gin/Fiber)"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("APP_ENV", "development")
                .with_description("Application environment")
                .with_category("Application"),
            EnvVar::new("PORT", "8080")
                .with_description("HTTP server port")
                .with_category("Server"),
            EnvVar::new("ADDR", "0.0.0.0")
                .with_description("HTTP server bind address")
                .with_category("Server"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_generator() {
        assert_eq!(GoGenerator.id(), "go");
        assert_eq!(GoGenerator.default_env_vars().len(), 3);
    }
}
