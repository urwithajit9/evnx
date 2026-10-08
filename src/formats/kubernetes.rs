// ============================================================================
// formats/kubernetes.rs
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;
pub struct KubernetesSecretConverter {
    pub secret_name: String,
}

impl Default for KubernetesSecretConverter {
    fn default() -> Self {
        Self {
            secret_name: "app-secrets".to_string(),
        }
    }
}

impl Converter for KubernetesSecretConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let mut output = String::new();
        output.push_str("apiVersion: v1\n");
        output.push_str("kind: Secret\n");
        output.push_str("metadata:\n");
        output.push_str(&format!("  name: {}\n", self.secret_name));
        output.push_str("type: Opaque\n");

        if options.base64 {
            output.push_str("data:\n");
        } else {
            output.push_str("stringData:\n");
        }

        for (k, v) in filtered.iter() {
            let key = options.transform_key(k);
            let value = options.transform_value(v);
            // ⛔ A quoted scalar, always. Unquoted, YAML types the value by its
            // shape: `PORT: 8080` is an integer and `BOOL: true` a boolean, both
            // of which kubectl rejects under `stringData:`; `a: b` becomes a
            // mapping; `#` starts a comment; and a value containing `\n---\n`
            // starts a second manifest.
            output.push_str(&format!(
                "  {}: {}\n",
                key,
                crate::formats::quoting::yaml_scalar(value.as_str())
            ));
        }

        Ok(output)
    }

    fn name(&self) -> &str {
        "kubernetes"
    }

    fn description(&self) -> &str {
        "Kubernetes Secret YAML"
    }
}
