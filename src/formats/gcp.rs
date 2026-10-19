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

/// ⚠️ `my-project` is a placeholder. `convert` requires `--project` (I-084,
/// DEC-CV1); only `cloud export` still builds this default.
impl Default for GcpSecretConverter {
    fn default() -> Self {
        Self {
            project_id: "my-project".to_string(),
        }
    }
}

impl Converter for GcpSecretConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let project = crate::core::shell::word(&self.project_id);

        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to GCP Secret Manager\n");
        output.push_str(&format!("# Project: {}\n", self.project_id));
        // ⛔ Create, or add a version when the secret exists (I-099). The
        // script used `gcloud secrets create` alone, which "will return an
        // error" for an existing secret (gcloud's own help), so a second run —
        // the normal way to update a value — failed for every secret. The
        // value is still printed once, and still reaches gcloud on stdin.
        output.push_str(
            "#\n# Creates each secret, or adds a new version when it already exists.\n\n",
        );
        output.push_str(&format!(
            "upsert_secret() {{\n\
             \x20 if gcloud secrets describe \"$1\" --project={project} </dev/null >/dev/null 2>&1; then\n\
             \x20   gcloud secrets versions add \"$1\" --data-file=- --project={project}\n\
             \x20 else\n\
             \x20   gcloud secrets create \"$1\" --data-file=- --project={project}\n\
             \x20 fi\n\
             }}\n\n"
        ));

        for (secret_name, value) in options.prepare(vars, self)? {
            output.push_str(&format!(
                // ⛔ `printf '%s'`, not `echo`, and the value quoted by the
                // shared helper.
                //
                // Two defects in one line. A single quote in the value closed
                // the quoting and the remainder became shell. And `echo` with no
                // `-n` appends a newline, so **every** secret pushed this way
                // was stored one byte longer than the value — which breaks an
                // exact comparison and, for a key or token, breaks the key.
                "printf '%s' {} | upsert_secret {}\n",
                crate::formats::quoting::shell_single(&value),
                crate::core::shell::word(&secret_name),
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

    /// Lowercase with hyphens. ⚠️ GCP does not require it — secret IDs take
    /// `A-Z a-z 0-9 _ -` — but it is the name every earlier script created, so
    /// it stays (DEC-CV2), and `A_B`/`a_b` becoming one `a-b` is refused rather
    /// than printed twice (I-249). Names as they are, like `migrate`, wait for
    /// the shared target catalogue (I-098).
    fn target_name(&self, key: &str) -> String {
        key.to_lowercase().replace('_', "-")
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

        assert!(result.contains("gcloud secrets create \"$1\""));
        assert!(result.contains("gcloud secrets versions add \"$1\""));
        assert!(result.contains("| upsert_secret database-url\n"));
    }
}
