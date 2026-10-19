// ============================================================================
// Azure Key Vault
// ============================================================================

use crate::core::converter::{ConvertOptions, Converter};
use anyhow::Result;
// use std::collections::HashMap;
use indexmap::IndexMap;

pub struct AzureKeyVaultConverter {
    pub vault_name: String,
}

/// ⚠️ `my-keyvault` is a placeholder. `convert` requires `--vault-name`
/// (I-084, DEC-CV1); only `cloud export` still builds this default.
impl Default for AzureKeyVaultConverter {
    fn default() -> Self {
        Self {
            vault_name: "my-keyvault".to_string(),
        }
    }
}

impl Converter for AzureKeyVaultConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to Azure Key Vault\n");
        output.push_str(&format!("# Vault: {}\n\n", self.vault_name));

        for (secret_name, value) in options.prepare(vars, self)? {
            output.push_str(&format!(
                // ⛔ Quoted by the shared helper, not by wrapping in `'{}'`.
                // A value containing a single quote closed the quoting and the
                // rest of it became shell:
                //
                //   PWNED=x';touch /tmp/pwned;'
                //   → az … --value 'x';touch /tmp/pwned;''
                //
                // which runs when the generated script is run. Verified against
                // the shipped converter with a stub `az` on PATH.
                "az keyvault secret set --vault-name {} --name {} --value {}\n",
                crate::core::shell::word(&self.vault_name),
                crate::core::shell::word(&secret_name),
                crate::formats::quoting::shell_single(&value)
            ));
        }

        Ok(output)
    }

    fn name(&self) -> &str {
        "azure-keyvault"
    }

    fn description(&self) -> &str {
        "Azure Key Vault (az cli commands)"
    }

    /// Key Vault names take letters, digits and `-` only.
    fn target_name(&self, key: &str) -> String {
        key.replace('_', "-")
    }

    /// And they ignore case: `A-B` and `a-b` are one secret, so `az keyvault
    /// secret set` — an upsert — would overwrite the first in silence (I-249).
    fn names_ignore_case(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_converter() {
        let mut vars = IndexMap::new();
        vars.insert("API_KEY".to_string(), "secret123".to_string());

        let converter = AzureKeyVaultConverter::default();
        let result = converter
            .convert(&vars, &ConvertOptions::default())
            .unwrap();

        assert!(result.contains("az keyvault secret set"));
        assert!(result.contains("API-KEY"));
    }
}
