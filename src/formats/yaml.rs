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
        let transformed: IndexMap<String, String> =
            options.prepare(vars, self)?.into_iter().collect();

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
