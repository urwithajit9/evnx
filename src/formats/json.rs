// ============================================================================
// formats/json.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct JsonConverter;

impl Converter for JsonConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let transformed: IndexMap<String, String> = filtered
            .iter()
            .map(|(k, v)| {
                let key = options.transform_key(k);
                let value = options.transform_value(v);
                (key, value)
            })
            .collect();

        let json = serde_json::to_string_pretty(&transformed)?;
        Ok(json)
    }

    fn name(&self) -> &str {
        "json"
    }

    fn description(&self) -> &str {
        "Generic JSON key-value format"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::collections::HashMap;

    #[test]
    fn test_json_converter() {
        let mut vars = IndexMap::new();
        vars.insert("KEY".to_string(), "value".to_string());

        let converter = JsonConverter;
        let options = ConvertOptions::default();
        let result = converter.convert(&vars, &options).unwrap();

        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["KEY"], "value");
    }
}
