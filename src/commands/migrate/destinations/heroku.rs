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

use anyhow::Result;
use colored::Colorize;
use dialoguer::Input;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

pub struct HerokuDestination {
    pub app: String,
}

impl HerokuDestination {
    /// Pure constructor — no I/O. Safe in tests and non-TTY contexts.
    pub fn new(app: String) -> Self {
        Self { app }
    }

    /// Interactive constructor — prompts only when `app` is `None`.
    pub fn interactive(app: Option<String>) -> Result<Self> {
        let app_name = match app {
            Some(a) => a,
            None => Input::new()
                .with_prompt("Heroku app name")
                .interact_text()?,
        };
        Ok(Self::new(app_name))
    }
}

impl MigrationDestination for HerokuDestination {
    fn name(&self) -> &str {
        "Heroku"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Heroku Config Vars migration", "🟣".cyan());
        println!("{} Requires Heroku CLI", "ℹ️".cyan());
        println!("  Install: https://devcenter.heroku.com/articles/heroku-cli");

        if opts.dry_run {
            println!("\n{}", "Dry-run — would upload to Heroku:".bold());
            println!("  App     : {}", self.app);
            println!("  Secrets : {}", secrets.len());
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Build a single bulk `heroku config:set` command — much faster than
        // individual calls and only triggers one dyno restart.
        println!(
            "\n{}",
            "Run this command (single bulk set = one restart):".bold()
        );
        println!();
        print!("heroku config:set --app {}", self.app);

        for (key, value) in secrets {
            // Shell-escape the value with single quotes
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
        println!("  1. Verify with `heroku config --app {}`.", self.app);
        println!("  2. The dyno restarts automatically after `config:set`.");
        println!("  3. Remove your local .env file.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dry_run() {
        // new() — no dialoguer, safe in non-TTY
        let dest = HerokuDestination::new("my-test-app".into());
        let mut secrets = IndexMap::new();
        secrets.insert("SECRET".into(), "value".into());
        let opts = MigrationOptions {
            dry_run: true,
            ..Default::default()
        };
        let result = dest.migrate(&secrets, &opts).unwrap();
        assert_eq!(result.skipped, 1);
    }
}
