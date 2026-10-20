//! evnx CLI entry point.

use anyhow::{Context, Result};
use clap::Parser;
// Only the backup and restore arms colour anything here now; the convert arm
// that printed "Invalid transform … ignoring" refuses instead (I-090).
#[cfg(feature = "backup")]
use colored::Colorize;
use std::path::Path;

use evnx::cli::{Cli, Commands, SpecAction};
use evnx::commands;
use evnx::core::converter::KeyTransform;
use evnx::core::verdict::{self, Stage};

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
        Err(e) => verdict::no_verdict(&e, Stage::BeforeRun),
    }
}

fn main() -> Result<()> {
    // Before anything prints: a closed stdout ends evnx quietly (exit 141)
    // instead of panicking, and Ctrl-C at a prompt gives the cursor back.
    evnx::utils::term::exit_quietly_on_broken_pipe();
    evnx::utils::term::restore_cursor_on_interrupt();

    let cli = Cli::parse();
    evnx::utils::ui::set_quiet(cli.quiet);

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
        Err(e) => verdict::no_verdict(&e, Stage::BeforeRun),
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

        Commands::Add {
            target,
            list,
            dry_run,
            json,
            path,
            yes,
        } => commands::add::run(
            target,
            commands::add::Options {
                path,
                yes,
                list,
                dry_run,
                json,
                verbose: cli.verbose,
            },
        ),

        Commands::Validate {
            env,
            env_name,
            example,
            strict,
            fix,
            dry_run,
            rotate_weak_secrets,
            format,
            exit_zero,
            ignore,
            validate_formats,
            fail_on,
        } => commands::validate::run(commands::validate::Options {
            env: resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            ),
            example: cfg.example_path(example),
            strict: evnx::core::config::any(strict, cfg.validate.strict),
            fix,
            dry_run,
            rotate_weak_secrets,
            format,
            exit_zero,
            // The flag wins over `[validate] fail_on`; both are checked (I-038).
            fail_on,
            fail_on_configured: cfg.validate.fail_on.clone(),
            verbose: cli.verbose,
            // Additive, as before; kept apart so an unknown type names its
            // source (I-038).
            ignore,
            ignore_configured: cfg.validate.ignore.clone().unwrap_or_default(),
            validate_formats: evnx::core::config::any(
                validate_formats,
                cfg.validate.validate_formats,
            ),
            spec: cfg.vars.clone(),
        }),

        Commands::Scan {
            path,
            exclude,
            extensions,
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
            commands::scan::patternset::merge(
                pattern,
                cfg.scan.patterns,
                cfg.scan.builtins.unwrap_or(true),
                cfg.scan.disable,
            ),
            evnx::core::config::any(ignore_placeholders, cfg.scan.ignore_placeholders),
            evnx::core::config::pick(severity, cfg.scan.severity, "low".to_string()),
            format,
            exit_zero,
            cli.verbose,
            cfg.vars.clone(),
            // The flag wins; either way the run announces the limit (DEC-U8-2).
            extensions.map(|e| (e, "--extensions")).or_else(|| {
                cfg.scan
                    .extensions
                    .clone()
                    .map(|e| (e, ".evnx.toml [scan] extensions"))
            }),
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
                // ⚠️ Through `no_verdict`, which prints `{:#}`: with `{}` only
                // the outermost context printed, so `diff` reported "Failed to
                // parse .env.example" and dropped the "Invalid format at line
                // 1" that says what to fix. It also lacked the "No verdict"
                // line every other command prints (U4).
                Err(e) => verdict::no_verdict(&e, Stage::DuringRun),
            }
        }

        Commands::Convert {
            env,
            env_name,
            to,
            output,
            force,
            gitignore,
            no_gitignore,
            include,
            exclude,
            allow_empty,
            allow_collisions,
            base64,
            prefix,
            transform,
            project,
            vault_name,
            heroku_app,
            name,
            namespace,
        } => {
            // ⚠️ Checked before anything is read: an unknown --transform was
            // dropped in silence (I-090), and a --prefix with shell syntax
            // reached the generated scripts (I-263).
            let transform = match transform.as_deref().map(KeyTransform::parse).transpose() {
                Ok(t) => t,
                Err(e) => verdict::no_verdict(&e, Stage::BeforeRun),
            };
            let targets = evnx::formats::Targets {
                project,
                vault_name,
                heroku_app,
                name,
                namespace,
            };
            let early = evnx::core::args::check_opt(
                "--prefix",
                prefix.as_deref(),
                evnx::core::args::Name::VarPrefix,
            )
            .and_then(|()| targets.check(to.as_deref().and_then(evnx::formats::find)));
            if let Err(e) = early {
                verdict::no_verdict(&e, Stage::BeforeRun);
            }

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
                .include(include)
                .exclude(exclude)
                .base64(base64)
                .prefix(prefix)
                .transform(transform)
                .targets(targets)
                .allow_empty(allow_empty)
                .allow_collisions(allow_collisions)
                .gitignore(evnx::utils::protect::GitignoreMode::from_flags(
                    gitignore,
                    no_gitignore,
                ))
                .verbose(cli.verbose)
                .force(force)
                .build();

            // No "Convert command failed:" in front: every other command's
            // error stands on its own, and this one nested three deep (I-091).
            commands::convert::run(config)
        }

        // ── Migrate ───────────────────────────────────────────────────────────
        //
        // The variant holds Box<MigrateOptions> so the Commands enum stays
        // small on the stack (~8 bytes for this arm vs. ~435 bytes inline).
        // All fields are accessed via `opts.` after auto-deref.
        #[cfg(feature = "migrate")]
        Commands::Migrate(opts) => commands::migrate::run(commands::migrate::MigrateArgs {
            from: opts.from.clone(),
            // I-031, S-M1: the file every other command would read. Migrate
            // took `.env` whatever `[defaults] env_name` said, so with
            // `env_name = "staging"` it pushed development values while
            // `validate`, `diff` and `sync` all checked `.env.staging`. The
            // process environment has no file, so nothing to resolve there.
            source_file: if opts.from.as_deref() == Some("environment") {
                opts.source_file.clone().unwrap_or_default()
            } else {
                resolve_env(
                    Path::new("."),
                    opts.source_file.as_deref(),
                    opts.env_name.as_deref(),
                    cfg.defaults.env_name.as_deref(),
                )
            },
            env_name: opts.env_name.clone(),
            to: opts.to.clone(),
            dry_run: opts.dry_run,
            print_only: opts.print_only,
            skip_existing: opts.skip_existing,
            overwrite: opts.overwrite,
            yes: opts.yes,
            verbose: cli.verbose,
            include: opts.include.clone(),
            exclude: opts.exclude.clone(),
            strip_prefix: opts.strip_prefix.clone(),
            add_prefix: opts.add_prefix.clone(),
            allow_collisions: opts.allow_collisions,
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
            railway_environment: opts.railway_environment.clone(),
            railway_service: opts.railway_service.clone(),
        })
        .map(verdict::finish),

        // ⚠️ `[sync] naming_policy` is checked even when the flag overrides it,
        // as `[validate] fail_on` is (I-038, I-121): a typo in the file is not
        // hidden by a flag that happens to be passed today.
        Commands::Sync { args } => cfg
            .sync
            .naming_policy
            .as_deref()
            .map(evnx::cli::NamingPolicy::from_config)
            .transpose()
            .with_context(|| format!("{} [sync] naming_policy", evnx::core::config::PROJECT_FILE))
            .and_then(|configured| {
                commands::sync::run(
                    resolve_env(
                        Path::new("."),
                        args.env.as_deref(),
                        args.env_name.as_deref(),
                        cfg.defaults.env_name.as_deref(),
                    ),
                    cfg.example_path(args.example.clone()),
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
                        configured,
                        evnx::cli::NamingPolicy::Warn,
                    ),
                    cfg.vars.clone(),
                    args.skip_without_env,
                )
            })
            .map(verdict::finish),

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
            mode: file_mode,
            mkdir,
            dry_run,
        } => {
            let mode = evnx::utils::protect::GitignoreMode::from_flags(gitignore, no_gitignore);
            let env = resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            );
            commands::template::run_options(
                input,
                output,
                env,
                commands::template::Options {
                    verbose: cli.verbose,
                    gitignore: mode,
                    strict,
                    file_mode,
                    mkdir,
                    dry_run,
                },
            )
        }

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

        #[cfg(feature = "backup")]
        Commands::Restore {
            backup,
            output,
            yes,
            gitignore,
            no_gitignore,
            dry_run,
            inspect,
            password_file,
        } => {
            match commands::restore::run(
                backup,
                commands::restore::Options {
                    output,
                    verbose: cli.verbose,
                    dry_run,
                    inspect,
                    password_file,
                    yes,
                    gitignore: evnx::utils::protect::GitignoreMode::from_flags(
                        gitignore,
                        no_gitignore,
                    ),
                },
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
        Commands::Org { command, server } => {
            evnx::cloud::run_org(command, server.as_deref(), cli.verbose)
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
                env_name,
                example,
                stdout,
                merge,
                force,
            } => {
                // ⚠️ Resolved only when the project's files are read: with
                // `--with`, a `[defaults] env_name` whose file is missing must
                // not stop a run that never reads it.
                let source = match with {
                    Some(ids) => commands::spec::Source::Components(ids),
                    None => commands::spec::Source::Project(commands::spec::Inputs {
                        env_named_by: env.is_some().then_some("--env"),
                        // The same resolution as validate and sync (I-189).
                        env: resolve_env(
                            Path::new("."),
                            env.as_deref(),
                            env_name.as_deref(),
                            cfg.defaults.env_name.as_deref(),
                        ),
                        example_named_by: if example.is_some() {
                            Some("--example")
                        } else if cfg.defaults.example.is_some() {
                            Some("[defaults] example in .evnx.toml")
                        } else {
                            None
                        },
                        example: cfg.example_path(example),
                    }),
                };
                commands::spec::run(source, stdout, merge, force, cli.verbose)
            }
        },

        Commands::Surface { json, compact } => commands::surface::run(json, compact),
        Commands::Completions { shell } => commands::completions::run(shell),
        Commands::Update { check, json } => commands::update::run(check, json).map(verdict::finish),
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
        verdict::no_verdict(&e, Stage::DuringRun);
    }

    Ok(())
}
