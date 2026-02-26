//! Generator for Twilio SMS/VoIP service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for Twilio SMS/VoIP service.
pub struct TwilioGenerator;

impl ServiceGenerator for TwilioGenerator {
    fn id(&self) -> &'static str {
        "twilio"
    }

    fn display_name(&self) -> &'static str {
        "Twilio"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("TWILIO_ACCOUNT_SID", "ACxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
                .with_description("Twilio account SID")
                .required()
                .with_category("Communications"),
            EnvVar::new("TWILIO_AUTH_TOKEN", "your_auth_token_here")
                .with_description("Twilio auth token")
                .required()
                .with_category("Communications"),
            EnvVar::new("TWILIO_PHONE_NUMBER", "+1234567890")
                .with_description("Twilio phone number")
                .with_category("Communications"),
        ]
    }
}
