//! commands/migrate/destinations/mod.rs
//!
//! Central registry for all migration destinations.
//!
//! # Adding a new destination
//!
//! 1. Create `src/commands/migrate/destinations/myplatform.rs`
//!    implementing `MigrationDestination`.
//! 2. Add `pub mod myplatform;` below.
//! 3. Add a match arm in `get_destination()`.
//! 4. Add the slug + label to `available_destinations()`.
//!
//! No other files need structural changes.

pub mod aws;
pub mod azure;
pub mod doppler;
pub mod gcp;
pub mod heroku;
pub mod infisical;
pub mod railway;
pub mod vercel;

// github.rs imports reqwest, which is only present when the `migrate` feature
// is enabled.  Gating the whole module here means Rust never tries to compile
// it (and resolve reqwest) in a default build.
#[cfg(feature = "migrate")]
pub mod github;

use anyhow::{anyhow, Result};

// Re-export the trait so callers can write `destinations::MigrationDestination`
// without having to reach back up two levels to `migrate::destination`.
pub use crate::commands::migrate::destination::MigrationDestination;

use crate::commands::migrate::MigrateArgs;

/// Resolve a destination slug to a boxed `MigrationDestination`.
///
/// Called once per `evnx migrate` run from `migrate/mod.rs`.
pub fn get_destination(name: &str, args: &MigrateArgs) -> Result<Box<dyn MigrationDestination>> {
    match name {
        // ── Cloud CI / secret managers ────────────────────────────────────
        "github-actions" | "github" => {
            #[cfg(feature = "migrate")]
            {
                let dest = github::GitHubDestination::interactive(
                    args.repo.clone(),
                    args.github_token.clone(),
                )?;
                Ok(Box::new(dest))
            }
            #[cfg(not(feature = "migrate"))]
            {
                use colored::Colorize;
                eprintln!(
                    "{} GitHub Actions migration requires the `migrate` feature.\n\
                     Rebuild with: cargo build --features migrate",
                    "✗".red()
                );
                Err(anyhow!("migrate feature not enabled"))
            }
        }

        "aws-secrets-manager" | "aws" => Ok(Box::new(aws::AwsDestination::interactive(
            args.secret_name.clone(),
            args.aws_profile.clone(),
        )?)),
        "doppler" => Ok(Box::new(doppler::DopplerDestination::new(
            args.project.clone(),
            args.doppler_config.clone(),
        ))),
        "infisical" => Ok(Box::new(infisical::InfisicalDestination::new(
            args.project.clone(),
            args.infisical_env.clone(),
        ))),
        "gcp-secret-manager" | "gcp" => Ok(Box::new(gcp::GcpDestination::new())),
        "azure-keyvault" | "azure" => Ok(Box::new(azure::AzureDestination::interactive(
            args.vault_name.clone(),
        )?)),

        // ── PaaS platforms ────────────────────────────────────────────────
        "vercel" => Ok(Box::new(vercel::VercelDestination::new(
            args.vercel_project.clone(),
        ))),
        "heroku" => Ok(Box::new(heroku::HerokuDestination::interactive(
            args.heroku_app.clone(),
        )?)),
        "railway" => Ok(Box::new(railway::RailwayDestination::new(
            args.railway_project.clone(),
        ))),

        other => Err(anyhow!(
            "Unknown destination: '{}'. Run `evnx migrate` without --to to pick from the list.",
            other
        )),
    }
}

/// Ordered `(slug, display-label)` pairs used by the interactive picker in
/// `migrate/mod.rs`. Update this list whenever a new destination is added.
pub fn available_destinations() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "github-actions",
            "github-actions      — GitHub Actions Secrets",
        ),
        (
            "aws-secrets-manager",
            "aws-secrets-manager — AWS Secrets Manager",
        ),
        ("doppler", "doppler             — Doppler secrets platform"),
        (
            "infisical",
            "infisical           — Infisical secrets platform",
        ),
        (
            "gcp-secret-manager",
            "gcp-secret-manager  — Google Cloud Secret Manager",
        ),
        ("azure-keyvault", "azure-keyvault      — Azure Key Vault"),
        (
            "vercel",
            "vercel              — Vercel Environment Variables",
        ),
        ("heroku", "heroku              — Heroku Config Vars"),
        ("railway", "railway             — Railway Variables"),
    ]
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::migrate::MigrateArgs;

    /// Returns args with every required destination field populated.
    ///
    /// `interactive()` constructors check `Option::is_some()` first and skip
    /// the dialoguer prompt entirely when a value is present.  Tests must use
    /// this helper (not `empty_args`) whenever they call `get_destination()` so
    /// no stdin prompt is attempted in a non-TTY environment.
    fn full_args() -> MigrateArgs {
        MigrateArgs {
            from: None,
            to: None,
            source_file: ".env".into(),
            dry_run: false,
            skip_existing: false,
            overwrite: false,
            verbose: false,
            include: None,
            exclude: None,
            strip_prefix: None,
            add_prefix: None,
            repo: None,
            github_token: None,
            // All required-when-interactive fields populated:
            secret_name: Some("test/secret".into()),
            aws_profile: None,
            project: Some("test-project".into()),
            doppler_config: None,
            infisical_env: None,
            vault_name: Some("test-vault".into()),
            heroku_app: Some("test-app".into()),
            vercel_project: None,
            railway_project: None,
        }
    }

    #[test]
    fn test_unknown_destination_errors() {
        // unknown slugs don't touch any constructor — empty args are fine here
        let args = full_args();
        let result = get_destination("consul", &args);
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("Unknown destination"));
    }

    #[test]
    fn test_aws_slug_and_alias_both_resolve() {
        // `secret_name` is populated in full_args — interactive() won't prompt
        for slug in ["aws", "aws-secrets-manager"] {
            let dest = get_destination(slug, &full_args()).unwrap();
            assert_eq!(dest.name(), "AWS Secrets Manager");
        }
    }

    #[test]
    fn test_all_non_github_destinations_resolve() {
        // full_args() ensures azure/heroku/aws interactive() paths skip stdin
        let args = full_args();
        for slug in [
            "aws",
            "doppler",
            "infisical",
            "gcp",
            "azure",
            "vercel",
            "heroku",
            "railway",
        ] {
            let result = get_destination(slug, &args);
            assert!(
                result.is_ok(),
                "slug '{}' failed to resolve: {:?}",
                slug,
                result.err()
            );
        }
    }

    /// Every slug in `available_destinations()` must be resolvable via
    /// `get_destination()`, otherwise the interactive picker would offer an
    /// option the user can never successfully run.
    #[test]
    fn test_available_destinations_are_all_resolvable() {
        let args = full_args();
        for (slug, _) in available_destinations() {
            if *slug == "github-actions" {
                continue;
            } // needs feature flag
            get_destination(slug, &args).unwrap_or_else(|e| {
                panic!(
                    "available_destinations() lists '{}' but it cannot be resolved: {}",
                    slug, e
                )
            });
        }
    }
}
