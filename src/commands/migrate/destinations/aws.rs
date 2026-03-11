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

use anyhow::Result;
use colored::Colorize;
use dialoguer::Input;
use indexmap::IndexMap;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

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

    /// Interactive constructor — prompts for the secret name only when
    /// `secret_name` is `None`. Called exclusively from `destinations::get()`.
    pub fn interactive(secret_name: Option<String>, profile: Option<String>) -> Result<Self> {
        let name = match secret_name {
            Some(n) => n,
            None => Input::new()
                .with_prompt("Secret name (e.g. prod/myapp/config)")
                .with_initial_text("prod/myapp/config")
                .interact_text()?,
        };
        Ok(Self::new(name, profile))
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
        println!("\n{} AWS Secrets Manager migration", "☁️".cyan());

        let json = serde_json::to_string_pretty(secrets)?;
        let profile_flag = self
            .profile
            .as_deref()
            .map(|p| format!("--profile {} ", p))
            .unwrap_or_default();

        if opts.dry_run {
            println!(
                "\n{}",
                "Dry-run — would upload to AWS Secrets Manager:".bold()
            );
            println!("  Secret name : {}", self.secret_name);
            println!("  Secrets     : {}", secrets.len());
            println!("\n  JSON preview (first 300 chars):");
            println!("  {}", &json.chars().take(300).collect::<String>());
            if json.len() > 300 {
                println!("  … ({} more chars)", json.len() - 300);
            }
            return Ok(MigrationResult {
                skipped: secrets.len(),
                ..Default::default()
            });
        }

        let escaped = json.replace('\'', "\\'");

        println!("\n{}", "Run one of the following commands:".bold());
        println!();
        println!("{}", "# Create a new secret:".dimmed());
        println!("aws secretsmanager create-secret \\");
        println!("  {}--name {} \\", profile_flag, self.secret_name);
        println!("  --secret-string '{}'", escaped);
        println!();
        println!("{}", "# Or update an existing secret:".dimmed());
        println!("aws secretsmanager update-secret \\");
        println!("  {}--secret-id {} \\", profile_flag, self.secret_name);
        println!("  --secret-string '{}'", escaped);

        // We print CLI commands; we don't upload directly, so mark as "uploaded"
        // conceptually (the operator will run the commands).
        Ok(MigrationResult {
            uploaded: secrets.len(),
            ..Default::default()
        })
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Run the printed `aws secretsmanager` command.");
        println!("  2. Grant IAM roles/users access to the new secret.");
        println!("  3. Update your application to read from Secrets Manager.");
        println!("  4. Remove or encrypt your local .env file.");
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn test_live_run_returns_uploaded() {
        let dest = AwsDestination::new("prod/test".into(), Some("my-profile".into()));
        let opts = MigrationOptions {
            dry_run: false,
            ..Default::default()
        };
        let result = dest.migrate(&make_secrets(), &opts).unwrap();
        assert_eq!(result.uploaded, 2);
    }
}
