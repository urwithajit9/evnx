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

use anyhow::Result;
use colored::Colorize;
use dialoguer::Input;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct AzureDestination {
    /// Azure Key Vault name (e.g. `my-prod-vault`).
    pub vault_name: String,
}

impl AzureDestination {
    /// Pure constructor — no I/O. Safe in tests and non-TTY contexts.
    pub fn new(vault_name: String) -> Self {
        Self { vault_name }
    }

    /// Interactive constructor — prompts only when `vault_name` is `None`.
    pub fn interactive(vault_name: Option<String>) -> Result<Self> {
        let name = match vault_name {
            Some(n) => n,
            None => Input::new()
                .with_prompt("Azure Key Vault name")
                .interact_text()?,
        };
        Ok(Self::new(name))
    }
}

impl MigrationDestination for AzureDestination {
    fn name(&self) -> &str {
        "Azure Key Vault"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Azure Key Vault migration", "☁️".cyan());
        println!("{} Requires Azure CLI (`az`)", "ℹ️".cyan());
        println!("  Install: https://learn.microsoft.com/cli/azure/install-azure-cli");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Azure Key Vault:".bold());
            println!("  Vault   : {}", self.vault_name);
            println!("  Secrets : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Azure Key Vault secret names must use only alphanumerics and hyphens.
        // Underscores are not allowed — we convert them automatically.
        println!("\n{}", "Run these commands:".bold());
        println!(
            "{}",
            "# Note: underscores in keys are converted to hyphens for AKV compatibility".dimmed()
        );
        println!();

        let mut warnings = vec![];

        for (key, value) in secrets {
            let akv_name = key.replace('_', "-");
            if akv_name != *key {
                warnings.push(format!("  {} → {} (underscores → hyphens)", key, akv_name));
            }
            let escaped = value.replace('"', "\\\"");
            println!(
                "az keyvault secret set --vault-name {} --name {} --value \"{}\"",
                self.vault_name, akv_name, escaped
            );
        }

        if !warnings.is_empty() {
            println!();
            println!("{}", "Key renames applied:".yellow());
            for w in &warnings {
                println!("{}", w);
            }
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Ensure you are logged in: `az login`.");
        println!("  2. Run the printed `az keyvault secret set` commands.");
        println!("  3. Grant your app a Managed Identity with Key Vault Secrets User role.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_underscore_conversion() {
        let key = "DB_URL".to_string();
        let akv_name = key.replace('_', "-");
        assert_eq!(akv_name, "DB-URL");
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
