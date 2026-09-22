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
use std::io::IsTerminal;

// Re-export the trait so callers can write `destinations::MigrationDestination`
// without having to reach back up two levels to `migrate::destination`.
pub use crate::commands::migrate::destination::MigrationDestination;

use crate::commands::migrate::MigrateArgs;

/// Resolve a destination identifier that may come from a flag, a prompt, or
/// neither.
///
/// ⚠️ This exists because `--dry-run` could not be used without a terminal.
/// The prompt lived in each destination's `interactive()` constructor, which
/// runs here in `get_destination` — **before** `migrate()` ever sees
/// `opts.dry_run`. So `evnx migrate --to aws --dry-run` in CI failed with
/// `IO error: not a terminal`, and the flag whose entire purpose is to preview
/// safely was the one thing that could not run unattended.
///
/// The order matters:
///
/// 1. A flag was given — use it, whatever else is true.
/// 2. `--dry-run` — nothing will be created, so a placeholder is enough to
///    preview which variables would go where.
/// 3. A terminal — ask.
/// 4. Otherwise — fail naming the exact flag, rather than reporting the
///    absence of a terminal as though that were the problem.
pub(crate) fn resolve_arg(
    value: Option<String>,
    flag: &str,
    prompt: &str,
    dry_run: bool,
) -> Result<String> {
    if let Some(v) = value {
        return Ok(v);
    }
    if dry_run {
        return Ok(format!("<{flag} not set>"));
    }
    if std::io::stdin().is_terminal() {
        return Ok(dialoguer::Input::new()
            .with_prompt(prompt)
            .interact_text()?);
    }
    Err(anyhow!(
        "{prompt} is required. Pass `{flag}`.\n\
         \x20 evnx would normally ask, but stdin is not a terminal."
    ))
}

