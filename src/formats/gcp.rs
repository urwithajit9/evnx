// ============================================================================
// GCP Secret Manager
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct GcpSecretConverter {
    pub project_id: String,
}

impl Default for GcpSecretConverter {
    fn default() -> Self {
        Self {
            project_id: "my-project".to_string(),
        }
    }
}

impl Converter for GcpSecretConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to GCP Secret Manager\n");
        output.push_str(&format!("# Project: {}\n\n", self.project_id));

        for (k, v) in filtered.iter() {
            let key = options.transform_key(k);
            let value = options.transform_value(v);

            // Convert to lowercase with hyphens (GCP naming)
            let secret_name = key.to_lowercase().replace('_', "-");

            output.push_str(&format!(
                // ⛔ `printf '%s'`, not `echo`, and the value quoted by the
                // shared helper.
                //
                // Two defects in one line. A single quote in the value closed
                // the quoting and the remainder became shell. And `echo` with no
                // `-n` appends a newline, so **every** secret pushed this way
                // was stored one byte longer than the value — which breaks an
                // exact comparison and, for a key or token, breaks the key.
                "printf '%s' {} | gcloud secrets create {} --data-file=- --project={}\n",
                crate::formats::quoting::shell_single(&value),
                secret_name,
                self.project_id
            ));
        }

        Ok(output)
    }

    fn name(&self) -> &str {
        "gcp-secrets"
    }

    fn description(&self) -> &str {
        "GCP Secret Manager (gcloud commands)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcp_converter() {
        let mut vars = IndexMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let converter = GcpSecretConverter::default();
        let result = converter
            .convert(&vars, &ConvertOptions::default())
            .unwrap();

        assert!(result.contains("gcloud secrets create"));
        assert!(result.contains("database-url"));
    }
}
