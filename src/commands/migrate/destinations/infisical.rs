//! infisical.rs — Migrate secrets to Infisical
//!
//! Prints Infisical CLI commands (`infisical secrets set …`) for you to run.
//! No `infisical --version` check first: printing needs no tool (I-110).
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to infisical --project myproject
//! evnx migrate --to infisical --dry-run
//! ```
//!
//! Install Infisical CLI: <https://infisical.com/docs/cli/overview>

use crate::core::shell;
use anyhow::{anyhow, Result};
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

pub struct InfisicalDestination {
    /// Infisical project slug or ID.
    pub project: Option<String>,
    /// Infisical environment, e.g. `dev`, `staging`, `prod`.
    pub environment: Option<String>,
}

impl InfisicalDestination {
    pub fn new(project: Option<String>, environment: Option<String>) -> Self {
        Self {
            project,
            environment,
        }
    }
}

/// The values `infisical secrets set KEY=VALUE` would not store as written
/// (I-270, DEC-MG4), as `(key, reason)`. Never the value itself.
///
/// ⚠️ Checked against the Infisical CLI v0.44.3 (`packages/cmd/secrets.go`).
/// Signed in with `infisical login`, a value starting with `@` is a **file
/// path**: the CLI reads that file and stores its contents. With a service
/// token or machine identity the same argument is sent as written. `\@`
/// escapes it for the first and is stored literally by the second, so no
/// spelling is right for both — and a `.env` value such as `@/home/me/.ssh/id_rsa`
/// would upload a private key. The CLI also rejects an empty value.
pub fn values_it_would_misread(secrets: &IndexMap<String, String>) -> Vec<(&str, &'static str)> {
    secrets
        .iter()
        .filter_map(|(k, v)| {
            if v.is_empty() {
                Some((k.as_str(), "empty — infisical rejects an empty value"))
            } else if v.starts_with('@') {
                Some((
                    k.as_str(),
                    "starts with `@` — read as a file path under `infisical login`, \
                     sent as written with a token",
                ))
            } else {
                None
            }
        })
        .collect()
}

impl MigrationDestination for InfisicalDestination {
    fn name(&self) -> &str {
        "Infisical"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        let refused = values_it_would_misread(secrets);
        if !refused.is_empty() {
            let lines: Vec<String> = refused
                .iter()
                .map(|(k, why)| format!("  {k}: {why}"))
                .collect();
            return Err(anyhow!(
                "these values cannot be passed to `infisical secrets set` as they are:\n{}\n\n\
                 Set them in Infisical directly, or leave them out with --exclude. \
                 Nothing was printed.",
                lines.join("\n")
            ));
        }

        let out = opts.out;
        out.status(format!(
            "\n{} Infisical migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));
        out.status(format!("{} Requires Infisical CLI", "·".cyan()));
        out.status("  Install: https://infisical.com/docs/cli/overview");

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would print commands for Infisical:".bold()
            ));
            if let Some(p) = &self.project {
                out.result(format!("  Project     : {}", p));
            }
            if let Some(e) = &self.environment {
                out.result(format!("  Environment : {}", e));
            }
            out.result(format!("  Secrets     : {}", secrets.len()));
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // Build optional flags
        let project_flag = self
            .project
            .as_deref()
            .map(|p| format!("--projectId {} ", shell::word(p)))
            .unwrap_or_default();

        let env_flag = self
            .environment
            .as_deref()
            .map(|e| format!("--env {} ", shell::word(e)))
            .unwrap_or_default();

        out.status(format!("\n{}", "Upload secrets with:".bold()));
        out.status("");
        for (key, value) in secrets {
            out.command(format!(
                "infisical secrets set {}{}{}",
                project_flag,
                env_flag,
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
        out.status("  1. Authenticate with `infisical login`.");
        out.status("  2. Run the printed `infisical secrets set` commands.");
        out.status("  3. Replace your app's .env with `infisical run -- <command>`.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i270_names_the_values_and_not_their_contents() {
        let mut s = IndexMap::new();
        s.insert("FILE".to_string(), "@/etc/passwd".to_string());
        s.insert("EMPTY".to_string(), String::new());
        s.insert("MID".to_string(), "a@b".to_string());
        let found = values_it_would_misread(&s);
        let keys: Vec<&str> = found.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["FILE", "EMPTY"]);
        assert!(found.iter().all(|(_, why)| !why.contains("/etc/passwd")));
    }
}
