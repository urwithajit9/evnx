//! vercel.rs — Migrate secrets to Vercel environment variables  [NEW]
//!
//! Generates `vercel env add` commands.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to vercel --vercel-project my-project
//! ```

use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct VercelDestination {
    pub project: Option<String>,
    /// `development`, `preview`, or `production` (default: `production`).
    pub environment: String,
}

impl VercelDestination {
    pub fn new(project: Option<String>) -> Self {
        Self {
            project,
            environment: "production".into(),
        }
    }
}

impl MigrationDestination for VercelDestination {
    fn name(&self) -> &str {
        "Vercel"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Vercel environment variable migration", "▲".cyan());
        println!("{} Requires Vercel CLI", "ℹ️".cyan());
        println!("  Install: npm install -g vercel");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Vercel:".bold());
            if let Some(p) = &self.project {
                println!("  Project     : {}", p);
            }
            println!("  Environment : {}", self.environment);
            println!("  Secrets     : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        let project_flag = self
            .project
            .as_deref()
            .map(|p| format!("--project-id {} ", p))
            .unwrap_or_default();

        println!(
            "\n{}",
            "Run these commands (uses heredoc for non-interactive input):".bold()
        );
        println!();

        for (key, value) in secrets {
            let escaped = value.replace('"', "\\\"");
            println!(
                "echo \"{}\" | vercel env add {} {} {}",
                escaped, key, self.environment, project_flag
            );
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Run `vercel env pull .env.local` to verify secrets were set.");
        println!("  2. Redeploy your project: `vercel --prod`.");
        println!("  3. Remove .env from your working directory.");
    }
}
