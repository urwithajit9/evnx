//! Generator for Sentry error tracking service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for Sentry error tracking service.
pub struct SentryGenerator;

impl ServiceGenerator for SentryGenerator {
    fn id(&self) -> &'static str {
        "sentry"
    }

    fn display_name(&self) -> &'static str {
        "Sentry"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("SENTRY_DSN", "https://YOUR_SENTRY_DSN@sentry.io/PROJECT_ID")
                .with_description("Sentry DSN for error tracking")
                .required()
                .with_category("Monitoring"),
        ]
    }
}
