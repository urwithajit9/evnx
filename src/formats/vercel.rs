// ============================================================================
// Vercel Environment Variables
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct VercelEnvConverter;

/// Targets of a variable that is not secret.
const PLAIN_TARGETS: [&str; 3] = ["production", "preview", "development"];
/// Targets of a Sensitive variable. ⚠️ Not `development`: Vercel's API
/// rejects it ("You cannot set a Sensitive Environment Variable's target to
/// development"; <https://vercel.com/docs/environment-variables/sensitive-environment-variables>).
const SENSITIVE_TARGETS: [&str; 2] = ["production", "preview"];

/// `vercel env add --sensitive` applies when `key=value` looks secret and the
/// environment is not `development` (shared with `migrate --to vercel`).
pub fn is_sensitive(key: &str, value: &str) -> bool {
    crate::core::classify::holds_secret(key, value)
}

impl Converter for VercelEnvConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let mut sensitive = 0usize;
        let transformed: IndexMap<String, serde_json::Value> = options
            .prepare(vars, self)?
            .into_iter()
            .map(|(key, value)| {
                let entry = if is_sensitive(&key, &value) {
                    sensitive += 1;
                    serde_json::json!({
                        "type": "sensitive",
                        "value": value,
                        "target": SENSITIVE_TARGETS
                    })
                } else {
                    serde_json::json!({
                        "type": "plain",
                        "value": value,
                        "target": PLAIN_TARGETS
                    })
                };
                (key, entry)
            })
            .collect();

        if sensitive > 0 {
            // stderr, never in the JSON.
            crate::utils::ui::warning(format!(
                "{sensitive} variable{} written as type \"sensitive\" and not set for \
                 development: Vercel does not allow a Sensitive variable there.",
                if sensitive == 1 { "" } else { "s" }
            ));
        }

        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }

    fn name(&self) -> &str {
        "vercel"
    }

    fn description(&self) -> &str {
        "Vercel Environment Variables JSON"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pairs: &[(&str, &str)]) -> serde_json::Value {
        let vars: IndexMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let out = VercelEnvConverter
            .convert(&vars, &ConvertOptions::default())
            .unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn secret_key_is_sensitive_without_development() {
        let v = run(&[("DB_PASSWORD", "hunter2")]);
        assert_eq!(v["DB_PASSWORD"]["type"], "sensitive");
        assert_eq!(
            v["DB_PASSWORD"]["target"],
            serde_json::json!(["production", "preview"])
        );
    }

    #[test]
    fn plain_key_is_unchanged() {
        let v = run(&[("APP_NAME", "demo")]);
        assert_eq!(v["APP_NAME"]["type"], "plain");
        assert_eq!(
            v["APP_NAME"]["target"],
            serde_json::json!(["production", "preview", "development"])
        );
    }
}
