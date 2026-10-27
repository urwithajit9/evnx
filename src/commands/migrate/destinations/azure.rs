//! azure.rs — Migrate secrets to Azure Key Vault  [NEW]
//!
//! Generates `az keyvault secret set` commands for the Azure CLI.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to azure-keyvault --vault-name my-vault
//! evnx migrate --to azure --vault-name my-vault --dry-run
//! ```
//!
//! Install Azure CLI: <https://learn.microsoft.com/cli/azure/install-azure-cli>

use crate::formats::azure::secret_name;
use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

pub struct AzureDestination {
    /// Azure Key Vault name (e.g. `my-prod-vault`).
    pub vault_name: String,
}

impl AzureDestination {
    /// Pure constructor — no I/O. Safe in tests and non-TTY contexts.
    pub fn new(vault_name: String) -> Self {
        Self { vault_name }
    }
}

impl MigrationDestination for AzureDestination {
    fn name(&self) -> &str {
        "Azure Key Vault"
    }

    /// Key Vault names ignore case: `A-B` and `a-b` are one secret, and
    /// `az keyvault secret set` — an upsert — would overwrite the first in
    /// silence (I-104; I-249 in convert).
    fn names_ignore_case(&self) -> bool {
        true
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        let out = opts.out;
        out.status(format!(
            "\n{} Azure Key Vault migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!("{} Requires Azure CLI (`az`)", "·".cyan()));
        out.status("  Install: https://learn.microsoft.com/cli/azure/install-azure-cli");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for Azure Key Vault:".bold()
            ));
            out.result(format!("  Vault   : {}", self.vault_name));
            out.result(format!("  Secrets : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Azure Key Vault secret names must use only alphanumerics and hyphens.
        // Underscores are not allowed — we convert them automatically, by the
        // rule `convert --to azure-keyvault` uses.
        out.status(format!("\n{}", "Run these commands:".bold()));
        out.command(
            "# Note: underscores in keys are converted to hyphens for AKV compatibility".dimmed(),
        );
        out.command("");

        let mut warnings = vec![];

        for (key, value) in secrets {
            let akv_name = secret_name(key);
            if akv_name != *key {
                warnings.push(format!("  {} → {} (underscores → hyphens)", key, akv_name));
            }
            // RM-06: the same script `convert` writes, the value off argv.
            out.command(crate::formats::azure::set_secret_script(
                &self.vault_name,
                &akv_name,
                value,
            ));
        }

        if !warnings.is_empty() {
            out.status("");
            out.status("Key renames applied:".yellow());
            for w in &warnings {
                out.status(w);
            }
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Ensure you are logged in: `az login`.");
        out.status("  2. Run the printed `az keyvault secret set` commands.");
        out.status("  3. Grant your app a Managed Identity with Key Vault Secrets User role.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_underscore_conversion() {
        assert_eq!(secret_name("DB_URL"), "DB-URL");
    }

    #[test]
    fn test_dry_run() {
        // new() — no dialoguer, safe in non-TTY
        let dest = AzureDestination::new("test-vault".into());
        let mut secrets = IndexMap::new();
        secrets.insert("DB_URL".into(), "postgres://".into());
        let opts = MigrationOptions {
            dry_run: true,
            ..Default::default()
        };
        let result = dest.migrate(&secrets, &opts).unwrap();
        assert_eq!(result.skipped, 1);
    }
}
