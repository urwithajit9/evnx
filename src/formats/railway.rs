// ============================================================================
// Railway Variables
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct RailwayConverter;

impl Converter for RailwayConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let transformed: IndexMap<String, String> =
            options.prepare(vars, self)?.into_iter().collect();

        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }

    fn name(&self) -> &str {
        "railway"
    }

    fn description(&self) -> &str {
        "Railway JSON format"
    }
}
