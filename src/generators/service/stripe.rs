//! Generator for Stripe payment service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for Stripe payment service.
pub struct StripeGenerator;

impl ServiceGenerator for StripeGenerator {
    fn id(&self) -> &'static str {
        "stripe"
    }

    fn display_name(&self) -> &'static str {
        "Stripe"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("STRIPE_SECRET_KEY", "sk_test_YOUR_KEY_HERE")
                .with_description("Stripe secret API key")
                .required()
                .with_category("Payments"),
            EnvVar::new("STRIPE_PUBLISHABLE_KEY", "pk_test_YOUR_KEY_HERE")
                .with_description("Stripe publishable API key")
                .required()
                .with_category("Payments"),
            EnvVar::new("STRIPE_WEBHOOK_SECRET", "whsec_YOUR_WEBHOOK_SECRET")
                .with_description("Stripe webhook signing secret")
                .with_category("Payments"),
        ]
    }
}
