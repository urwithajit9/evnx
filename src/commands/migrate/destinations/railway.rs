//! railway.rs — Migrate secrets to Railway  [NEW]
//!
//! Prints one Railway CLI command that sets every variable:
//!
//! ```text
//! railway variable set [--project P --environment E] [--service S] \
//!   KEY='value' \
//!   KEY2='value2'
//! ```
//!
//! ⚠️ It printed `railway --project P variables set …` until 0.10.10 (I-269).
//! The root `railway` command takes no arguments (Railway CLI v5.71.0,
//! `src/macros.rs`), so that did not parse; `--project` belongs to
//! `variable set`, and with it Railway requires `--environment` (DEC-MG1).
//! Without `--railway-project` the command is for a directory linked with
//! `railway link`. No `railway --version` check first: printing needs no
//! tool (I-110).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to railway
//! evnx migrate --to railway --railway-project <ID> --railway-environment production
//! ```
//!
//! Install Railway CLI: <https://docs.railway.com/cli>

use crate::core::shell;
use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

pub struct RailwayDestination {
    pub project: Option<String>,
    /// Required with `project` (the registry refuses one without the other).
    pub environment: Option<String>,
    /// Optional: the linked service, or the project's only one, otherwise.
    pub service: Option<String>,
}

impl RailwayDestination {
    pub fn new(
        project: Option<String>,
        environment: Option<String>,
        service: Option<String>,
    ) -> Self {
        Self {
            project,
            environment,
            service,
        }
    }

    /// `railway variable set` and its flags, without the assignments.
    fn set_command(&self) -> String {
        let mut words = vec!["railway".to_string(), "variable".into(), "set".into()];
        for (flag, value) in [
            ("--project", &self.project),
            ("--environment", &self.environment),
            ("--service", &self.service),
        ] {
            if let Some(v) = value {
                words.push(flag.into());
                words.push(shell::word(v));
            }
        }
        words.join(" ")
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
        let out = opts.out;
        out.status(format!(
            "\n{} Railway migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!("{} Requires Railway CLI", "·".cyan()));
        out.status("  Install: npm install -g @railway/cli");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for Railway:".bold()
            ));
            if let Some(p) = &self.project {
                out.result(format!("  Project     : {}", p));
            }
            if let Some(e) = &self.environment {
                out.result(format!("  Environment : {}", e));
            }
            if let Some(s) = &self.service {
                out.result(format!("  Service     : {}", s));
            }
            out.result(format!("  Secrets     : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        out.status(format!("\n{}", "Upload secrets with:".bold()));
        out.status("");

        // One call sets every variable, and triggers one deploy.
        let mut command = self.set_command();
        for (key, value) in secrets {
            command.push_str(&format!(" \\\n  {}", shell::assignment(key, value)));
        }
        out.command(command);

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Login first: `railway login`.");
        if self.project.is_none() {
            out.status("  2. Link the project here: `railway link`.");
        } else {
            out.status("  2. The command names the project; no `railway link` needed.");
        }
        out.status("  3. Run the printed `railway variable set` command.");
        out.status("  4. Setting variables redeploys the service.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i269_flags_belong_to_variable_set() {
        let linked = RailwayDestination::new(None, None, None);
        assert_eq!(linked.set_command(), "railway variable set");
        let named = RailwayDestination::new(
            Some("p-1".into()),
            Some("production".into()),
            Some("web".into()),
        );
        assert_eq!(
            named.set_command(),
            "railway variable set --project p-1 --environment production --service web"
        );
    }
}
