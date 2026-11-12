// ============================================================================
// Vercel Environment Variables
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct VercelEnvConverter;

/// Targets of every variable, secret or not.
///
/// ⚠️ A Sensitive (now "Secret") variable is allowed in `development`. Vercel
/// reversed its earlier rule that limited them to Production and Preview:
/// "Secrets are allowed in Development. This reverses the earlier rule that
/// limited Sensitive environment variables to Production and Preview"
/// (<https://vercel.com/docs/environment-variables/sensitive-environment-variables>,
/// updated 2026-08-28; changelog
/// <https://vercel.com/changelog/environment-variables-now-use-config-and-secret-types>).
/// evnx 0.13.0 followed the old rule and left them out of `development` (F14-2).
const TARGETS: [&str; 3] = ["production", "preview", "development"];

/// `vercel env add --sensitive` applies when `key=value` looks secret, in every
/// environment (shared with `migrate --to vercel`). `--sensitive` is the legacy
/// spelling of the Secret type and still works.
pub fn is_sensitive(key: &str, value: &str) -> bool {
    crate::core::classify::holds_secret(key, value)
}

impl Converter for VercelEnvConverter {
    /// The request body of Vercel's "create one or more environment variables"
    /// (`POST /v10/projects/{idOrName}/env`): an array of
    /// `{key, value, type, target}`, where `type` is `plain` or `sensitive`
    /// (RM-65, I-092). It was a map keyed by name, which that endpoint does
    /// not take.
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let entries: Vec<serde_json::Value> = options
            .prepare(vars, self)?
            .into_iter()
            .map(|(key, value)| {
                let kind = if is_sensitive(&key, &value) {
                    "sensitive"
                } else {
                    "plain"
                };
                serde_json::json!({
                    "key": key,
                    "value": value,
                    "type": kind,
                    "target": TARGETS
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&entries)?)
    }

    fn name(&self) -> &str {
        "vercel"
    }

    fn description(&self) -> &str {
        "Vercel environment variables (JSON request body)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pairs: &[(&str, &str)]) -> serde_json::Value {
        let list = run_list(pairs);
        let mut map = serde_json::Map::new();
        for e in list.as_array().unwrap() {
            map.insert(e["key"].as_str().unwrap().to_string(), e.clone());
        }
        serde_json::Value::Object(map)
    }

    fn run_list(pairs: &[(&str, &str)]) -> serde_json::Value {
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
    fn secret_key_is_sensitive_in_every_environment() {
        let v = run(&[("DB_PASSWORD", "hunter2")]);
        assert_eq!(v["DB_PASSWORD"]["type"], "sensitive");
        assert_eq!(
            v["DB_PASSWORD"]["target"],
            serde_json::json!(["production", "preview", "development"])
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

    /// The shape of the documented request body: an array, each entry with
    /// exactly `key`, `value`, `type` and `target`.
    #[test]
    fn output_is_the_documented_request_body_array() {
        let v = run_list(&[("APP_NAME", "demo"), ("DB_PASSWORD", "hunter2")]);
        let list = v.as_array().expect("an array");
        assert_eq!(list.len(), 2);
        for e in list {
            let mut keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["key", "target", "type", "value"]);
        }
        assert_eq!(list[0]["key"], "APP_NAME");
        assert_eq!(list[1]["type"], "sensitive");
    }
}
