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
        Commands::Init { path, yes } => {
            // Clean signature: no legacy flags
            commands::init::run(path, yes, cli.verbose)
        }

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
                    std::process::exit(2); // Distinct code for runtime errors
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
            // Parse transform string to KeyTransform enum with validation
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

            // Build config using builder pattern (pass Option values directly)
            let config = commands::convert::ConvertConfig::builder()
                .env(env) // String → impl Into<String>
                .target_format(to) // Option<String> → Option<impl Into<String>>
                .output_path(output) // Option<String> → Option<impl Into<String>>
                .include_pattern(include) // Option<String> → Option<impl Into<String>>
                .exclude_pattern(exclude) // Option<String> → Option<impl Into<String>>
                .base64(base64) // bool
                .prefix(prefix) // Option<String> → Option<impl Into<String>>
                .transform(transform_enum) // Option<KeyTransform>
                .verbose(cli.verbose) // bool
                .build();

            // Execute convert command with error context (return the Result)
            commands::convert::run(config).context("Convert command failed")
        }

        #[cfg(feature = "migrate")]
        Commands::Migrate {
            from,
            to,
            source_file,
            repo,
            secret_name,
            dry_run,
            skip_existing,
            overwrite,
            github_token,
            aws_profile,
        } => commands::migrate::run(
            from,
            to,
            source_file,
            repo,
            secret_name,
            dry_run,
            skip_existing,
            overwrite,
            github_token,
            aws_profile,
            cli.verbose,
        ),

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

        Commands::Doctor { path, verbose } => {
            // Cleanest: delegate Result handling to main's return type
            evnx::commands::doctor::run(path, verbose)
        }

        Commands::Completions { shell } => commands::completions::run(shell),
    }
}
