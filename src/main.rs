//! evnx CLI entry point.

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use evnx::cli::{Cli, Commands, SpecAction};
use evnx::commands;
use evnx::core::converter::KeyTransform;
use evnx::core::error::{report, EvnxError};
use evnx::core::verdict::Verdict;

/// Resolve which `.env.<name>` a command operates on, or fail with
/// [`EvnxError::Config`] (exit 2).
///
/// ⚠️ **Not a bare `?` into `main`'s `Result`**, for the same reason `config::load`
/// below is not. A `--env-name` that names a file which does not exist is
/// "evnx could not run", and surfaced as a plain error it would exit **1** — which
/// `validate` uses to mean *invalid*, `diff` to mean *differences found*, and
/// `sync --check` to mean *drifted*.
///
/// So a typo in a CI flag reported a finding rather than a broken setup, and the
/// pipeline branched on it as though the answer were real. `scan` is unaffected
/// only because it has no `--env-name` to mistype.
///
/// 2 is what every command with a three-value contract already uses for "no
/// verdict", so the answer is the same whichever command was asked.
use evnx::utils::path::label;

/// The `-C, --dir` folder as the commands take it: the current folder when unset.
fn project_dir(dir: evnx::cli::args::DirArgs) -> String {
    dir.dir
        .map_or_else(|| ".".to_string(), |p| p.to_string_lossy().into_owned())
}

fn resolve_env(
    dir: &Path,
    env: Option<&str>,
    env_name: Option<&str>,
    configured: Option<&str>,
) -> Result<String, EvnxError> {
    evnx::core::env_name::select(dir, env, env_name, configured).map_err(EvnxError::Config)
}

/// The env file and template for a command that compares the two
/// (`core::resolve`, WP-5), with the `files` line on stderr when the pairing
/// is not the plain default (D3). A bad name is "no verdict", as above.
fn resolve_files(
    env: Option<&str>,
    env_name: Option<&str>,
    example: Option<&str>,
    cfg: &evnx::core::config::Config,
    need: evnx::core::resolve::Need,
    quiet: bool,
    verbose: bool,
) -> Result<evnx::core::resolve::Resolved, EvnxError> {
    use evnx::core::resolve::{resolve, Flags};
    let flags = Flags {
        env,
        env_name,
        example,
    };
    let r = resolve(Path::new("."), flags, cfg, need).map_err(EvnxError::Config)?;
    if let Some(line) = r.line(verbose) {
        evnx::utils::ui::files_line(&line, quiet);
    }
    Ok(r)
}

/// Print clap's own message (usage error, `--help`, `--version`) and give its
/// code: 2 for an error, 0 for the others.
fn clap_done(e: clap::Error) -> Result<Verdict, EvnxError> {
    let _ = e.print();
    if e.use_stderr() {
        Err(EvnxError::Said)
    } else {
        Ok(Verdict::Clean)
    }
}

/// `Ok(())` is a clean run.
fn clean(_: ()) -> Verdict {
    Verdict::Clean
}

/// The only thing `main` does: run, and let [`report`] turn the result into a
/// code. Nothing below it calls `process::exit` (WP-9).
fn main() -> ExitCode {
    // Before anything prints: a closed stdout ends evnx quietly (exit 141)
    // instead of panicking, and Ctrl-C at a prompt gives the cursor back.
    evnx::utils::term::exit_quietly_on_broken_pipe();
    evnx::utils::term::restore_cursor_on_interrupt();

    report(run())
}

