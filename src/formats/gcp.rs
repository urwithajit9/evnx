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

/// The secret ID `key` gets in Secret Manager: lowercase, `_` as `-`
/// (`DB_URL` → `db-url`).
///
/// ⚠️ **The one definition** of the rename, for `convert --to gcp-secrets`,
/// `migrate --to gcp-secret-manager` and `cloud export --to gcp-secrets` (the
/// last renders through the converter), so the three cannot drift (I-098).
/// GCP does not require it — secret IDs take `A-Z a-z 0-9 _ -` (the
/// `GcpSecretId` row of `core::key_rules`, which still accepts the renamed
/// ID) — but it is the name every earlier `convert` script created (DEC-CV2).
/// `A_B`/`a_b` becoming one `a-b` is refused by
/// [`ConvertOptions::prepare`](crate::core::converter::ConvertOptions::prepare)
/// rather than printed twice (I-249).
pub fn secret_id(key: &str) -> String {
    key.to_lowercase().replace('_', "-")
}

/// The comment above [`upsert_function`], for a reader of the printed script.
pub const UPSERT_COMMENT: &str =
    "# Creates each secret, or adds a new version when it already exists.";

/// `upsert_secret NAME`: create the secret from stdin, or add a version when
/// it exists. One definition for `convert --to gcp-secrets` and
/// `migrate --to gcp-secret-manager` (I-100), so the two print the same thing.
///
/// ⛔ Create, or add a version (I-099). `gcloud secrets create` alone "will
/// return an error" for an existing secret (gcloud's own help), so a second
/// run — the normal way to update a value — failed for every secret.
pub fn upsert_function(project_id: &str) -> String {
    let project = crate::core::shell::word(project_id);
    format!(
        "upsert_secret() {{\n\
         \x20 if gcloud secrets describe \"$1\" --project={project} </dev/null >/dev/null 2>&1; then\n\
         \x20   gcloud secrets versions add \"$1\" --data-file=- --project={project}\n\
         \x20 else\n\
         \x20   gcloud secrets create \"$1\" --data-file=- --project={project}\n\
         \x20 fi\n\
         }}\n"
    )
}

/// `printf '%s' 'VALUE' | upsert_secret NAME`, with no line break.
///
/// ⛔ `printf '%s'`, not `echo`, and the value quoted by the shared helper.
///
/// Two defects in one line. A single quote in the value closed the quoting
/// and the remainder became shell. And `echo` with no `-n` appends a newline,
/// so **every** secret pushed this way was stored one byte longer than the
/// value — which breaks an exact comparison and, for a key or token, breaks
/// the key. `migrate` printed `echo -n "${NAME}"` instead, which sent whatever
/// the user's shell held under that name, or nothing (I-100).
pub fn upsert_line(name: &str, value: &str) -> String {
    format!(
        "printf '%s' {} | upsert_secret {}",
        crate::formats::quoting::shell_single(value),
        crate::core::shell::word(name),
    )
}

impl Converter for GcpSecretConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to GCP Secret Manager\n");
        output.push_str(&format!("# Project: {}\n", self.project_id));
        // ⛔ Create, or add a version when the secret exists (I-099). The
        // script used `gcloud secrets create` alone, which "will return an
        // error" for an existing secret (gcloud's own help), so a second run —
        // the normal way to update a value — failed for every secret. The
        // value is still printed once, and still reaches gcloud on stdin.
        output.push_str(&format!("#\n{UPSERT_COMMENT}\n\n"));
        output.push_str(&upsert_function(&self.project_id));
        output.push('\n');

        for (secret_name, value) in options.prepare(vars, self)? {
            output.push_str(&upsert_line(&secret_name, &value));
            output.push('\n');
        }

        Ok(output)
    }

    fn name(&self) -> &str {
        "gcp-secrets"
    }

    fn description(&self) -> &str {
        "GCP Secret Manager (gcloud commands)"
    }

    /// Lowercase with hyphens: [`secret_id`], shared with `migrate` (I-098).
    fn target_name(&self, key: &str) -> String {
        secret_id(key)
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

    /// I-098: convert, migrate and export name secrets by one function.
    #[test]
    fn t_i098_secret_id_is_lowercase_hyphen() {
        assert_eq!(secret_id("DB_URL"), "db-url");
        assert_eq!(secret_id("db-url"), "db-url");
        assert_eq!(secret_id(&secret_id("A_B")), "a-b");
    }

    #[test]
    fn t_i098_converter_names_come_from_secret_id() {
        let c = GcpSecretConverter::default();
        for k in ["DB_URL", "API_KEY", "x"] {
            assert_eq!(Converter::target_name(&c, k), secret_id(k));
        }
    }
}
