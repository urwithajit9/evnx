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

/// `key` as a Key Vault secret name: `_` becomes `-`, since names take
/// letters, digits and `-` only. The one rule for `convert --to
/// azure-keyvault` and `migrate --to azure-keyvault`.
pub fn secret_name(key: &str) -> String {
    key.replace('_', "-")
}

/// The commands that store `value` as `name` in `vault`, with the value kept
/// off every command line (RM-06, I-086).
///
/// `--value <secret>` put it in `ps`, in shell history and in a `set -x` log.
/// Now a quoted heredoc writes it to a private temp file — no expansion, and
/// `set -x` traces only the `head` line — and `az` reads it with `--file`.
/// `head -c <bytes>` drops the newline every heredoc ends with, which `az`
/// would otherwise store as part of the secret; only the length is on argv.
/// The subshell's trap removes the file whatever `az` does.
pub fn set_secret_script(vault: &str, name: &str, value: &str) -> String {
    // A delimiter that is not a line of the value, or the heredoc ends early.
    let mut end = String::from("EVNX_VALUE");
    while value.lines().any(|l| l == end) {
        end.push('_');
    }
    format!(
        "# {name}: through a private temp file, not the command line\n\
         (\n\
         umask 077; f=$(mktemp) || exit 1; trap 'rm -f \"$f\"' EXIT\n\
         head -c {len} > \"$f\" <<'{end}'\n\
         {value}\n\
         {end}\n\
         az keyvault secret set --vault-name {vault} --name {quoted} --file \"$f\" --encoding utf-8\n\
         )\n",
        name = name,
        len = value.len(),
        vault = crate::core::shell::word(vault),
        quoted = crate::core::shell::word(name),
    )
}

impl Converter for AzureKeyVaultConverter {
    fn convert(&self, vars: &IndexMap<String, String>, options: &ConvertOptions) -> Result<String> {
        let mut output = String::new();
        output.push_str("#!/bin/bash\n");
        output.push_str("# Upload secrets to Azure Key Vault\n");
        output.push_str(&format!("# Vault: {}\n\n", self.vault_name));

        // One block per secret, a blank line between them.
        let blocks: Vec<String> = options
            .prepare(vars, self)?
            .into_iter()
            .map(|(secret_name, value)| set_secret_script(&self.vault_name, &secret_name, &value))
            .collect();
        output.push_str(&blocks.join("\n"));

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
        secret_name(key)
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
        // RM-06: never on the command line.
        assert!(!result.contains("--value"), "{result}");
        assert!(
            result.contains("--file \"$f\" --encoding utf-8"),
            "{result}"
        );
    }

    /// A value holding the delimiter as a line cannot end the heredoc early.
    #[test]
    fn rm06_the_delimiter_is_never_a_line_of_the_value() {
        let script = set_secret_script("kv", "X", "a\nEVNX_VALUE\nb");
        assert!(script.contains("<<'EVNX_VALUE_'"), "{script}");
    }
}
