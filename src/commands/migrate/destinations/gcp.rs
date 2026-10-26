//! gcp.rs — Migrate secrets to Google Cloud Secret Manager
//!
//! Prints the script `convert --to gcp-secrets` prints: an `upsert_secret`
//! function, then one `printf '%s' 'VALUE' | upsert_secret NAME` per secret
//! (`formats::gcp`, one definition for both commands).
//!
//! ⚠️ It printed `echo -n "${NAME}" | gcloud secrets create NAME …` until
//! 0.10.10 (I-100): the value came from whatever the user's shell held under
//! that name — usually nothing, so an empty secret — `create` failed for a
//! secret that existed, and `--project` was accepted and dropped. Names stay
//! as they are in the `.env`; convert's lowercase-hyphen rename waits for the
//! shared catalogue (I-098).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to gcp-secret-manager --project my-project
//! ```
//!
//! Install the Google Cloud CLI: <https://cloud.google.com/sdk/docs/install>

use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};
use crate::formats::gcp::{upsert_function, upsert_line, UPSERT_COMMENT};

pub struct GcpDestination {
    /// GCP project ID, printed as `--project=` into every command (DEC-MG2).
    pub project: String,
}

impl GcpDestination {
    /// Pure constructor — no I/O.
    pub fn new(project: String) -> Self {
        Self { project }
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
        let out = opts.out;
        out.status(format!(
            "\n{} GCP Secret Manager migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!(
            "{} Requires Google Cloud CLI (`gcloud`)",
            "·".cyan()
        ));
        out.status("  Install: https://cloud.google.com/sdk/docs/install");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for GCP Secret Manager:".bold()
            ));
            out.result(format!("  Project : {}", self.project));
            out.result(format!("  Secrets : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        out.status(format!(
            "\n{}",
            "Run these commands (each value goes to gcloud on stdin):".bold()
        ));
        out.status("");
        out.command(UPSERT_COMMENT.dimmed());
        out.command(upsert_function(&self.project).trim_end());
        out.command("");
        for (key, value) in secrets {
            out.command(upsert_line(key, value));
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Run the printed commands (`gcloud auth login` first).");
        out.status("  2. Grant service accounts `secretmanager.secretAccessor` on each secret.");
        out.status("  3. Update your app to call `secretmanager.googleapis.com`.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i100_dry_run_counts_and_prints_no_value() {
        let dest = GcpDestination::new("my-proj".into());
        let mut secrets = IndexMap::new();
        secrets.insert("A".into(), "1".into());
        let opts = MigrationOptions {
            dry_run: true,
            ..Default::default()
        };
        let result = dest.migrate(&secrets, &opts).unwrap();
        assert_eq!(result.skipped, 1);
    }
}
