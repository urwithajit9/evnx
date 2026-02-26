//! Generator for OpenAI API service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for OpenAI API service.
pub struct OpenAiGenerator;

impl ServiceGenerator for OpenAiGenerator {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![EnvVar::new("OPENAI_API_KEY", "sk-proj-YOUR_KEY_HERE")
            .with_description("OpenAI API key")
            .required()
            .with_category("AI")]
    }
}
