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

use crate::core::args::{self, Name};
use anyhow::{anyhow, Result};

// Re-export the trait so callers can write `destinations::MigrationDestination`
// without having to reach back up two levels to `migrate::destination`.
pub use crate::commands::migrate::destination::MigrationDestination;

use crate::commands::migrate::MigrateArgs;

/// What a missing argument costs, which decides what `--dry-run` may do without
/// it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgKind {
    /// Names the thing being written to — a Heroku app, a secret name, a repo.
    ///
    /// ⚠️ These are **printed into the commands evnx tells the user to run**, so
    /// a placeholder does not merely look untidy: `--dry-run` without
    /// `--heroku-app` emitted
    /// `heroku config --app <--heroku-app not set>` and exited 0. Someone
    /// pasting that gets an error with no obvious cause.
    Identifier,
    /// A credential. Never printed, and never needed by `--dry-run`, which
    /// uploads nothing — so a placeholder here costs nothing and keeping it is
    /// what lets `--dry-run` work on a machine with no credentials at all.
    /// `env` is the variable that supplies it, named first when it is missing:
    /// the flag is the form evnx warns against (I-273).
    // Only the GitHub destination takes one, and it is built with `migrate`.
    #[cfg_attr(not(feature = "migrate"), allow(dead_code))]
    Credential { env: &'static str },
}

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
/// 2. `--dry-run` **and a credential** — nothing will be uploaded, so a
///    placeholder costs nothing. An *identifier* still has to be real: it gets
///    printed into the command evnx tells the user to run. See [`ArgKind`].
/// 3. A terminal — ask.
/// 4. Otherwise — fail naming the exact flag, rather than reporting the
///    absence of a terminal as though that were the problem.
pub(crate) fn resolve_arg(
    value: Option<String>,
    flag: &str,
    prompt: &str,
    dry_run: bool,
    kind: ArgKind,
) -> Result<String> {
    resolve_arg_with(
        value,
        flag,
        prompt,
        dry_run,
        kind,
        crate::utils::term::interactive(),
    )
}

