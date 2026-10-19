// ============================================================================
// Doppler Format
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct DopplerConverter;

impl Converter for DopplerConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let transformed: IndexMap<String, serde_json::Value> = options
            .prepare(vars, self)?
            .into_iter()
            .map(|(key, value)| {
                (
                    key,
                    serde_json::json!({
                        "value": value,
                        "computed": false
                    }),
                )
            })
            .collect();

        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }

    fn name(&self) -> &str {
        "doppler"
    }

    fn description(&self) -> &str {
        "Doppler secrets JSON format"
    }
}
