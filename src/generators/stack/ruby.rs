use crate::generators::{EnvVar, StackGenerator};

pub struct RubyGenerator;

impl StackGenerator for RubyGenerator {
    fn id(&self) -> &'static str {
        "ruby"
    }
    fn display_name(&self) -> &'static str {
        "Ruby on Rails"
    }

    fn default_env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("RAILS_ENV", "development").with_category("Application"),
            EnvVar::new("SECRET_KEY_BASE", "generate-with-rails-secret")
                .required()
                .with_category("Security"),
        ]
    }
}
