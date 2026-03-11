//! infisical.rs — Migrate secrets to Infisical
//!
//! Wraps the Infisical CLI (`infisical secrets set …`).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to infisical --project myproject
//! evnx migrate --to infisical --dry-run
//! ```
//!
//! Install Infisical CLI: <https://infisical.com/docs/cli/overview>

use anyhow::{anyhow, Result};
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct InfisicalDestination {
    /// Infisical project slug or ID.
    pub project: Option<String>,
    /// Infisical environment, e.g. `dev`, `staging`, `prod`.
    pub environment: Option<String>,
}

impl InfisicalDestination {
    pub fn new(project: Option<String>, environment: Option<String>) -> Self {
        Self {
            project,
            environment,
        }
    }
}

impl MigrationDestination for InfisicalDestination {
    fn name(&self) -> &str {
        "Infisical"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Infisical migration", "🔒".cyan());
        println!("{} Requires Infisical CLI", "ℹ️".cyan());
        println!("  Install: https://infisical.com/docs/cli/overview");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Infisical:".bold());
            if let Some(p) = &self.project {
                println!("  Project     : {}", p);
            }
            if let Some(e) = &self.environment {
                println!("  Environment : {}", e);
            }
            println!("  Secrets     : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Check CLI
        if std::process::Command::new("infisical")
            .arg("--version")
            .output()
            .is_err()
        {
            return Err(anyhow!(
                "Infisical CLI not found. Install from https://infisical.com/docs/cli/overview"
            ));
        }

        // Build optional flags
        let project_flag = self
            .project
            .as_deref()
            .map(|p| format!("--projectId {} ", p))
            .unwrap_or_default();

        let env_flag = self
            .environment
            .as_deref()
            .map(|e| format!("--env {} ", e))
            .unwrap_or_default();

        println!("\n{}", "Upload secrets with:".bold());
        println!();
        for (key, value) in secrets {
            let escaped = value.replace('\'', "'\\''");
            println!(
                "infisical secrets set {}{}{}='{}'",
                project_flag, env_flag, key, escaped
            );
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Authenticate with `infisical login`.");
        println!("  2. Run the printed `infisical secrets set` commands.");
        println!("  3. Replace your app's .env with `infisical run -- <command>`.");
    }
}
