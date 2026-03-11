//! gcp.rs — Migrate secrets to Google Cloud Secret Manager

use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct GcpDestination;

impl Default for GcpDestination {
    fn default() -> Self {
        Self
    }
}

impl GcpDestination {
    pub fn new() -> Self {
        Self {}
    }
}

impl MigrationDestination for GcpDestination {
    fn name(&self) -> &str {
        "GCP Secret Manager"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} GCP Secret Manager migration", "☁️".cyan());

        if opts.dry_run {
            println!(
                "\n{}",
                "Dry-run — would upload to GCP Secret Manager:".bold()
            );
            println!("  Secrets : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        println!(
            "{}",
            "Tip: use `evnx convert --to gcp-secrets > upload.sh && bash upload.sh`".dimmed()
        );
        println!("\n{}", "Or run per-secret:".bold());
        println!();

        for (key, _) in secrets {
            println!("echo -n \"${{{}}}\" | \\", key);
            println!("  gcloud secrets create {} \\", key);
            println!("    --data-file=- --replication-policy=automatic");
            println!();
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Grant service accounts `secretmanager.secretAccessor` on each secret.");
        println!("  2. Update your app to call `secretmanager.googleapis.com`.");
    }
}