/// The logic, with the terminal decision handed in.
///
/// ⚠️ `resolve_arg` reads `term::interactive()`, which is ambient process
/// state — so the tests below inherited whatever the invoking shell happened to
/// provide. Under `cargo test` run from a terminal that is *true*, and
/// `interact_text()` sat waiting for someone to type, hanging the suite. In CI
/// stdin is `/dev/null`, so the same tests passed. A test whose result depends
/// on how it was launched is a test that reports nothing.
pub(crate) fn resolve_arg_with(
    value: Option<String>,
    flag: &str,
    prompt: &str,
    dry_run: bool,
    kind: ArgKind,
    interactive: bool,
) -> Result<String> {
    if let Some(v) = value {
        return Ok(v);
    }
    if dry_run && matches!(kind, ArgKind::Credential { .. }) {
        return Ok(format!("<{flag} not set>"));
    }
    if dry_run {
        return Err(anyhow!(
            "{prompt} is required for this destination. Pass `{flag}`.\n\
             \x20 --dry-run still needs it: without it evnx would print a command\n\
             \x20 with a placeholder where the name should be."
        ));
    }
    if interactive {
        return Ok(dialoguer::Input::new()
            .with_prompt(prompt)
            .interact_text()?);
    }
    let how = match kind {
        ArgKind::Credential { env } => format!("Set `{env}`, or pass `{flag}`"),
        ArgKind::Identifier => format!("Pass `{flag}`"),
    };
    Err(anyhow!(
        "{prompt} is required. {how}.\n\
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
                    ArgKind::Identifier,
                )?;
                args::check("--repo", &repo, Name::GitHubRepo)?;
                // I-108: a token typed as a flag sits in shell history and in
                // `ps` for as long as the upload runs. Said, not refused — the
                // flag has always worked, and it is the token's owner's call.
                // ⚠️ The flag no longer has clap's `env = "GITHUB_TOKEN"`, so
                // `Some` here means it was typed, never that it was exported.
                if args.github_token.is_some() {
                    crate::utils::ui::warning(
                        "--github-token puts the token in your shell history and in `ps` \
                         while evnx runs. Export GITHUB_TOKEN instead: evnx reads it when \
                         the flag is absent.",
                    );
                }
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
                    ArgKind::Credential {
                        env: "GITHUB_TOKEN",
                    },
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
                ArgKind::Identifier,
            )?;
            args::check("--secret-name", &name, Name::AwsSecretName)?;
            args::check_opt("--aws-profile", args.aws_profile.as_deref(), Name::Slug)?;
            Ok(Box::new(aws::AwsDestination::new(
                name,
                args.aws_profile.clone(),
            )))
        }
        "doppler" => {
            args::check_opt("--project", args.project.as_deref(), Name::Slug)?;
            args::check_opt(
                "--doppler-config",
                args.doppler_config.as_deref(),
                Name::Slug,
            )?;
            Ok(Box::new(doppler::DopplerDestination::new(
                args.project.clone(),
                args.doppler_config.clone(),
            )))
        }
        "infisical" => {
            args::check_opt("--project", args.project.as_deref(), Name::Slug)?;
            args::check_opt("--infisical-env", args.infisical_env.as_deref(), Name::Slug)?;
            Ok(Box::new(infisical::InfisicalDestination::new(
                args.project.clone(),
                args.infisical_env.clone(),
            )))
        }
        // DEC-MG2: required, as `convert --to gcp-secrets` requires it (DEC-CV1).
        // It is printed into every command; gcloud's configured project is a
        // guess about where the reader's shell will point.
        "gcp-secret-manager" | "gcp" => {
            let project = resolve_arg(
                args.project.clone(),
                "--project",
                "GCP project ID",
                args.dry_run,
                ArgKind::Identifier,
            )?;
            args::check("--project", &project, Name::GcpProject)?;
            Ok(Box::new(gcp::GcpDestination::new(project)))
        }
        "azure-keyvault" | "azure" => {
            let name = resolve_arg(
                args.vault_name.clone(),
                "--vault-name",
                "Azure Key Vault name",
                args.dry_run,
                ArgKind::Identifier,
            )?;
            args::check("--vault-name", &name, Name::AzureVault)?;
            Ok(Box::new(azure::AzureDestination::new(name)))
        }

        // ── PaaS platforms ────────────────────────────────────────────────
        "vercel" => {
            args::check_opt(
                "--vercel-project",
                args.vercel_project.as_deref(),
                Name::Slug,
            )?;
            Ok(Box::new(vercel::VercelDestination::new(
                args.vercel_project.clone(),
            )))
        }
        "heroku" => {
            let app = resolve_arg(
                args.heroku_app.clone(),
                "--heroku-app",
                "Heroku app name",
                args.dry_run,
                ArgKind::Identifier,
            )?;
            args::check("--heroku-app", &app, Name::HerokuApp)?;
            Ok(Box::new(heroku::HerokuDestination::new(app)))
        }
        "railway" => {
            args::check_opt(
                "--railway-project",
                args.railway_project.as_deref(),
                Name::Slug,
            )?;
            args::check_opt(
                "--railway-environment",
                args.railway_environment.as_deref(),
                Name::Slug,
            )?;
            args::check_opt(
                "--railway-service",
                args.railway_service.as_deref(),
                Name::Slug,
            )?;
            // DEC-MG1, I-269: Railway refuses `--project` without
            // `--environment` ("--environment is required when using
            // --project"), so a command printed with one and not the other
            // would fail when run.
            if args.railway_project.is_some() && args.railway_environment.is_none() {
                return Err(anyhow!(
                    "--railway-project needs --railway-environment: Railway asks for \
                     the environment whenever a command names the project.\n\
                     \x20 evnx migrate --to railway --railway-project <ID> \
                     --railway-environment production\n\
                     \x20 Or leave both out and run the command in a directory linked \
                     with `railway link`."
                ));
            }
            Ok(Box::new(railway::RailwayDestination::new(
                args.railway_project.clone(),
                args.railway_environment.clone(),
                args.railway_service.clone(),
            )))
        }

        other => Err(anyhow!(
            "Unknown destination: '{}'. Run `evnx migrate` without --to to pick from the list.",
            other
        )),
    }
}

