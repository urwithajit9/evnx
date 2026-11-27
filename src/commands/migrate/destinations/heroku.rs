//! heroku.rs — Migrate secrets to Heroku Config Vars  [NEW]
//!
//! Generates a single `heroku config:set` command (bulk set is far faster
//! than setting vars one-at-a-time).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to heroku --heroku-app my-heroku-app
//! ```

use crate::core::secret::Vars;
use crate::core::shell;
use anyhow::Result;
use colored::Colorize;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

pub struct HerokuDestination {
    pub app: String,
}

impl HerokuDestination {
    /// Pure constructor — no I/O. Safe in tests and non-TTY contexts.
    pub fn new(app: String) -> Self {
        Self { app }
    }
}

impl MigrationDestination for HerokuDestination {
    fn name(&self) -> &str {
        "Heroku"
    }

    fn migrate(&self, secrets: &Vars, opts: &MigrationOptions) -> Result<MigrationResult> {
        let out = opts.out;
        out.status(format!(
            "\n{} Heroku Config Vars migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!("{} Requires Heroku CLI", "·".cyan()));
        out.status("  Install: https://devcenter.heroku.com/articles/heroku-cli");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for Heroku:".bold()
            ));
            out.result(format!("  App     : {}", self.app));
            out.result(format!("  Secrets : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Build a single bulk `heroku config:set` command — much faster than
        // individual calls and only triggers one dyno restart.
        out.heading(format!(
            "\n{}",
            "Run this command (single bulk set = one restart):".bold()
        ));
        out.heading("");
        let mut command = format!("heroku config:set --app {}", shell::word(&self.app));
        for (key, value) in secrets {
            command.push_str(&format!(
                " \\\n  {}",
                shell::assignment(key, value.expose())
            ));
        }
        out.command(command);
        out.warn_values_in_argv();

        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.next_steps(&[
            format!(
                "Verify with `heroku config --app {}`.",
                shell::word(&self.app)
            ),
            "The dyno restarts automatically after `config:set`.".into(),
            "Remove your local .env file.".into(),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::secret::Vars;

    #[test]
    fn test_dry_run() {
        // new() — no dialoguer, safe in non-TTY
        let dest = HerokuDestination::new("my-test-app".into());
        let mut secrets = Vars::new();
        secrets.insert("SECRET".into(), "value".into());
        let opts = MigrationOptions {
            dry_run: true,
            ..Default::default()
        };
        let result = dest.migrate(&secrets, &opts).unwrap();
        assert_eq!(result.skipped, 1);
    }
}
