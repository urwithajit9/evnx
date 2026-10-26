//! doppler.rs — Migrate secrets to Doppler
//!
//! Prints Doppler CLI commands (`doppler secrets set …`) for you to run.
//!
//! ⚠️ It used to run `doppler --version` first and refuse without the CLI —
//! on a real run only, never on a dry run — so commands could not be
//! generated on a machine that would not run them (I-110). Printing needs no
//! tool; whether one is installed is the reader's business.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to doppler --project myapp --doppler-config dev
//! evnx migrate --to doppler --dry-run
//! ```
//!
//! Install Doppler CLI: <https://docs.doppler.com/docs/cli>

use crate::core::shell;
use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

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
        let out = opts.out;
        out.status(format!(
            "\n{} Doppler migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!("{} Requires Doppler CLI", "·".cyan()));
        out.status("  Install: https://docs.doppler.com/docs/cli");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for Doppler:".bold()
            ));
            if let Some(p) = &self.project {
                out.result(format!("  Project : {}", p));
            }
            if let Some(c) = &self.config {
                out.result(format!("  Config  : {}", c));
            }
            out.result(format!("  Secrets : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Build optional project/config flags
        let project_flags = match (&self.project, &self.config) {
            (Some(p), Some(c)) => {
                format!("--project {} --config {} ", shell::word(p), shell::word(c))
            }
            (Some(p), None) => format!("--project {} ", shell::word(p)),
            (None, Some(c)) => format!("--config {} ", shell::word(c)),
            (None, None) => String::new(),
        };

        out.status(format!("\n{}", "Upload secrets with:".bold()));
        out.status("");

        for (key, value) in secrets {
            out.command(format!(
                "doppler secrets set {}{}",
                project_flags,
                shell::assignment(key, value)
            ));
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Run the printed `doppler secrets set` commands.");
        out.status("  2. Replace `.env` references in your app with `doppler run --`.");
        out.status("  3. Add `doppler run -- <your-command>` to your CI/CD pipeline.");
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
