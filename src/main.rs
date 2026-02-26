use anyhow::Result;
use clap::Parser;
use evnx::cli::{Cli, Commands};
use evnx::commands;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up colored output
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Execute command
    match cli.command {
        Commands::Init {
            stack,
            services,
            path,
            yes,
        } => commands::init::run(stack, services, path, yes, cli.verbose),
        Commands::Validate {
            env,
            example,
            strict,
            fix,
            format,
            exit_zero,
        } => commands::validate::run(env, example, strict, fix, format, exit_zero, cli.verbose),
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
        } => commands::diff::run(env, example, show_values, format, reverse, cli.verbose),
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
        Commands::Sync {
            direction,
            placeholder,
        } => commands::sync::run(direction, placeholder, cli.verbose),
        Commands::Template { input, output, env } => {
            commands::template::run(input, output, env, cli.verbose)
        }
        #[cfg(feature = "backup")]
        Commands::Backup { env, output } => commands::backup::run(env, output, cli.verbose),
        #[cfg(feature = "backup")]
        Commands::Restore { backup, output } => commands::restore::run(backup, output, cli.verbose),
        Commands::Doctor { path } => commands::doctor::run(path, cli.verbose),
        Commands::Completions { shell } => commands::completions::run(shell),
    }
}
