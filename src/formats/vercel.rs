// ============================================================================
// Vercel Environment Variables
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct VercelEnvConverter;

impl Converter for VercelEnvConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let transformed: IndexMap<String, serde_json::Value> = options
            .prepare(vars, self)?
            .into_iter()
            .map(|(key, value)| {
                (
                    key,
                    serde_json::json!({
                        "type": "plain",
                        "value": value,
                        "target": ["production", "preview", "development"]
                    }),
                )
            })
            .collect();

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
