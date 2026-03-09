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
            output.push_str(&format!("  {}: {}\n", key, value));
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
