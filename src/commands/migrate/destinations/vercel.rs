//! vercel.rs — Migrate secrets to Vercel environment variables  [NEW]
//!
//! Generates `vercel env add` commands, one per variable, each value on stdin:
//!
//! ```text
//! printf '%s' 'VALUE' | vercel env add NAME production [--project P] --force --yes
//! ```
//!
//! ⚠️ Checked against vercel 64.7.0 (`packages/cli/src/commands/env/`), I-268:
//!
//! - `--project`, not `--project-id`. There is no `--project-id`, so every
//!   command evnx printed with `--vercel-project` was a usage error. Without
//!   the flag vercel uses the linked project (`vercel link`,
//!   `VERCEL_PROJECT_ID`).
//! - `--force`: an existing variable is an error without it, so a re-run —
//!   the normal way to change a value — failed. Every other destination evnx
//!   prints for replaces a value too.
//! - `--yes`: `env add` can still ask how to store the value ("Store this
//!   value as?"); the printed command should run without a question.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to vercel --vercel-project my-project
//! ```

use crate::core::shell;
use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

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

    /// `vercel env add NAME ENVIRONMENT [--project P] --force --yes`.
    fn add_command(&self, key: &str) -> String {
        let mut words = vec![
            "vercel".to_string(),
            "env".into(),
            "add".into(),
            shell::word(key),
            shell::word(&self.environment),
        ];
        if let Some(p) = &self.project {
            words.push("--project".into());
            words.push(shell::word(p));
        }
        words.push("--force".into());
        words.push("--yes".into());
        words.join(" ")
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
        let out = opts.out;
        out.status(format!(
            "\n{} Vercel environment variable migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!(
            "{} Requires Vercel CLI",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status("  Install: npm install -g vercel");

        if opts.dry_run {
            out.result(format!("\n{}", "Dry-run — would upload to Vercel:".bold()));
            if let Some(p) = &self.project {
                out.result(format!("  Project     : {}", p));
            }
            out.result(format!("  Environment : {}", self.environment));
            out.result(format!("  Secrets     : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        out.status(format!(
            "\n{}",
            "Run these commands (each value goes to vercel on stdin):".bold()
        ));
        out.status("");

        for (key, value) in secrets {
            // ⚠️ `printf '%s'`, not `echo "…"`: double quotes expanded `$5`
            // and ran `$(…)` (I-101), and `echo` adds a newline.
            out.command(format!(
                "printf '%s' {} | {}",
                shell::quote(value),
                self.add_command(key)
            ));
        }

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Run `vercel env pull .env.local` to verify secrets were set.");
        out.status("  2. Redeploy your project: `vercel --prod`.");
        out.status("  3. Remove .env from your working directory.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i268_project_flag_is_project() {
        let with = VercelDestination::new(Some("prj_1".into()));
        assert_eq!(
            with.add_command("A"),
            "vercel env add A production --project prj_1 --force --yes"
        );
        let without = VercelDestination::new(None);
        assert_eq!(
            without.add_command("A"),
            "vercel env add A production --force --yes"
        );
    }
}
