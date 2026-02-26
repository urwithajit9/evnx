//! Generator for Node.js frameworks (Next.js/Express).

use crate::generators::{EnvVar, StackGenerator};

/// Generator for Node.js frameworks (Next.js/Express).
pub struct NodeJsGenerator;

impl StackGenerator for NodeJsGenerator {
    fn id(&self) -> &'static str {
        "nodejs"
    }

    fn display_name(&self) -> &'static str {
        "Node.js (Next.js/Express)"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("NODE_ENV", "development")
                .with_description("Application environment (development/production)")
                .with_category("Application"),
            EnvVar::new("PORT", "3000")
                .with_description("HTTP server port")
                .with_category("Server"),
            EnvVar::new("HOST", "0.0.0.0")
                .with_description("HTTP server bind address")
                .with_category("Server"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nodejs_generator() {
        assert_eq!(NodeJsGenerator.id(), "nodejs");
        let vars = NodeJsGenerator.default_env_vars();
        assert_eq!(vars.len(), 3);
    }
}
