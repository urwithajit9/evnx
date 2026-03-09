//! evnx CLI entry point.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

use evnx::cli::{Cli, Commands};
use evnx::commands;

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

        // Commands::Validate {
        //     env,
        //     example,
        //     strict,
        //     fix,
        //     format,
        //     exit_zero,
        // } => commands::validate::run(env, example, strict, fix, format, exit_zero, cli.verbose),
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

        // Commands::Diff {
        //     env,
        //     example,
        //     show_values,
        //     format,
        //     reverse,
        // } => commands::diff::run(env, example, show_values, format, reverse, cli.verbose),

        // src/main.rs — Update the Diff match arm:
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
        } => commands::convert::run(
            env,
            to,
            output,
            include,
            exclude,
            base64,
            prefix,
            transform,
            cli.verbose,
        ),

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

        // "forward" → sync_forward()  // .env → .env.example
        // "reverse" → sync_reverse()  // .env.example → .env

        // Commands::Sync {
        //     direction,
        //     placeholder,
        // } => commands::sync::run(direction, placeholder, cli.verbose),
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
        Commands::Restore { backup, output } => commands::restore::run(backup, output, cli.verbose),

        // Commands::Doctor { path } => commands::doctor::run(path, cli.verbose),
        Commands::Doctor { path, verbose } => {
            // Cleanest: delegate Result handling to main's return type
            evnx::commands::doctor::run(path, verbose)
        }

        Commands::Completions { shell } => commands::completions::run(shell),
    }
}
