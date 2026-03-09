// ============================================================================
// formats/docker.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct DockerComposeConverter;

impl Converter for DockerComposeConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let mut output = String::new();
        output.push_str("environment:\n");

        for (k, v) in filtered.iter() {
            let key = options.transform_key(k);
            let value = options.transform_value(v);
            output.push_str(&format!("  - {}={}\n", key, value));
        }

        Ok(output)
    }

    fn name(&self) -> &str {
        "docker-compose"
    }

    fn description(&self) -> &str {
        "Docker Compose YAML environment section"
    }
}