fn run() -> Result<Verdict, EvnxError> {
    // A removed spelling stops here, before clap: the message names what to
    // type instead, and nothing has been read or written.
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(message) = evnx::cli::removed::check(&argv) {
        eprintln!("{message}");
        return Err(EvnxError::Said);
    }

    // Parsed from the matches, not `Cli::parse()`, so that a flag with a default
    // can still be asked "did the person type it?" (`doctor --format`).
    //
    // A usage error, `--help` and `--version` are clap's to print; its code
    // (2, 0, 0) is returned, not exited with.
    let matches = match evnx::cli::exit_help::command().try_get_matches() {
        Ok(m) => m,
        Err(e) => return clap_done(e),
    };
    let cli = match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => {
            return clap_done(e.format(&mut evnx::cli::exit_help::command()));
        }
    };
    evnx::utils::ui::set_quiet(cli.quiet);
    // `scan --exit-only` (WP-13, D4): the exit code is the whole answer, so
    // nothing printed from here on, a config warning or an error included, may
    // reach either stream. A usage error was already printed by clap above.
    let scan_flags = match &cli.command {
        Commands::Scan {
            action: None,
            strict,
            exit_only,
            policy,
            severity,
            ..
        } => Some((
            *strict,
            *exit_only,
            policy.clone(),
            severity.map(|s| s.as_str().to_string()),
        )),
        _ => None,
    };
    if matches!(scan_flags, Some((_, true, _, _))) {
        evnx::utils::term::silence_output();
    }

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
    // ⚠️ Not `?` into `main`: that is exit 1, "findings". It is `Io`, exit 2.
    let here = std::env::current_dir().map_err(|source| EvnxError::Io {
        path: PathBuf::from("the current directory"),
        source,
    })?;
    // ⚠️ Not `?`. A `.evnx.toml` that will not parse is "evnx could not run",
    // and `?` would surface it through `main`'s `Result` as exit **1** — which
    // for `scan` is the code that means *secrets found*, and for `sync --check`
    // means *drifted*. A malformed config would have read as a finding.
    //
    // 2 is the code every command with a three-value contract already uses for
    // "no verdict", so a broken config reports the same thing everywhere rather
    // than impersonating whatever failure that command happens to number 1.
    //
    // ⚠️ Not read at all by the commands that describe the binary rather than
    // the project (UP-3): a half-edited `.evnx.toml` stopped `evnx update` and
    // `evnx completions bash` with exit 2, and a valid one put a `config` line
    // in front of the completion script.
    let describes_the_binary = matches!(
        cli.command,
        Commands::Update { .. } | Commands::Surface { .. } | Commands::Completions { .. }
    );
    // `scan --no-config`: nothing from the repository's `.evnx.toml` applies.
    let no_config = matches!(
        cli.command,
        Commands::Scan {
            no_config: true,
            action: None,
            ..
        }
    );
    let loaded = if describes_the_binary || no_config {
        evnx::core::config::Loaded::default()
    } else {
        match evnx::core::config::load(&here) {
            Ok(loaded) => loaded,
            Err(e) => return Err(EvnxError::Config(e)),
        }
    };
    if no_config && !cli.quiet {
        eprintln!("  config    ignored (--no-config): built-in defaults");
    }
    // `scan --strict` (D10): an unknown key is "evnx could not run", exit 2.
    // `scan --policy` / `EVNX_POLICY` (WP-13): read after `.evnx.toml`, and
    // checked against it and the flags before any file is scanned.
    if let Some((strict, _, policy, severity)) = &scan_flags {
        if *strict && !loaded.unknown.is_empty() {
            return Err(EvnxError::Config(anyhow::anyhow!(
                "--strict: unknown key in the config\n  {}",
                loaded.unknown.join("\n  ").replace(" (ignored)", "")
            )));
        }
        if let Some(file) = policy.as_deref().filter(|f| !f.is_empty()) {
            let policy = evnx::core::policy::Policy::load(Path::new(file)).and_then(|p| {
                p.check(&loaded.config.scan, severity.as_deref())
                    .map(|()| p)
            });
            if let Err(e) = policy {
                return Err(EvnxError::Config(e));
            }
            if !cli.quiet {
                eprintln!(
                    "  policy    {}",
                    evnx::utils::ui::short_path(Path::new(file))
                );
            }
        }
    }
    if let Some(source) = &loaded.source {
        let overrides = loaded.config.security_overrides();
        // I-060: `-q` is findings only — except, for `scan`, the line saying a
        // committed config weakens the scan. Every override is a `[scan]` or
        // `[vars] secret` setting, so for any other command it stays hidden.
        let quiet =
            cli.quiet && !(matches!(cli.command, Commands::Scan { .. }) && !overrides.is_empty());
        evnx::utils::ui::config_banner(source, &overrides, quiet);
    }
    // Shown under `-q` for `scan`, like the overrides line above: a misspelt
    // `severty` is a setting that did not apply (D10).
    let warn_quiet = cli.quiet && !matches!(cli.command, Commands::Scan { .. });
    for warning in &loaded.warnings {
        evnx::utils::ui::config_warning(warning, warn_quiet);
    }
    // D2, FLD-11: outside a git repository a parent folder's config is not
    // used; say which, under -v, so a missing policy is explainable.
    if let (Some(parent), true, false) = (&loaded.ignored_parent, cli.verbose, cli.quiet) {
        eprintln!(
            "  config    {} not used: no git repository here, so only this folder's .evnx.toml applies",
            evnx::utils::ui::short_path(parent)
        );
    }
    let config_source = loaded.source.clone();
    let cfg = loaded.config;

    // Route to command handler
    // ⚠️ Bound, not returned. See the exit-code handling at the end of `run`.
    let outcome: anyhow::Result<Verdict> = match cli.command {
        Commands::Init {
            dir,
            confirm,
            blueprint,
            with,
            list_components,
            detect,
            from_source,
            force,
            env_name,
            dry,
            output,
            here,
            no_contract,
        } => {
            let path = project_dir(dir);
            let yes = confirm.yes;
            let dry_run = dry.dry_run;
            let json = output.resolved() == evnx::cli::args::Format::Json;
            // The target's own config, before anything is written (I-298).
            // `--list-components` is about evnx, not a project; `--here`
            // takes nothing from a config above the folder.
            let own = Path::new(&path).join(evnx::core::config::PROJECT_FILE);
            let target_config = if list_components || (here && !own.is_file()) {
                evnx::core::config::Config::default()
            } else {
                match commands::init::target_config(Path::new(&path), config_source.as_deref()) {
                    Ok(loaded) => loaded.config,
                    Err(e) => return Err(EvnxError::Config(e)),
                }
            };
            commands::init::run(
                path,
                yes,
                force,
                blueprint,
                with,
                list_components,
                detect,
                from_source,
                cli.verbose,
                target_config,
                dry_run,
                json,
                env_name,
                here,
                no_contract,
            )
            .map(clean)
        }

        Commands::Add {
            target,
            list,
            dry,
            output,
            dir,
            confirm,
            env_name,
            example,
            no_template,
        } => commands::add::run(
            target,
            commands::add::Options {
                path: project_dir(dir),
                yes: confirm.yes,
                list,
                dry_run: dry.dry_run,
                json: output.resolved() == evnx::cli::args::Format::Json,
                verbose: cli.verbose,
                env_name,
                example,
                no_template,
            },
        )
        .map(clean),

        Commands::Remove {
            names,
            dir,
            dry,
            confirm,
            output,
            env_name,
            template,
            contract,
            all,
            ignore_missing,
            no_backup,
            allow_dangling,
            all_names,
            check,
        } => {
            let opts = commands::remove::Options {
                names,
                path: project_dir(dir),
                env_name,
                template,
                contract,
                all,
                dry_run: dry.dry_run,
                yes: confirm.yes,
                ignore_missing,
                no_backup,
                allow_dangling,
                all_names,
                json: output.resolved() == evnx::cli::args::Format::Json,
                verbose: cli.verbose,
                check,
            };
            if check {
                commands::remove::check::run(opts)
            } else {
                commands::remove::run(opts).map(clean)
            }
        }

        Commands::Set {
            assignment,
            env,
            dir,
            if_missing,
            dry,
            confirm,
            output,
        } => commands::set::run(commands::set::Options {
            assignment,
            dir: project_dir(dir),
            env: env.env,
            env_name: env.env_name,
            if_missing,
            dry_run: dry.dry_run,
            yes: confirm.yes,
            json: output.resolved() == evnx::cli::args::Format::Json,
        })
        .map(clean),

        Commands::Get {
            name,
            env,
            dir,
            exists,
            reveal,
            output,
        } => commands::get::run(commands::get::Options {
            name,
            dir: project_dir(dir),
            env: env.env,
            env_name: env.env_name,
            exists,
            reveal,
            json: output.resolved() == evnx::cli::args::Format::Json,
        }),

        Commands::Validate {
            files: file_args,
            input_format,
            strict,
            fix,
            dry,
            confirm: _,
            rotate_weak_secrets,
            output,
            exit_zero,
            ignore,
            validate_formats,
            fail_on,
        } => {
            let flags = file_args.flags();
            let files = resolve_files(
                flags.env,
                flags.env_name,
                flags.example,
                &cfg,
                evnx::core::resolve::Need::Existing,
                cli.quiet,
                cli.verbose,
            )?;
            commands::validate::run(commands::validate::Options {
                // RM-32: `./.env` in every message and report.
                env: label(&files.env),
                input_format,
                example: label(&files.template),
                strict: evnx::core::config::any(strict, cfg.validate.strict),
                fix,
                dry_run: dry.dry_run,
                rotate_weak_secrets,
                format: output.resolved(),
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
            })
        }

        // C6: `evnx scan rules …`.
        Commands::Scan {
            action: Some(evnx::cli::ScanAction::Rules { command }),
            ..
        } => {
            use evnx::cli::RulesCommand;
            let project_root = config_source
                .as_deref()
                .and_then(|p| p.parent())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| here.clone());
            let configured = cfg.scan.patterns.clone().unwrap_or_default();
            match command {
                RulesCommand::List { category, output } => commands::scan::rules_cmd::list(
                    &project_root,
                    &configured,
                    category.as_deref(),
                    output.resolved() == evnx::cli::args::Format::Json,
                )
                .map(clean),
                RulesCommand::New { id, personal } => {
                    commands::scan::rules_cmd::new(&project_root, &id, personal)
                        .map(|_| Verdict::Clean)
                }
                RulesCommand::Add {
                    source,
                    from,
                    personal,
                } => {
                    commands::scan::packs_cmd::add(&project_root, &source, from.as_str(), personal)
                        .map(clean)
                }
                RulesCommand::Update {
                    name,
                    personal,
                    confirm,
                } => commands::scan::packs_cmd::update(
                    &project_root,
                    name.as_deref(),
                    personal,
                    confirm.yes,
                )
                .map(clean),
                RulesCommand::Remove { name, personal } => {
                    commands::scan::packs_cmd::remove(&project_root, &name, personal).map(clean)
                }
                RulesCommand::Test { file } => {
                    commands::scan::rules_cmd::test(&project_root, file.as_deref())
                }
            }
        }

        Commands::Scan {
            action: Some(evnx::cli::ScanAction::Hook { command }),
            ..
        } => {
            use evnx::cli::HookCommand;
            match command {
                HookCommand::Install => {
                    commands::scan::hook::install(&here).map(|_| Verdict::Clean)
                }
                HookCommand::Uninstall => commands::scan::hook::uninstall(&here).map(clean),
                HookCommand::Status => commands::scan::hook::status(&here).map(clean),
            }
        }

        Commands::Scan {
            action: None,
            path,
            exclude,
            max_file_size,
            extensions,
            pattern,
            ignore_placeholders,
            no_ignore_placeholders,
            severity,
            confidence,
            baseline,
            update_baseline,
            rule_files,
            output,
            exit_zero,
            include_ignored,
            staged,
            no_config,
            follow_symlinks,
            include_dir,
            policy: _,
            strict: _,
            exit_only: _,
        } => {
            let builtins = cfg.scan.builtins;
            let repo_settings = commands::scan::models::RepoSettings {
                exclude: cfg.scan.exclude.clone().unwrap_or_default(),
                ignore_placeholders: cfg.scan.ignore_placeholders == Some(true),
                extensions: cfg.scan.extensions.clone(),
                max_file_size: match cfg.scan.max_file_size.as_ref().map(|s| s.bytes()) {
                    Some(Ok(b)) if b < evnx::commands::scan::filters::MAX_WALK_BYTES => Some(b),
                    _ => None,
                },
                secret_false: cfg
                    .vars
                    .iter()
                    .filter(|(_, v)| v.secret == Some(false))
                    .map(|(k, _)| k.clone())
                    .collect(),
            };
            commands::scan::run(commands::scan::ScanOptions {
                paths: path,
                exclude: evnx::core::config::extend(
                    exclude,
                    // Read from the folder holding `.evnx.toml` (I-333).
                    match (
                        cfg.scan.exclude,
                        config_source.as_deref().and_then(Path::parent),
                    ) {
                        (Some(patterns), Some(dir)) => {
                            Some(evnx::core::config::rebase_exclude(patterns, dir, &here))
                        }
                        (patterns, _) => patterns,
                    },
                ),
                // Additive, like `exclude` above: `--pattern` is this run's rule and
                // `[[scan.patterns]]` is the project's standing set.
                disable: cfg.scan.disable.clone().unwrap_or_default(),
                patterns: commands::scan::patternset::merge(
                    pattern,
                    cfg.scan.patterns,
                    builtins.unwrap_or(true),
                    cfg.scan.disable,
                ),
                // `--no-ignore-placeholders` can undo a project's `true` (I-057).
                ignore_placeholders: evnx::core::config::pick(
                    evnx::core::config::either(ignore_placeholders, no_ignore_placeholders),
                    cfg.scan.ignore_placeholders,
                    false,
                ),
                // The flag beats `[scan] max_file_size`; a bad config value is
                // "could not run", like a bad `--max-file-size` (I-061).
                max_file_size: match max_file_size {
                    Some(bytes) => Some(bytes),
                    None => match cfg.scan.max_file_size.as_ref().map(|s| s.bytes()) {
                        Some(Ok(bytes)) => Some(bytes),
                        Some(Err(e)) => {
                            return Err(EvnxError::Config(anyhow::anyhow!(
                                ".evnx.toml [scan] max_file_size: {e}"
                            )))
                        }
                        None => None,
                    },
                },
                severity: evnx::core::config::pick(
                    severity.map(|s| s.as_str().to_string()),
                    cfg.scan.severity,
                    "low".to_string(),
                ),
                // D5: short values under secret names need the floor named,
                // not merely defaulted to `low`.
                short_values: (confidence.is_some() || cfg.scan.confidence.is_some())
                    && evnx::core::config::pick(
                        confidence.map(|c| c.as_str().to_string()),
                        cfg.scan.confidence.clone(),
                        "low".to_string(),
                    )
                    .parse::<evnx::core::secretscan::Confidence>()
                    .is_ok_and(|c| c == evnx::core::secretscan::Confidence::Low),
                confidence: evnx::core::config::pick(
                    confidence.map(|c| c.as_str().to_string()),
                    cfg.scan.confidence,
                    "low".to_string(),
                ),
                baseline,
                update_baseline,
                rule_files,
                project_root: config_source
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| here.clone()),
                format: match output.resolved() {
                    evnx::cli::args::ScanFormat::Pretty => commands::scan::OutputFormat::Pretty,
                    evnx::cli::args::ScanFormat::Json => commands::scan::OutputFormat::Json,
                    evnx::cli::args::ScanFormat::Sarif => commands::scan::OutputFormat::Sarif,
                    evnx::cli::args::ScanFormat::Github => commands::scan::OutputFormat::Github,
                },
                exit_zero,
                verbose: cli.verbose,
                spec: cfg.vars.clone(),
                // The flag wins; either way the run announces the limit (DEC-U8-2).
                extensions: extensions.map(|e| (e, "--extensions")).or_else(|| {
                    cfg.scan
                        .extensions
                        .clone()
                        .map(|e| (e, ".evnx.toml [scan] extensions"))
                }),
                include_ignored,
                staged,
                follow_symlinks,
                include_dirs: include_dir,
                no_config,
                config_files: config_source
                    .as_deref()
                    .map(|p| vec![evnx::utils::ui::short_path(p)])
                    .unwrap_or_default(),
                builtins_disabled: builtins == Some(false),
                repo_settings,
            })
        }

        Commands::Diff {
            files: file_args,
            against,
            values,
            show_values,
            output,
            reverse,
            ignore_keys,
            fail_on,
            skip_without_env,
            with_stats,
            interactive,
        } => {
            use commands::diff::{Compare, DiffOptions, FailOn};
            let here = Path::new(".");
            let format = output.resolved().to_string();
            let flags = file_args.flags();
            let files = resolve_files(
                flags.env,
                flags.env_name,
                flags.example,
                &cfg,
                evnx::core::resolve::Need::Existing,
                cli.quiet || against.is_some(),
                cli.verbose && against.is_none(),
            )?;
            let env = files.env.clone();
            // The right-hand side, and with it what is compared (I-078):
            // `--against` names an environment, so values are compared too;
            // otherwise it is the template — `--example`, `[defaults] example`
            // or `.env.example`, by the rule validate, sync and spec use — and
            // only key names are, unless `--values`. `env_name` is not
            // consulted here: it names the *left* side.
            let (example, compare) = match against.as_deref() {
                Some(name) => (resolve_env(here, None, Some(name), None)?, Compare::Values),
                None if values => (files.template.clone(), Compare::Values),
                None => (files.template.clone(), Compare::Keys),
            };
            if let Err(e) =
                commands::diff::check_flags(&format, with_stats, interactive, compare, &fail_on)
            {
                return Err(EvnxError::Config(e));
            }
            // ⚠️ Errors print `{:#}` through `report`: with `{}` only the
            // outermost context printed, so `diff` reported "Failed to parse
            // .env.example" and dropped the "Invalid format at line 1" that says
            // what to fix. It also lacked the "No verdict" line every other
            // command prints (U4).
            commands::diff::run(DiffOptions {
                env: label(&env),
                example: label(&example),
                compare,
                show_values,
                format,
                reverse,
                verbose: cli.verbose,
                ignore_keys: evnx::core::config::extend(ignore_keys, cfg.diff.ignore_keys),
                fail_on: FailOn::from_names(&fail_on),
                with_stats,
                interactive,
                skip_without_env,
            })
        }

        Commands::Convert {
            env: env_args,
            to,
            output,
            force,
            gitignore,
            no_gitignore,
            include,
            exclude,
            only,
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
            terraform_declarations,
            confirm,
            dry,
        } => {
            let to = to.map(|t| t.format_name());
            // ⚠️ Checked before anything is read: an unknown --transform was
            // dropped in silence (I-090), and a --prefix with shell syntax
            // reached the generated scripts (I-263).
            let transform = match transform.as_deref().map(KeyTransform::parse).transpose() {
                Ok(t) => t,
                Err(e) => return Err(EvnxError::Config(e)),
            };
            let targets = evnx::formats::Targets {
                project,
                vault_name,
                heroku_app,
                name,
                namespace,
                terraform_declarations,
            };
            let early = evnx::core::args::check_opt(
                "--prefix",
                prefix.as_deref(),
                evnx::core::args::Name::VarPrefix,
            )
            .and_then(|()| targets.check(to.and_then(evnx::formats::find)));
            if let Err(e) = early {
                return Err(EvnxError::Config(e));
            }

            let env = resolve_env(
                Path::new("."),
                env_args.env.as_deref(),
                env_args.env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            )?;
            // `--yes` answers the .gitignore question with yes, as `--gitignore`
            // does; an explicit `--no-gitignore` still wins.
            let gitignore = gitignore || (confirm.yes && output.is_some() && !no_gitignore);
            let config = commands::convert::ConvertConfig::builder()
                .env(env)
                .target_format(to)
                .output_path(output)
                .include(include)
                .exclude(exclude)
                .only(only)
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
                .yes(confirm.yes)
                .dry_run(dry.dry_run)
                .build();

            // No "Convert command failed:" in front: every other command's
            // error stands on its own, and this one nested three deep (I-091).
            commands::convert::run(config).map(clean)
        }

        // ── Migrate ───────────────────────────────────────────────────────────
        //
        // The variant holds Box<MigrateOptions> so the Commands enum stays
        // small on the stack (~8 bytes for this arm vs. ~435 bytes inline).
        // All fields are accessed via `opts.` after auto-deref.
        #[cfg(feature = "migrate")]
        Commands::Migrate(opts) => {
            // The flag still works; the shell history and the process list are
            // where it leaks (I-108). Said once, whatever the destination.
            if opts.github_token.is_some() {
                eprintln!(
                    "warning: --github-token puts the token in your shell history and process list; set GITHUB_TOKEN instead"
                );
            }
            commands::migrate::run(commands::migrate::MigrateArgs {
                from: opts.from.map(|f| f.slug().to_string()),
                // I-031, S-M1: the file every other command would read. Migrate
                // took `.env` whatever `[defaults] env_name` said, so with
                // `env_name = "staging"` it pushed development values while
                // `validate`, `diff` and `sync` all checked `.env.staging`. The
                // process environment has no file, so nothing to resolve there.
                source_file: if opts.from == Some(evnx::cli::MigrateSource::Environment) {
                    opts.files.env.clone().unwrap_or_default()
                } else {
                    resolve_env(
                        Path::new("."),
                        opts.files.env.as_deref(),
                        opts.files.env_name.as_deref(),
                        cfg.defaults.env_name.as_deref(),
                    )?
                },
                env_name: opts.files.env_name.clone(),
                to: opts.to.map(|t| t.slug().to_string()),
                dry_run: opts.dry_run.dry_run,
                print_only: opts.print_only,
                output: opts.output.clone(),
                force: opts.force,
                gitignore: evnx::utils::protect::GitignoreMode::from_flags(
                    opts.gitignore,
                    opts.no_gitignore,
                ),
                skip_existing: opts.skip_existing,
                overwrite: opts.overwrite,
                yes: opts.confirm.yes,
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
        }

        // ⚠️ `[sync] naming_policy` is checked even when the flag overrides it,
        // as `[validate] fail_on` is (I-038, I-121): a typo in the file is not
        // hidden by a flag that happens to be passed today.
        // `--prune` is its own mode (N14-11): it compares the template with
        // every environment file, so none of sync's file or mode flags apply
        // (clap refuses the combinations).
        Commands::Sync { args } if args.prune => {
            commands::remove::prune::run(commands::remove::prune::PruneOptions {
                path: ".".to_string(),
                example: resolve_files(
                    None,
                    None,
                    args.files.example.as_deref(),
                    &cfg,
                    evnx::core::resolve::Need::MayCreate,
                    cli.quiet,
                    cli.verbose,
                )?
                .template,
                declared: cfg.vars.keys().cloned().collect(),
                dry_run: args.dry.dry_run,
                yes: args.confirm.yes,
            })
            .map(clean)
        }

        Commands::Sync { args } => {
            let configured = cfg
                .sync
                .naming_policy
                .as_deref()
                .map(evnx::cli::NamingPolicy::from_config)
                .transpose()
                .with_context(|| {
                    format!("{} [sync] naming_policy", evnx::core::config::PROJECT_FILE)
                })?;
            let flags = args.files.flags();
            let files = resolve_files(
                flags.env,
                flags.env_name,
                flags.example,
                &cfg,
                evnx::core::resolve::Need::Existing,
                cli.quiet,
                cli.verbose,
            )?;
            commands::sync::run(
                files.env,
                files.template,
                args.direction.or(args.to.map(Into::into)),
                false,
                cli.verbose,
                args.dry.dry_run,
                args.confirm.yes,
                args.check,
                args.output.resolved().to_string(),
                args.template_config.clone(),
                evnx::core::config::pick(
                    args.naming_policy,
                    configured,
                    evnx::cli::NamingPolicy::Warn,
                ),
                cfg.vars.clone(),
                args.skip_without_env,
            )
        }

        Commands::Align { args } => {
            let evnx::cli::AlignArgs {
                example,
                env,
                env_name,
                dry_run,
                check,
                output,
                no_backup,
            } = args;
            // Each file paired with its template the way every other command
            // pairs them (core::resolve): `--env` files first, then each
            // `--env-name`; neither given means `[defaults] env_name` or `.env`.
            let pair = |env: Option<&str>, env_name: Option<&str>| {
                let r = resolve_files(
                    env,
                    env_name,
                    example.as_deref(),
                    &cfg,
                    evnx::core::resolve::Need::Existing,
                    cli.quiet,
                    cli.verbose,
                )?;
                Ok::<_, EvnxError>(commands::align::Target {
                    env: r.env,
                    template: r.template,
                })
            };
            let mut targets = Vec::new();
            for e in &env {
                targets.push(pair(Some(e), None)?);
            }
            for n in &env_name {
                targets.push(pair(None, Some(n))?);
            }
            if targets.is_empty() {
                targets.push(pair(None, None)?);
            }
            commands::align::run(commands::align::Options {
                targets,
                mode: if check {
                    commands::align::Mode::Check
                } else if dry_run {
                    commands::align::Mode::DryRun
                } else {
                    commands::align::Mode::Write
                },
                json: output.resolved() == evnx::cli::args::Format::Json,
                no_backup,
            })
        }

        // Commands::Template { input, output, env } => {
        //     commands::template::run(input, output, env, cli.verbose)
        // }
        Commands::Template {
            input,
            output,
            env: env_args,
            confirm,
            gitignore,
            no_gitignore,
            allow_unresolved,
            strict,
            mode: file_mode,
            mkdir,
            dry: dry_args,
            force,
            follow_symlinks,
        } => {
            // RM-14 (DEC-T2): refusing is the default; --strict asked for it.
            if strict {
                evnx::utils::ui::info(
                    "--strict is the default since 0.11.0 and is no longer needed; \
                     it is removed in 1.0",
                );
            }
            let dry_run = dry_args.dry_run;
            // `--yes` answers the .gitignore question with yes, as `--gitignore`
            // does; an explicit `--no-gitignore` still wins.
            let gitignore = gitignore || (confirm.yes && output != "-" && !no_gitignore);
            let mode = evnx::utils::protect::GitignoreMode::from_flags(gitignore, no_gitignore);
            let env = resolve_env(
                Path::new("."),
                env_args.env.as_deref(),
                env_args.env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            )?;
            commands::template::run_options(
                input,
                output,
                env,
                commands::template::Options {
                    verbose: cli.verbose,
                    gitignore: mode,
                    strict: !allow_unresolved,
                    file_mode,
                    mkdir,
                    dry_run,
                    force,
                    follow_symlinks,
                },
            )
            .map(clean)
        }

        #[cfg(feature = "backup")]
        Commands::Backup {
            env,
            env_name,
            output,
            password_file,
            keep,
            verify,
        } => commands::backup::run(
            resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            )?,
            output,
            cli.verbose,
            password_file,
            evnx::core::config::pick(keep, cfg.backup.keep, 3),
            verify,
        )
        .map(clean),

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
        } => commands::restore::run(
            backup,
            commands::restore::Options {
                output,
                verbose: cli.verbose,
                dry_run,
                inspect,
                password_file,
                yes,
                gitignore: evnx::utils::protect::GitignoreMode::from_flags(gitignore, no_gitignore),
            },
        )
        .map(clean),

        #[cfg(feature = "cloud")]
        Commands::Auth { command, server } => {
            evnx::cloud::run_auth(command, server.as_deref(), cli.verbose).map(clean)
        }

        #[cfg(feature = "cloud")]
        Commands::Vault { command, server } => {
            evnx::cloud::run_vault(command, server.as_deref(), cli.verbose).map(clean)
        }

        #[cfg(feature = "cloud")]
        Commands::Org { command, server } => {
            evnx::cloud::run_org(command, server.as_deref(), cli.verbose).map(clean)
        }

        #[cfg(feature = "cloud")]
        // `pull` declined: already said, and 4 as in `restore` (D6): `classify`.
        Commands::Cloud { command, server } => {
            evnx::cloud::run(command, server.as_deref(), cli.verbose)
        }

        // `diff` found differences: 1, as `cloud diff --local` and `diff`.
        // `pull` and `push` declined: 4, as in `cloud pull` (D6).
        #[cfg(feature = "net")]
        Commands::Platform { command } => evnx::platform::run(command),

        Commands::Doctor {
            path,
            dir,
            fix,
            strict,
            skip_without_env,
            output,
            verbose,
        } => evnx::commands::doctor::run(
            dir.dir
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or(path),
            verbose,
            // Either saying yes is enough: a flag can turn repair on, never off,
            // which is the same rule `.evnx.toml` booleans follow.
            fix,
            strict,
            // `None` when neither `--format` nor `--json` was typed, so that
            // `EVNX_OUTPUT_JSON` can decide (and an explicit `pretty` beat it).
            {
                use clap::parser::ValueSource::CommandLine;
                let doctor = matches.subcommand_matches("doctor");
                let typed = output.json
                    || doctor.and_then(|m| m.value_source("format")) == Some(CommandLine);
                typed.then(|| output.resolved() == evnx::cli::args::Format::Json)
            },
            skip_without_env,
        ),

        Commands::Spec { action } => match action {
            SpecAction::Init {
                with,
                files: file_args,
                stdout,
                merge,
                force,
            } => {
                let flags = file_args.flags();
                // ⚠️ Resolved only when the project's files are read: with
                // `--with`, a `[defaults] env_name` whose file is missing must
                // not stop a run that never reads it.
                let source = match with {
                    Some(ids) => commands::spec::Source::Components(ids),
                    None => {
                        use evnx::core::resolve::TemplateFrom;
                        // The same resolution as validate and sync (I-189).
                        let files = resolve_files(
                            flags.env,
                            flags.env_name,
                            flags.example,
                            &cfg,
                            evnx::core::resolve::Need::Existing,
                            cli.quiet,
                            cli.verbose,
                        )?;
                        commands::spec::Source::Project(commands::spec::Inputs {
                            env_named_by: flags.env.is_some().then_some("--env"),
                            env: files.env,
                            example_named_by: match files.template_from {
                                TemplateFrom::Flag => Some("--example"),
                                TemplateFrom::Configured => {
                                    Some("[defaults] example in .evnx.toml")
                                }
                                TemplateFrom::MatchedFlag | TemplateFrom::MatchedConfigured => {
                                    Some("the environment's own template")
                                }
                                TemplateFrom::Project => None,
                            },
                            example: files.template,
                        })
                    }
                };
                commands::spec::run(source, stdout, merge, force, cli.verbose).map(clean)
            }
            SpecAction::Check {
                files: file_args,
                output,
                skip_without_env,
            } => {
                let flags = file_args.flags();
                let json = output.resolved() == evnx::cli::args::Format::Json;
                if json {
                    evnx::utils::ui::set_quiet(true);
                }
                // `--skip-without-env`: a named file may be missing, and is
                // then skipped like an unnamed `.env` (SPEC-3).
                let need = if skip_without_env {
                    evnx::core::resolve::Need::MayCreate
                } else {
                    evnx::core::resolve::Need::Existing
                };
                let files = resolve_files(
                    flags.env,
                    flags.env_name,
                    flags.example,
                    &cfg,
                    need,
                    cli.quiet || json,
                    cli.verbose,
                )?;
                let named = (flags.env.is_some() || flags.env_name.is_some())
                    && !(skip_without_env && !Path::new(&files.env).exists());
                commands::spec::check(&cfg.vars, &files.template, &files.env, named, json)
            }
        },

        Commands::Run {
            env,
            env_name,
            include,
            exclude,
            allow_unsafe_names,
            mask,
            no_mask,
            prefer,
            command,
        } => {
            let env = resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            )?;
            commands::run::run(
                &env,
                include.as_deref(),
                exclude.as_deref(),
                allow_unsafe_names,
                evnx::core::inject::masking(mask, no_mask),
                cli.verbose,
                prefer,
                &command,
            )
            .map(clean)
        }

        Commands::History { output } => {
            commands::undo::history(output.resolved() == evnx::cli::args::Format::Json).map(clean)
        }

        Commands::Undo {
            file,
            confirm,
            dry,
            clear,
        } => commands::undo::run(commands::undo::Options {
            file,
            yes: confirm.yes,
            dry_run: dry.dry_run,
            clear,
        })
        .map(clean),

        Commands::Parse {
            env,
            env_name,
            output,
            values,
            show_values,
        } => {
            let env = resolve_env(
                Path::new("."),
                env.as_deref(),
                env_name.as_deref(),
                cfg.defaults.env_name.as_deref(),
            )?;
            commands::parse::run(
                &env,
                output.resolved() == evnx::cli::args::Format::Json,
                values,
                show_values,
            )
        }

        Commands::Surface { output, compact } => {
            commands::surface::run(output.resolved() == evnx::cli::args::Format::Json, compact)
                .map(clean)
        }
        Commands::Completions { shell } => commands::completions::run(shell).map(clean),
        Commands::Update {
            check,
            list,
            output,
        } => commands::update::run(
            check,
            list,
            output.resolved() == evnx::cli::args::Format::Json,
        ),
    };

    // ⚠️ An error is exit **2**, not 1.
    //
    // `scan`, `diff`, `doctor` and `sync --check` document 2 as "could not run,
    // so there is no verdict", and 1 as a real finding — *secrets found*,
    // *differences*, *unhealthy*, *drifted*. Returning the command's `Result`
    // from `main` would give Rust's default `Termination`, which numbers any
    // error 1, so `evnx validate --env ./missing.env` would report **validation
    // failed** for a file it never opened.
    //
    // So the error is classified here (`core::error::classify`: a prompt
    // answered no is 4, a cloud failure 10–15, a restore written aside 5, the
    // rest 2), and `report`, in `main`, prints it and returns the code. No
    // command exits on its own.
    outcome.map_err(EvnxError::from)
}
