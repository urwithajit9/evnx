//! Generator for PHP frameworks (Laravel).

use crate::generators::{EnvVar, StackGenerator};

/// Generator for PHP frameworks (Laravel).
pub struct PhpGenerator;

impl StackGenerator for PhpGenerator {
    fn id(&self) -> &'static str {
        "php"
    }

    fn display_name(&self) -> &'static str {
        "PHP (Laravel)"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("APP_NAME", "Laravel")
                .with_description("Application name")
                .with_category("Application"),
            EnvVar::new("APP_ENV", "local")
                .with_description("Application environment")
                .with_category("Application"),
            EnvVar::new("APP_KEY", "base64:generate-with-php-artisan-key-generate")
                .with_description("Laravel encryption key")
                .required()
                .with_category("Security"),
            EnvVar::new("APP_DEBUG", "true")
                .with_description("Enable debug mode")
                .with_category("Application"),
            EnvVar::new("APP_URL", "http://localhost")
                .with_description("Application base URL")
                .with_category("Application"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_php_generator() {
        assert_eq!(PhpGenerator.id(), "php");
        let vars = PhpGenerator.default_env_vars();
        assert_eq!(vars.len(), 5);
        let app_key = vars.iter().find(|v| v.key == "APP_KEY").unwrap();
        assert!(app_key.required);
    }
}
