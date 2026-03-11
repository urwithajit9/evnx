//! evnx CLI entry point.

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;

use evnx::cli::{Cli, Commands};
use evnx::commands;
use evnx::core::converter::KeyTransform;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Configure colored output
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Route to command handler
    match cli.command {
        Commands::Init { path, yes } => commands::init::run(path, yes, cli.verbose),

        Commands::Add { target, path, yes } => commands::add::run(target, path, yes, cli.verbose),

        Commands::Validate {
            env,
            example,
            strict,
            fix,
            format,
            exit_zero,
            ignore,
            validate_formats,
            pattern,
        } => commands::validate::run(
            env,
            example,
            strict,
            fix,
            format,
            exit_zero,
            cli.verbose,
            ignore,
            validate_formats,
            pattern,
        ),

        Commands::Scan {
            path,
            exclude,
            pattern,
            ignore_placeholders,
            format,
            exit_zero,
        } => commands::scan::run(
            path,
            exclude,
            pattern,
            ignore_placeholders,
            format,
            exit_zero,
            cli.verbose,
        ),

        Commands::Diff {
            env,
            example,
            show_values,
            format,
            reverse,
            ignore_keys,
            with_stats,
            interactive,
        } => {
            match commands::diff::run(
                env,
                example,
                show_values,
                format,
                reverse,
                cli.verbose,
                ignore_keys,
                with_stats,
                interactive,
            ) {
                Ok(exit_code) => std::process::exit(exit_code),
                Err(e) => {
                    eprintln!("{} {}", "Error:".on_red().bold(), e);
                    std::process::exit(2);
                }
            }
        }

        Commands::Convert {
            env,
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
                        eprintln!(
                            "{} Invalid transform '{}', ignoring",
                            "⚠️".yellow(),
                            unknown
                        );
                    }
                    None
                }
            });

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
            args.direction,
            args.placeholder,
            cli.verbose,
            args.dry_run,
            args.force,
            args.template_config.clone(),
            args.naming_policy,
        ),

        Commands::Template { input, output, env } => {
            commands::template::run(input, output, env, cli.verbose)
        }

        #[cfg(feature = "backup")]
        Commands::Backup { env, output } => commands::backup::run(env, output, cli.verbose),

        #[cfg(feature = "backup")]
        Commands::Restore {
            backup,
            output,
            dry_run,
        } => commands::restore::run(backup, output, cli.verbose, dry_run),

        Commands::Doctor { path, verbose } => evnx::commands::doctor::run(path, verbose),

        Commands::Completions { shell } => commands::completions::run(shell),
    }
}