/// Whether `slug` names the destination that uploads rather than prints —
/// asked before it is built, which may prompt (`--print-only`, I-111). The
/// test `uploads_agrees_with_kind` keeps it in step with
/// [`MigrationDestination::kind`].
pub fn uploads(slug: &str) -> bool {
    matches!(slug, "github-actions" | "github")
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
            env_name: None,
            dry_run: false,
            print_only: false,
            allow_collisions: false,
            skip_existing: false,
            overwrite: false,
            yes: false,
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
            railway_environment: None,
            railway_service: None,
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
        let got = resolve_arg_with(
            Some("prod/app".into()),
            "--secret-name",
            "Secret",
            true,
            ArgKind::Identifier,
            false,
        )
        .unwrap();
        assert_eq!(got, "prod/app");
    }

    /// ⚠️ The regression this whole helper exists for. `--dry-run` used to hit
    /// a `dialoguer` prompt inside the destination's constructor and die with
    /// `IO error: not a terminal`, so the flag meant for CI could not run in CI.
    #[test]
    fn resolve_arg_under_dry_run_never_prompts_for_a_credential() {
        let got = resolve_arg_with(
            None,
            "--github-token",
            "Token",
            true,
            ArgKind::Credential {
                env: "GITHUB_TOKEN",
            },
            false,
        )
        .unwrap();
        assert_eq!(got, "<--github-token not set>");
    }

    /// I-273: a missing credential names the variable first; the flag is the
    /// form evnx warns against.
    #[test]
    fn a_missing_credential_names_the_variable_first() {
        let err = resolve_arg_with(
            None,
            "--github-token",
            "GitHub personal access token",
            false,
            ArgKind::Credential {
                env: "GITHUB_TOKEN",
            },
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("Set `GITHUB_TOKEN`, or pass `--github-token`"),
            "{err}"
        );
    }

    /// ⚠️ An **identifier** is different: it is printed into the command evnx
    /// tells the user to run. `evnx migrate --to heroku --dry-run` without
    /// `--heroku-app` used to emit
    /// `heroku config --app <--heroku-app not set>` and exit 0, so someone
    /// pasting the preview got an error with no visible cause.
    #[test]
    fn resolve_arg_under_dry_run_still_needs_an_identifier() {
        let err = resolve_arg_with(
            None,
            "--heroku-app",
            "Heroku app name",
            true,
            ArgKind::Identifier,
            false,
        )
        .expect_err("a dry run cannot print a command it does not know");
        let msg = err.to_string();
        assert!(msg.contains("--heroku-app"), "flag not named: {msg}");
    }

    /// Not a terminal and not a dry run: fail, but name the flag. The old error
    /// said only "not a terminal", which describes evnx's situation rather than
    /// what the caller has to do about it.
    #[test]
    fn resolve_arg_headless_error_names_the_flag() {
        // ⚠️ `interactive: false` is passed explicitly. This used to call
        // `resolve_arg`, whose comment claimed "the test harness runs without a
        // tty" — which is true in CI and false when `cargo test` is run from a
        // terminal. In that case it reached `interact_text()` and **hung the
        // whole suite** waiting for someone to type. Ambient process state is
        // not a fixture.
        let err = resolve_arg_with(
            None,
            "--vault-name",
            "Azure Key Vault name",
            false,
            ArgKind::Identifier,
            false,
        )
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

    /// `uploads()` answers before a destination is built; `kind()` after.
    #[cfg(feature = "migrate")]
    #[test]
    fn uploads_agrees_with_kind() {
        use crate::commands::migrate::destination::DestinationKind;
        let args = MigrateArgs {
            repo: Some("owner/repo".into()),
            github_token: Some("ghp_test".into()),
            ..full_args()
        };
        for (slug, _) in available_destinations() {
            let dest = get_destination(slug, &args).unwrap();
            assert_eq!(
                uploads(slug),
                dest.kind() == DestinationKind::Uploads,
                "{slug}"
            );
        }
        assert!(uploads("github"));
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
