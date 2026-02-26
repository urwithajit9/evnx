//! Generator for Python frameworks (Django/FastAPI).

use crate::generators::{EnvVar, StackGenerator};

/// Generator for Python frameworks (Django/FastAPI).
pub struct PythonGenerator;

impl StackGenerator for PythonGenerator {
    fn id(&self) -> &'static str {
        "python"
    }

    fn display_name(&self) -> &'static str {
        "Python (Django/FastAPI)"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("SECRET_KEY", "generate-with-openssl-rand-hex-32")
                .with_description("Django/FastAPI secret key for cryptographic signing")
                .required()
                .with_category("Security"),
            EnvVar::new("DEBUG", "True")
                .with_description("Enable debug mode (disable in production)")
                .with_category("Application"),
            EnvVar::new("ALLOWED_HOSTS", "localhost,127.0.0.1")
                .with_description("Comma-separated list of allowed host headers")
                .with_category("Security"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_generator_id() {
        assert_eq!(PythonGenerator.id(), "python");
    }

    #[test]
    fn test_python_default_vars_count() {
        let vars = PythonGenerator.default_env_vars();
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn test_secret_key_required() {
        let vars = PythonGenerator.default_env_vars();
        let secret_key = vars.iter().find(|v| v.key == "SECRET_KEY").unwrap();
        assert!(secret_key.required);
    }
}
