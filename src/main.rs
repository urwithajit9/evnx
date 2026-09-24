//! evnx CLI entry point.

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use std::path::Path;

use evnx::cli::{Cli, Commands, SpecAction};
use evnx::commands;
use evnx::core::converter::KeyTransform;

/// `[sync] naming_policy` arrives as a string; map it to the enum clap parses.
///
/// An unrecognised value yields `None`, so it falls through to the built-in
/// default — and `core::config` has already warned that the key is not
/// understood, so it is reported rather than silently ignored.
fn parse_naming_policy(value: &str) -> Option<evnx::cli::NamingPolicy> {
    match value.trim().to_ascii_lowercase().as_str() {
        "warn" => Some(evnx::cli::NamingPolicy::Warn),
        "error" => Some(evnx::cli::NamingPolicy::Error),
        "ignore" => Some(evnx::cli::NamingPolicy::Ignore),
        _ => None,
    }
}

/// Resolve which `.env.<name>` a command operates on, or exit 2.
///
/// ⚠️ **Not `?`**, for the same reason `config::load` below is not. A
/// `--env-name` that names a file which does not exist is "evnx could not run",
/// and `?` surfaces that through `main`'s `Result` as exit **1** — which
/// `validate` uses to mean *invalid*, `diff` to mean *differences found*, and
/// `sync --check` to mean *drifted*.
///
/// So a typo in a CI flag reported a finding rather than a broken setup, and the
/// pipeline branched on it as though the answer were real. `scan` is unaffected
/// only because it has no `--env-name` to mistype.
///
/// 2 is what every command with a three-value contract already uses for "no
/// verdict", so the answer is the same whichever command was asked.
fn resolve_env(
    dir: &Path,
    env: Option<&str>,
    env_name: Option<&str>,
    configured: Option<&str>,
) -> String {
    match evnx::core::env_name::select(dir, env, env_name, configured) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("{} {:#}", "Error:".on_red().bold(), e);
            eprintln!();
            eprintln!("No verdict: evnx did not run. This is not a clean result.");
            std::process::exit(2);
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Configure colored output
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Project policy from `.evnx.toml`, loaded once and applied per command
    // below. Precedence is config < flag throughout: see `core::config::pick`.
    //
    // ⚠️ Announced rather than applied silently. `[scan] severity` and
    // `[scan] exclude` can weaken what the scanner reports, and the file is
    // committed — so one commit could otherwise narrow scanning for everyone who
    // clones the repository with nothing on the command line to show it.
    // ⚠️ An **absolute** directory, not `Path::new(".")`. `find` walks upward with
    // `Path::parent`, and `".".parent()` is `""` rather than the parent directory
    // — so a relative start makes the walk inert and the file is found only in
    // the working directory. `cloud::sync` and `cloud::status` already pass
    // `current_dir()` for this reason.
    let here = std::env::current_dir().context("reading the current directory")?;
    // ⚠️ Not `?`. A `.evnx.toml` that will not parse is "evnx could not run",
    // and `?` would surface it through `main`'s `Result` as exit **1** — which
    // for `scan` is the code that means *secrets found*, and for `sync --check`
    // means *drifted*. A malformed config would have read as a finding.
    //
    // 2 is the code every command with a three-value contract already uses for
    // "no verdict", so a broken config reports the same thing everywhere rather
    // than impersonating whatever failure that command happens to number 1.
    let loaded = match evnx::core::config::load(&here) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("{} {:#}", "Error:".on_red().bold(), e);
            eprintln!();
            eprintln!("No verdict: evnx did not run. This is not a clean result.");
            std::process::exit(2);
        }
    };
    if let Some(source) = &loaded.source {
        evnx::utils::ui::config_banner(source, &loaded.config.security_overrides(), cli.quiet);
    }
    for warning in &loaded.warnings {
        evnx::utils::ui::config_warning(warning, cli.quiet);
    }
    let cfg = loaded.config;

    // Route to command handler
    // ⚠️ Bound, not returned. See the exit-code handling at the end of `main`.
    let outcome = match cli.command {
        Commands::Init {
            path,
            yes,
            blueprint,
            with,
            list_components,
            detect,
            from_source,
            force,
        } => commands::init::run(
            path,
            yes,
            force,
            blueprint,
            with,
            list_components,
            detect,
            from_source,
            cli.verbose,
        ),

        Commands::Add { target, path, yes } => commands::add::run(target, path, yes, cli.verbose),

        Commands::Validate {
            env,
            env_name,
            example,
            strict,
            fix,
            format,
            exit_zero,
            ignore,
            validate_formats,
        } => commands::validate::run(
            resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            ),
            evnx::core::config::pick(
                example,
                cfg.defaults.example.clone(),
                ".env.example".to_string(),
            ),
            evnx::core::config::any(strict, cfg.validate.strict),
            fix,
            format,
            exit_zero,
            cli.verbose,
            evnx::core::config::extend(ignore, cfg.validate.ignore),
            evnx::core::config::any(validate_formats, cfg.validate.validate_formats),
            cfg.vars.clone(),
        ),

        Commands::Scan {
            path,
            exclude,
            pattern,
            ignore_placeholders,
            severity,
            format,
            exit_zero,
        } => commands::scan::run(
            path,
            evnx::core::config::extend(exclude, cfg.scan.exclude),
            // Additive, like `exclude` above: `--pattern` is this run's rule and
            // `[[scan.patterns]]` is the project's standing set.
            commands::scan::patternset::merge(pattern, cfg.scan.patterns),
            evnx::core::config::any(ignore_placeholders, cfg.scan.ignore_placeholders),
            evnx::core::config::pick(severity, cfg.scan.severity, "low".to_string()),
            format,
            exit_zero,
            cli.verbose,
            cfg.vars.clone(),
        ),

        Commands::Diff {
            env,
            env_name,
            example,
            against,
            show_values,
            format,
            reverse,
            ignore_keys,
            with_stats,
            interactive,
        } => {
            let here = Path::new(".");
            match commands::diff::run(
                resolve_env(
                    here,
                    env.as_deref(),
                    env_name.as_deref(),
                    cfg.defaults.env_name.as_deref(),
                ),
                // The right-hand side: --example, then --against as a name,
                // then [defaults] example. `env_name` is not consulted here —
                // it names the *left* side.
                match (&example, &against) {
                    (Some(path), _) => path.clone(),
                    (None, Some(_)) => resolve_env(here, None, against.as_deref(), None),
                    (None, None) => cfg
                        .defaults
                        .example
                        .clone()
                        .unwrap_or_else(|| ".env.example".to_string()),
                },
                show_values,
                format,
                reverse,
                cli.verbose,
                evnx::core::config::extend(ignore_keys, cfg.diff.ignore_keys),
                with_stats,
                interactive,
            ) {
                Ok(exit_code) => std::process::exit(exit_code),
                Err(e) => {
                    // ⚠️ `{:#}`, like every other anyhow site here. With `{}`
                    // only the outermost context printed, so `diff` reported
                    // "Failed to parse .env.example" and dropped the
                    // "Invalid format at line 1: missing '=' separator" that
                    // says what to actually fix. The two sites below print a
                    // downcast BackupError/RestoreError, where `{}` is right.
                    eprintln!("{} {:#}", "Error:".on_red().bold(), e);
                    std::process::exit(2);
                }
            }
        }

        Commands::Convert {
            env,
            env_name,
            to,
            output,
            include,
            exclude,
            base64,
            prefix,
            transform,
        } => {
            let transform_enum = transform.as_deref().and_then(|t| match t {
                "uppercase" => Some(KeyTransform::Uppercase),
                "lowercase" => Some(KeyTransform::Lowercase),
                "camelCase" => Some(KeyTransform::CamelCase),
                "snake_case" => Some(KeyTransform::SnakeCase),
                unknown => {
                    if cli.verbose {
                        eprintln!("{} Invalid transform '{}', ignoring", "!".yellow(), unknown);
                    }
                    None
                }
            });

            let env = resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            );
            let config = commands::convert::ConvertConfig::builder()
                .env(env)
                .target_format(to)
                .output_path(output)
                .include_pattern(include)
                .exclude_pattern(exclude)
                .base64(base64)
                .prefix(prefix)
                .transform(transform_enum)
                .verbose(cli.verbose)
                .build();

            commands::convert::run(config).context("Convert command failed")
        }

        // ── Migrate ───────────────────────────────────────────────────────────
        //
        // The variant holds Box<MigrateOptions> so the Commands enum stays
        // small on the stack (~8 bytes for this arm vs. ~435 bytes inline).
        // All fields are accessed via `opts.` after auto-deref.
        #[cfg(feature = "migrate")]
        Commands::Migrate(opts) => commands::migrate::run(commands::migrate::MigrateArgs {
            from: opts.from.clone(),
            source_file: opts.source_file.clone(),
            to: opts.to.clone(),
            dry_run: opts.dry_run,
            skip_existing: opts.skip_existing,
            overwrite: opts.overwrite,
            verbose: cli.verbose,
            include: opts.include.clone(),
            exclude: opts.exclude.clone(),
            strip_prefix: opts.strip_prefix.clone(),
            add_prefix: opts.add_prefix.clone(),
            repo: opts.repo.clone(),
            github_token: opts.github_token.clone(),
            secret_name: opts.secret_name.clone(),
            aws_profile: opts.aws_profile.clone(),
            project: opts.project.clone(),
            doppler_config: opts.doppler_config.clone(),
            infisical_env: opts.infisical_env.clone(),
            vault_name: opts.vault_name.clone(),
            heroku_app: opts.heroku_app.clone(),
            vercel_project: opts.vercel_project.clone(),
            railway_project: opts.railway_project.clone(),
        }),

        Commands::Sync { args } => commands::sync::run(
            resolve_env(
                Path::new("."),
                args.env.as_deref(),
                args.env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            ),
            evnx::core::config::pick(
                args.example.clone(),
                cfg.defaults.example.clone(),
                ".env.example".to_string(),
            ),
            args.direction,
            args.placeholder,
            cli.verbose,
            args.dry_run,
            args.force,
            args.check,
            args.format.clone(),
            args.template_config.clone(),
            evnx::core::config::pick(
                args.naming_policy,
                cfg.sync
                    .naming_policy
                    .as_deref()
                    .and_then(parse_naming_policy),
                evnx::cli::NamingPolicy::Warn,
            ),
            cfg.vars.clone(),
        ),

        // Commands::Template { input, output, env } => {
        //     commands::template::run(input, output, env, cli.verbose)
        // }
        Commands::Template {
            input,
            output,
            env,
            env_name,
            gitignore,
            no_gitignore,
            strict,
        } => {
            let mode = if gitignore {
                commands::template::GitignoreMode::Auto
            } else if no_gitignore {
                commands::template::GitignoreMode::Skip
            } else {
                commands::template::GitignoreMode::Default
            };
            let env = resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            );
            commands::template::run(input, output, env, cli.verbose, mode, strict)
        }

        // #[cfg(feature = "backup")]
        // Commands::Backup { env, output } => commands::backup::run(env, output, cli.verbose),
        #[cfg(feature = "backup")]
        Commands::Backup {
            env,
            env_name,
            output,
            key_file,
            keep,
            verify,
        } => match commands::backup::run(
            resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            ),
            output,
            cli.verbose,
            key_file,
            evnx::core::config::pick(keep, cfg.backup.keep, 3),
            verify,
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                if let Some(be) = e.downcast_ref::<commands::backup::BackupError>() {
                    if !be.is_silent() {
                        eprintln!("{} {}", "Error:".on_red().bold(), be);
                    }
                    std::process::exit(be.exit_code());
                }
                Err(e)
            }
        },

        // #[cfg(feature = "backup")]
        // Commands::Restore {
        //     backup,
        //     output,
        //     dry_run,
        // } => commands::restore::run(backup, output, cli.verbose, dry_run),
        #[cfg(feature = "backup")]
        Commands::Restore {
            backup,
            output,
            dry_run,
            inspect,
            password_file,
        } => {
            match commands::restore::run(
                backup,
                output,
                cli.verbose,
                dry_run,
                inspect,
                password_file,
            ) {
                Ok(()) => Ok(()),
                Err(e) => {
                    if let Some(re) = e.downcast_ref::<commands::restore::RestoreError>() {
                        // Cancelled and ValidationFallback are silent — they
                        // already printed a complete inline explanation.
                        if !re.is_silent() {
                            eprintln!("{} {}", "Error:".on_red().bold(), re);
                        }
                        std::process::exit(re.exit_code());
                    }
                    // Generic anyhow error — propagate for default formatting.
                    Err(e)
                }
            }
        }

        #[cfg(feature = "cloud")]
        Commands::Auth { command, server } => {
            evnx::cloud::run_auth(command, server.as_deref(), cli.verbose)
        }

        #[cfg(feature = "cloud")]
        Commands::Vault { command, server } => {
            evnx::cloud::run_vault(command, server.as_deref(), cli.verbose)
        }

        #[cfg(feature = "cloud")]
        Commands::Cloud { command, server } => {
            evnx::cloud::run(command, server.as_deref(), cli.verbose)
        }

        Commands::Doctor {
            path,
            project_path,
            fix,
            strict,
            format,
            verbose,
        } => evnx::commands::doctor::run(
            project_path.unwrap_or(path),
            verbose,
            // Either saying yes is enough: a flag can turn repair on, never off,
            // which is the same rule `.evnx.toml` booleans follow.
            fix,
            strict,
            format,
        ),

        Commands::Spec { action } => match action {
            SpecAction::Init {
                with,
                env,
                example,
                stdout,
                force,
            } => commands::spec::run(with, env, example, stdout, force, cli.verbose),
        },

        Commands::Completions { shell } => commands::completions::run(shell),
    };

    // ⚠️ An error is exit **2**, not 1.
    //
    // `scan`, `diff`, `doctor` and `sync --check` already document 2 as "could
    // not run, so there is no verdict", and 1 as a real finding — *secrets
    // found*, *differences*, *unhealthy*, *drifted*. Returning the command's
    // `Result` from `main` gave Rust's default `Termination`, which numbers any
    // error 1, so `evnx validate --env ./missing.env` reported **validation
    // failed** for a file it never opened.
    //
    // `diff` and `sync` were already wrapped individually and are unaffected;
    // this catches `validate`, `convert` and `template`, and anything added
    // later, by default rather than by remembering.
    //
    // Commands that exit on their own — `scan`, `doctor`, `backup`, `restore`
    // and the findings path in `validate` — never reach here.
    if let Err(e) = outcome {
        eprintln!("{} {:#}", "Error:".on_red().bold(), e);
        eprintln!();
        eprintln!("No verdict: evnx did not finish. This is not a clean result.");
        std::process::exit(2);
    }

    Ok(())
}
