//! gcp.rs — Migrate secrets to Google Cloud Secret Manager
//!
//! Prints the script `convert --to gcp-secrets` prints: an `upsert_secret`
//! function, then one `printf '%s' 'VALUE' | upsert_secret NAME` per secret
//! (`formats::gcp`, one definition for both commands).
//!
//! ⚠️ It printed `echo -n "${NAME}" | gcloud secrets create NAME …` until
//! 0.10.10 (I-100): the value came from whatever the user's shell held under
//! that name — usually nothing, so an empty secret — `create` failed for a
//! secret that existed, and `--project` was accepted and dropped.
//!
//! ⚠️ **Names are convert's** (I-098): `DB_URL` becomes `db-url`, through
//! [`secret_id`](crate::formats::gcp::secret_id), the function
//! `convert --to gcp-secrets` renames with. 0.12 kept the `.env` name
//! (`DB_URL`), so a project migrated then gets new secrets beside the old ones.
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

    /// [`secret_id`], the rename `convert --to gcp-secrets` applies (I-098).
    fn target_name(&self, key: &str) -> String {
        crate::formats::gcp::secret_id(key)
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

    /// I-098: `prepare_with` hands `migrate` the names `convert` prints.
    #[test]
    fn t_i098_migrate_names_match_convert() {
        use crate::core::converter::{ConvertOptions, Converter};
        let mut vars = IndexMap::new();
        vars.insert("DB_URL".to_string(), "postgres://h/db".to_string());
        vars.insert("API_KEY".to_string(), "abc".to_string());
        let options = ConvertOptions::default();

        let dest = GcpDestination::new("p".into());
        let dest: &dyn MigrationDestination = &dest;
        let migrated: Vec<String> = options
            .prepare_with(&vars, dest, "")
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let converted: Vec<String> = options
            .prepare(&vars, &crate::formats::gcp::GcpSecretConverter::default())
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(migrated, vec!["db-url", "api-key"]);
        assert_eq!(migrated, converted);

        // The printed script uses those names.
        let secrets: IndexMap<String, String> = options
            .prepare_with(&vars, dest, "")
            .unwrap()
            .into_iter()
            .collect();
        let script = crate::formats::gcp::GcpSecretConverter::default()
            .convert(&vars, &options)
            .unwrap();
        for name in secrets.keys() {
            assert!(script.contains(&format!("| upsert_secret {name}\n")));
        }
    }

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
