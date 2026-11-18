// ============================================================================
// formats/docker.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use crate::core::secret::Vars;
use anyhow::Result;
// use std::collections::HashMap;

pub struct DockerComposeConverter;

impl Converter for DockerComposeConverter {
    fn convert(&self, vars: &Vars, options: &ConvertOptions) -> Result<String> {
        let mut output = String::new();
        output.push_str("environment:\n");

        for (key, value) in options.prepare(vars, self)? {
            // ⛔ The whole `KEY=value` item as one quoted scalar. Unquoted,
            // Compose reads `-` items the same way YAML does — `a: b` turns the
            // item into a map, `#` truncates — and additionally interpolates
            // `$`, for which a literal needs `$$`.
            let item = format!("{}={}", key, value.replace('$', "$$"));
            output.push_str(&format!(
                "  - {}\n",
                crate::formats::quoting::yaml_scalar(&item)
            ));
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