/// Resolve a destination slug to a boxed `MigrationDestination`.
///
/// Called once per `evnx migrate` run from `migrate/mod.rs`.
pub fn get_destination(name: &str, args: &MigrateArgs) -> Result<Box<dyn MigrationDestination>> {
    match name {
        // ── Cloud CI / secret managers ────────────────────────────────────
        "github-actions" | "github" => {
            #[cfg(feature = "migrate")]
            {
                let repo = resolve_arg(
                    args.repo.clone(),
                    "--repo",
                    "GitHub repository (owner/repo)",
                    args.dry_run,
                )?;
                // ⚠️ `GITHUB_TOKEN` stays ahead of the prompt. Inside a GitHub
                // Actions job that variable is already set, and this is the one
                // destination that really uploads — losing the fallback would
                // mean the only working CI path needed a flag it never used to.
                let token = resolve_arg(
                    args.github_token
                        .clone()
                        .or_else(|| std::env::var("GITHUB_TOKEN").ok()),
                    "--github-token",
                    "GitHub personal access token",
                    args.dry_run,
                )?;
                Ok(Box::new(github::GitHubDestination::new(repo, token)))
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

        "aws-secrets-manager" | "aws" => {
            let name = resolve_arg(
                args.secret_name.clone(),
                "--secret-name",
                "AWS secret name (e.g. prod/myapp/config)",
                args.dry_run,
            )?;
            Ok(Box::new(aws::AwsDestination::new(
                name,
                args.aws_profile.clone(),
            )))
        }
        "doppler" => Ok(Box::new(doppler::DopplerDestination::new(
            args.project.clone(),
            args.doppler_config.clone(),
        ))),
        "infisical" => Ok(Box::new(infisical::InfisicalDestination::new(
            args.project.clone(),
            args.infisical_env.clone(),
        ))),
        "gcp-secret-manager" | "gcp" => Ok(Box::new(gcp::GcpDestination::new())),
        "azure-keyvault" | "azure" => {
            let name = resolve_arg(
                args.vault_name.clone(),
                "--vault-name",
                "Azure Key Vault name",
                args.dry_run,
            )?;
            Ok(Box::new(azure::AzureDestination::new(name)))
        }

        // ── PaaS platforms ────────────────────────────────────────────────
        "vercel" => Ok(Box::new(vercel::VercelDestination::new(
            args.vercel_project.clone(),
        ))),
        "heroku" => {
            let app = resolve_arg(
                args.heroku_app.clone(),
                "--heroku-app",
                "Heroku app name",
                args.dry_run,
            )?;
            Ok(Box::new(heroku::HerokuDestination::new(app)))
        }
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

    // ─── resolve_arg ────────────────────────────────────────────────────────

    /// A flag beats everything else, including `--dry-run`'s placeholder.
    #[test]
    fn resolve_arg_prefers_the_flag() {
        let got = resolve_arg(Some("prod/app".into()), "--secret-name", "Secret", true).unwrap();
        assert_eq!(got, "prod/app");
    }

    /// ⚠️ The regression this whole helper exists for. `--dry-run` used to hit
    /// a `dialoguer` prompt inside the destination's constructor and die with
    /// `IO error: not a terminal`, so the flag meant for CI could not run in CI.
    #[test]
    fn resolve_arg_under_dry_run_never_prompts() {
        let got = resolve_arg(None, "--secret-name", "Secret", true).unwrap();
        assert_eq!(got, "<--secret-name not set>");
    }

    /// Not a terminal and not a dry run: fail, but name the flag. The old error
    /// said only "not a terminal", which describes evnx's situation rather than
    /// what the caller has to do about it.
    #[test]
    fn resolve_arg_headless_error_names_the_flag() {
        // The test harness runs without a tty, which is exactly the case here.
        let err = resolve_arg(None, "--vault-name", "Azure Key Vault name", false)
            .expect_err("should refuse without a tty");
        let msg = err.to_string();
        assert!(msg.contains("--vault-name"), "flag not named: {msg}");
        assert!(
            msg.contains("Azure Key Vault name"),
            "prompt not shown: {msg}"
        );
    }

    // ─── kind() ─────────────────────────────────────────────────────────────

    /// Eight of the nine only print commands, and the summary must not call
    /// that an upload. Pinned per destination so adding a real uploader is a
    /// deliberate act rather than a default.
    #[test]
    fn only_github_reports_that_it_uploads() {
        use crate::commands::migrate::destination::DestinationKind;
        let args = full_args();
        for slug in [
            "aws-secrets-manager",
            "doppler",
            "infisical",
            "gcp-secret-manager",
            "azure-keyvault",
            "vercel",
            "heroku",
            "railway",
        ] {
            let dest = get_destination(slug, &args).unwrap();
            assert_eq!(
                dest.kind(),
                DestinationKind::EmitsCommands,
                "{slug} claims to upload, but it prints commands"
            );
        }
    }

    /// ⚠️ `full_args()` deliberately leaves `repo` and `github_token` unset —
    /// which is why the resolvability test above skips this slug. Both are
    /// supplied here, so the test exercises `kind()` rather than the argument
    /// resolver.
    #[cfg(feature = "migrate")]
    #[test]
    fn github_is_the_one_that_uploads() {
        use crate::commands::migrate::destination::DestinationKind;
        let args = MigrateArgs {
            repo: Some("owner/repo".into()),
            github_token: Some("ghp_test".into()),
            ..full_args()
        };
        let dest = get_destination("github-actions", &args).unwrap();
        assert_eq!(dest.kind(), DestinationKind::Uploads);
    }

    /// Headless, no `--repo`, not a dry run: refuse and say which flag.
    ///
    /// This is the behaviour that replaced `interactive()`. Worth pinning
    /// because the previous version reached for a prompt here and reported
    /// `IO error: not a terminal`.
    #[cfg(feature = "migrate")]
    #[test]
    fn github_without_repo_names_the_flag() {
        // `Box<dyn MigrationDestination>` is not `Debug`, so `expect_err` is
        // unavailable here.
        match get_destination("github-actions", &full_args()) {
            Ok(_) => panic!("should refuse without --repo"),
            Err(e) => assert!(e.to_string().contains("--repo"), "{e}"),
        }
    }
}
