//! railway.rs — Migrate secrets to Railway  [NEW]
//!
//! Generates `railway variables set` commands via the Railway CLI.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to railway
//! ```
//!
//! Install Railway CLI: <https://docs.railway.app/develop/cli>

use anyhow::{anyhow, Result};
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct RailwayDestination {
    pub project: Option<String>,
}

impl RailwayDestination {
    pub fn new(project: Option<String>) -> Self {
        Self { project }
    }
}

impl MigrationDestination for RailwayDestination {
    fn name(&self) -> &str {
        "Railway"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Railway migration", "🚂".cyan());
        println!("{} Requires Railway CLI", "ℹ️".cyan());
        println!("  Install: npm install -g @railway/cli");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Railway:".bold());
            if let Some(p) = &self.project {
                println!("  Project : {}", p);
            }
            println!("  Secrets : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        if std::process::Command::new("railway")
            .arg("--version")
            .output()
            .is_err()
        {
            return Err(anyhow!(
                "Railway CLI not found. Install with: npm install -g @railway/cli"
            ));
        }

        let project_flag = self
            .project
            .as_deref()
            .map(|p| format!("--project {} ", p))
            .unwrap_or_default();

        println!("\n{}", "Upload secrets with:".bold());
        println!();

        // Railway CLI supports bulk `variables set KEY=VALUE …`
        print!("railway {}variables set", project_flag);
        for (key, value) in secrets {
            let escaped = value.replace('\'', "'\\''");
            print!(" \\\n  {}='{}'", key, escaped);
        }
        println!();

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Login first: `railway login`.");
        println!("  2. Link project: `railway link`.");
        println!("  3. Run the printed `railway variables set` command.");
        println!("  4. Deploy: `railway up`.");
    }
}
