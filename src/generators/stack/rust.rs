//! Generator for Rust frameworks (Axum/Actix).

use crate::generators::{EnvVar, StackGenerator};

/// Generator for Rust frameworks (Axum/Actix).
pub struct RustGenerator;

impl StackGenerator for RustGenerator {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn display_name(&self) -> &'static str {
        "Rust (Axum/Actix)"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("APP__ENVIRONMENT", "development")
                .with_description("Application environment (development/staging/production)")
                .with_category("Application"),
            EnvVar::new("APP__PORT", "8080")
                .with_description("HTTP server port")
                .with_category("Server"),
            EnvVar::new("APP__ADDRESS", "0.0.0.0")
                .with_description("HTTP server bind address")
                .with_category("Server"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_generator() {
        assert_eq!(RustGenerator.id(), "rust");
        let vars = RustGenerator.default_env_vars();
        assert_eq!(vars.len(), 3);
    }
}
