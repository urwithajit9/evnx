// ============================================================================
// formats/yaml.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;
pub struct YamlConverter;

impl Converter for YamlConverter {
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

        let yaml = serde_yaml::to_string(&transformed)?;
        Ok(yaml)
    }

    fn name(&self) -> &str {
        "yaml"
    }

    fn description(&self) -> &str {
        "Generic YAML key-value format"
    }
}
