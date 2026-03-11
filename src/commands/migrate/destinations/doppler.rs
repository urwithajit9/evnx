//! doppler.rs — Migrate secrets to Doppler
//!
//! Wraps the Doppler CLI (`doppler secrets set …`).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to doppler --project myapp --config dev
//! evnx migrate --to doppler --dry-run
//! ```
//!
//! Install Doppler CLI: <https://docs.doppler.com/docs/cli>

use anyhow::{anyhow, Result};
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct DopplerDestination {
    /// Doppler project slug (optional — falls back to CLI's current project).
    pub project: Option<String>,
    /// Doppler config name, e.g. `dev`, `staging`, `prd`.
    pub config: Option<String>,
}

impl DopplerDestination {
    pub fn new(project: Option<String>, config: Option<String>) -> Self {
        Self { project, config }
    }
}

impl MigrationDestination for DopplerDestination {
    fn name(&self) -> &str {
        "Doppler"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Doppler migration", "🔐".cyan());
        println!("{} Requires Doppler CLI", "ℹ️".cyan());
        println!("  Install: https://docs.doppler.com/docs/cli");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Doppler:".bold());
            if let Some(p) = &self.project {
                println!("  Project : {}", p);
            }
            if let Some(c) = &self.config {
                println!("  Config  : {}", c);
            }
            println!("  Secrets : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Check CLI is available
        if std::process::Command::new("doppler")
            .arg("--version")
            .output()
            .is_err()
        {
            return Err(anyhow!(
                "Doppler CLI not found. Install from https://docs.doppler.com/docs/cli"
            ));
        }

        // Build optional project/config flags
        let project_flags = match (&self.project, &self.config) {
            (Some(p), Some(c)) => format!("--project {} --config {} ", p, c),
            (Some(p), None) => format!("--project {} ", p),
            (None, Some(c)) => format!("--config {} ", c),
            (None, None) => String::new(),
        };

        println!("\n{}", "Upload secrets with:".bold());
        println!();

        for (key, value) in secrets {
            let escaped = value.replace('\'', "'\\''");
            println!("doppler secrets set {}{}='{}'", project_flags, key, escaped);
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Run the printed `doppler secrets set` commands.");
        println!("  2. Replace `.env` references in your app with `doppler run --`.");
        println!("  3. Add `doppler run -- <your-command>` to your CI/CD pipeline.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dry_run() {
        let dest = DopplerDestination::new(Some("myapp".into()), Some("dev".into()));
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
