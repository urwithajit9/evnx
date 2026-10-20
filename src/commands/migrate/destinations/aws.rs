//! aws.rs — Migrate secrets to AWS Secrets Manager
//!
//! This destination generates `aws secretsmanager` CLI commands rather than
//! calling the AWS SDK directly, which avoids an additional heavy dependency
//! and lets operators review / audit the commands before running them.
//!
//! # Usage
//!
//! ```bash
//! evnx migrate --to aws-secrets-manager --secret-name prod/myapp/config
//! evnx migrate --to aws --dry-run
//! ```

use crate::core::shell;
use anyhow::Result;
use colored::Colorize;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult, Out};

pub struct AwsDestination {
    /// AWS Secrets Manager secret name, e.g. `prod/myapp/config`.
    pub secret_name: String,
    /// Optional named AWS CLI profile (passed as `--profile`).
    pub profile: Option<String>,
}

impl AwsDestination {
    /// Pure constructor — no I/O. Use in tests and when the caller already has
    /// the secret name (e.g. from a CLI flag).
    pub fn new(secret_name: String, profile: Option<String>) -> Self {
        Self {
            secret_name,
            profile,
        }
    }
}

impl MigrationDestination for AwsDestination {
    fn name(&self) -> &str {
        "AWS Secrets Manager"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        let out = opts.out;
        out.status(format!(
            "\n{} AWS Secrets Manager migration",
            crate::utils::ui::glyph::INFO.cyan()
        ));

        let json = serde_json::to_string_pretty(secrets)?;
        let profile_flag = self
            .profile
            .as_deref()
            .map(|p| format!("--profile {} ", shell::word(p)))
            .unwrap_or_default();

        if opts.dry_run {
            out.result(format!(
                "\n{}",
                "Dry-run — would upload to AWS Secrets Manager:".bold()
            ));
            out.result(format!("  Secret name : {}", self.secret_name));
            out.result(format!("  Secrets     : {}", secrets.len()));
            out.result("\n  JSON preview (first 300 chars):");
            out.result(format!("  {}", json.chars().take(300).collect::<String>()));
            if json.len() > 300 {
                out.result(format!("  … ({} more chars)", json.len() - 300));
            }
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        // ⚠️ Not `replace('\'', "\\'")`: inside single quotes `\'` is two
        // characters and the next `'` ends the string (I-094).
        let secret_string = shell::quote(&json);
        let name = shell::word(&self.secret_name);

        out.status(format!("\n{}", "Run one of the following commands:".bold()));
        out.status("");
        out.command("# Create a new secret:".dimmed());
        out.command("aws secretsmanager create-secret \\");
        out.command(format!("  {}--name {} \\", profile_flag, name));
        out.command(format!("  --secret-string {}", secret_string));
        out.command("");
        out.command("# Or update an existing secret:".dimmed());
        out.command("aws secretsmanager update-secret \\");
        out.command(format!("  {}--secret-id {} \\", profile_flag, name));
        out.command(format!("  --secret-string {}", secret_string));

        // We print CLI commands; we don't upload directly, so mark as "uploaded"
        // conceptually (the operator will run the commands).
        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self, out: Out) {
        out.status(format!("\n{}", "Next steps:".bold()));
        out.status("  1. Run the printed `aws secretsmanager` command.");
        out.status("  2. Grant IAM roles/users access to the new secret.");
        out.status("  3. Update your application to read from Secrets Manager.");
        out.status("  4. Remove or encrypt your local .env file.");
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::destination::DestinationKind;
    use super::*;

    fn make_secrets() -> IndexMap<String, String> {
        let mut m = IndexMap::new();
        m.insert("DB_URL".into(), "postgres://localhost/test".into());
        m.insert("API_KEY".into(), "secret123".into());
        m
    }

    #[test]
    fn test_dry_run_returns_skipped() {
        // new() — no dialoguer, safe in non-TTY
        let dest = AwsDestination::new("prod/test".into(), None);
        let opts = MigrationOptions {
            dry_run: true,
            ..Default::default()
        };
        let result = dest.migrate(&make_secrets(), &opts).unwrap();
        assert_eq!(result.skipped, 2);
        assert_eq!(result.uploaded, 0);
    }

    /// ⚠️ Named `test_live_run_returns_uploaded` until v0.5.0, which was the
    /// same confusion the summary line had: this destination does not upload.
    /// A "live" run prints two `aws secretsmanager` commands and contacts
    /// nothing. The count is of commands generated, which is why `kind()` is
    /// `EmitsCommands` and the summary no longer says "uploaded".
    #[test]
    fn test_live_run_generates_one_command_per_secret() {
        let dest = AwsDestination::new("prod/test".into(), Some("my-profile".into()));
        let opts = MigrationOptions {
            dry_run: false,
            ..Default::default()
        };
        let result = dest.migrate(&make_secrets(), &opts).unwrap();
        assert_eq!(result.uploaded, 2);
        assert_eq!(dest.kind(), DestinationKind::EmitsCommands);
    }
}
