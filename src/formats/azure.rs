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

impl Default for AzureKeyVaultConverter {
    fn default() -> Self {
        Self {
            vault_name: "my-keyvault".to_string(),
        }
    }
}

impl Converter for AzureKeyVaultConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let filtered = options.filter_vars(vars);

        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to Azure Key Vault\n");
        output.push_str(&format!("# Vault: {}\n\n", self.vault_name));

        for (k, v) in filtered.iter() {
            let key = options.transform_key(k);
            let value = options.transform_value(v);

            // Azure Key Vault naming: alphanumeric and hyphens only
            let secret_name = key.replace('_', "-");

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
                self.vault_name,
                secret_name,
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
