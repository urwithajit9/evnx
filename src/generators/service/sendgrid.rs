//! Generator for SendGrid email service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for SendGrid email service.
pub struct SendGridGenerator;

impl ServiceGenerator for SendGridGenerator {
    fn id(&self) -> &'static str {
        "sendgrid"
    }

    fn display_name(&self) -> &'static str {
        "SendGrid"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![EnvVar::new("SENDGRID_API_KEY", "SG.YOUR_API_KEY_HERE")
            .with_description("SendGrid API key")
            .required()
            .with_category("Email")]
    }
}
