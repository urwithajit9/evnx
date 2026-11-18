//! commands/migrate/mod.rs — Entry point for `evnx migrate`
//!
//! # Responsibility
//!
//! Pure orchestration only. No destination logic lives here.
//!
//! ```text
//! run()
//!   ├── select_source()                          ← interactive, sources are stable
//!   ├── destinations::select_destination()       ← reads available_destinations()
//!   ├── sources::load_secrets()
//!   ├── ConvertOptions::prepare()                ← filter, strip, add (core::converter)
//!   ├── destinations::get_destination()          ← registry in destinations/mod.rs
//!   ├── ConvertOptions::prepare_with()           ← collisions, by the destination's naming
//!   └── dest.migrate() + print_summary() + print_next_steps()
//! ```
//!
//! # Extending
//!
//! To add a new migration destination, see `destinations/mod.rs` only.
//! This file does **not** need to change.

pub mod destination; // MigrationDestination trait + shared types
pub mod destinations; // per-platform modules + registry
pub mod filtering; // include/exclude/prefix transforms
pub mod sources; // load_secrets()

use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::Select;
use indexmap::IndexMap;

use destination::{DestinationKind, MigrationOptions, Out};
use filtering::AsIs;
use sources::load_secrets;

use crate::core::args::{self, Name};
use crate::core::converter::{Collisions, ConvertOptions};
use crate::core::parser::is_env_key;
use crate::core::verdict::Verdict;
use crate::docs;
use crate::utils::ui;

// ─── Args struct ──────────────────────────────────────────────────────────────

/// All CLI flags forwarded from `main.rs` into the migrate command.
/// See `cli_patch.rs` for the clap definitions.
pub struct MigrateArgs {
    // ── Source ────────────────────────────────────────────────────────────
    pub from: Option<String>,
    /// The `.env` to read: `--env`, else `.env.<--env-name>`, else
    /// `[defaults] env_name`, else `.env` — resolved by `main` (I-031).
    pub source_file: String,
    /// `--env-name`, kept to refuse it with `--from environment`.
    pub env_name: Option<String>,

    // ── Destination ───────────────────────────────────────────────────────
    pub to: Option<String>,

    // ── Behaviour ─────────────────────────────────────────────────────────
    pub dry_run: bool,
    /// `--print-only`: stdout holds the commands and nothing else (I-111).
    pub print_only: bool,
    /// `-o FILE`: the commands, real values, to a private file.
    pub output: Option<String>,
    pub force: bool,
    pub gitignore: crate::utils::protect::GitignoreMode,
    pub skip_existing: bool,
    pub overwrite: bool,
    pub yes: bool,
    pub verbose: bool,

    // ── Filtering / key transforms ────────────────────────────────────────
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub strip_prefix: Option<String>,
    pub add_prefix: Option<String>,
    /// Two names that become one: keep the last (I-104, DEC-PL2).
    pub allow_collisions: bool,

    // ── Destination-specific flags ────────────────────────────────────────
    pub repo: Option<String>, // GitHub owner/repo
    pub github_token: Option<String>,
    pub secret_name: Option<String>, // AWS secret name
    pub aws_profile: Option<String>,
    pub project: Option<String>, // Doppler / Infisical / GCP project
    pub doppler_config: Option<String>, // Doppler config (dev/staging/prd)
    pub infisical_env: Option<String>, // Infisical environment
    pub vault_name: Option<String>, // Azure Key Vault name
    pub heroku_app: Option<String>,
    pub vercel_project: Option<String>,
    pub railway_project: Option<String>,
    pub railway_environment: Option<String>,
    pub railway_service: Option<String>,
}

/// The flag combinations that cannot work for this source and destination,
/// refused as usage errors. Called before the banner when both are named on the
/// command line, and again once a prompt has chosen them (WP-10).
fn check_combinations(args: &MigrateArgs, source: &str, destination: &str) -> Result<()> {
    use crate::core::verdict::usage;
    if source == "environment" && args.env_name.is_some() {
        return Err(usage(
            "--env-name names a .env file, and --from environment reads the process \
             environment instead. Drop one of them.",
        ));
    }
    // I-111: `--print-only` is about printed commands. The one destination
    // that uploads prints none, so the flag could only be ignored there.
    if args.print_only && destinations::uploads(destination) {
        return Err(usage(format!(
            "--print-only applies to destinations evnx prints commands for. \
             `{destination}` uploads; drop --print-only, or preview with --dry-run."
        )));
    }
    if args.output.is_some() && destinations::uploads(destination) {
        return Err(usage(format!(
            "--output applies to destinations evnx prints commands for. \
             `{destination}` uploads; drop --output, or preview with --dry-run."
        )));
    }
    // ⚠️ `--skip-existing` and `--overwrite` are about secrets that already
    // exist at the destination, which only a destination evnx actually talks to
    // can know. The other eight print commands for you to run, and accepted
    // both flags in silence — no warning, no effect, exit 0 (F10, flagged in
    // three consecutive reviews). Refusing is better than a warning: a flag
    // that cannot work should not look like it did.
    //
    // Vercel is the one printing destination `--overwrite` means something
    // for: it decides whether the printed commands carry `--force`.
    let overwrite_applies = destinations::uploads(destination) || destination == "vercel";
    let refused = (args.skip_existing, args.overwrite && !overwrite_applies);
    if !destinations::uploads(destination) && (refused.0 || refused.1) {
        let which = match refused {
            (true, true) => "--skip-existing and --overwrite",
            (true, false) => "--skip-existing",
            _ => "--overwrite",
        };
        let only = if destination == "vercel" {
            "Only `--to github` uploads, and it honours both. For `--to vercel`, \
             `--overwrite` adds `--force` to the printed commands; `evnx platform push` \
             talks to Vercel itself."
        } else {
            "Only `--to github` uploads, and it honours both."
        };
        return Err(usage(format!(
            "{which} cannot apply to `{destination}`.\n\n\
             evnx does not contact {destination} — it prints the commands for \
             you to run, so it cannot see which secrets are already there. \
             Drop the flag, or pass the platform's own equivalent to the \
             printed command.\n\n\
             {only}"
        )));
    }
    Ok(())
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(args: MigrateArgs) -> Result<Verdict> {
    // I-267: the prefixes become part of every name, and a name is printed
    // into commands — gcp printed `gcloud secrets create X;touch PWNED;PORT`.
    // The rule convert's `--prefix` follows (I-263).
    args::check_opt("--add-prefix", args.add_prefix.as_deref(), Name::VarPrefix)?;
    args::check_opt(
        "--strip-prefix",
        args.strip_prefix.as_deref(),
        Name::VarPrefix,
    )?;

    let out = Out {
        quiet: ui::is_quiet(),
        print_only: args.print_only,
        to_file: args.output.is_some(),
    };

    if args.verbose {
        out.status("Running migrate in verbose mode".dimmed());
    }

    // ⚠️ Argument problems are said before the banner (WP-10). With both ends
    // named there is nothing to ask, so what is wrong with the flags is known
    // now; a destination's missing `--repo` or `--project` is too, when there
    // is no terminal to ask on.
    if let Some(destination) = args.to.as_deref() {
        let source = args.from.as_deref().unwrap_or("env-file");
        check_combinations(&args, source, destination)?;
        if !crate::utils::term::can_prompt() {
            destinations::get_destination(destination, &args)
                .map(drop)
                .map_err(|e| crate::core::verdict::usage(format!("{e:#}")))?;
        }
    }

    print_banner();

    // ── Resolve source and destination ────────────────────────────────────
    let source = match args.from.clone() {
        Some(s) => s,
        None => select_source()?,
    };
    let destination = match args.to.clone() {
        Some(d) => d,
        None => select_destination()?,
    };
    check_combinations(&args, &source, &destination)?;
    // Before anything is built, so a refusal writes nothing.
    if let Some(path) = args.output.as_deref() {
        crate::commands::convert::check_output(&args.source_file, path, args.force)?;
    }

    // ── Load secrets from source ──────────────────────────────────────────
    // I-103: the environment is read only with `--include` selecting from it.
    if source == "environment" {
        crate::core::envsource::require_include(args.include.as_deref())?;
    }
    let raw = load_secrets(&source, &args.source_file, args.verbose)?;

    // `source` is the source *kind* ("env-file"); the path is what says which
    // environment was read, now that `--env-name` and `[defaults] env_name`
    // can choose it (I-031).
    let what = if source == "environment" {
        "the process environment".to_string()
    } else {
        args.source_file.clone()
    };
    out.status(format!(
        "\n{} Loaded {} secret{} from {}",
        "✓".green(),
        raw.len(),
        if raw.len() == 1 { "" } else { "s" },
        what,
    ));

    if raw.is_empty() {
        // ⚠️ Not `Ok(())`. This returned 0 and printed no docs link, so a
        // migrate that moved nothing reported the same success as one that
        // moved everything — and the user's transcript in issue #13 is exactly
        // this branch. Naming the source is the part that distinguishes "the
        // file really is empty" from "I am in the wrong directory".
        ui::print_docs_hint(&docs::MIGRATE);
        anyhow::bail!(
            "{what} holds no variables, so there is nothing to migrate.\n\n\
             Check you are in the right directory, or point at another file \
             with --env. `evnx validate` will confirm what evnx can read."
        );
    }

    // ── Filter and rename: the pipeline convert uses (A-12) ──────────────
    let mut options = ConvertOptions {
        include: args.include.clone().unwrap_or_default(),
        exclude: args.exclude.clone().unwrap_or_default(),
        strip_prefix: args.strip_prefix.clone(),
        prefix: args.add_prefix.clone(),
        ..Default::default()
    };
    // I-267: `--strip-prefix` can take a whole name; heroku printed `''='x'`.
    // Since 0.13.0 (D12) a leading digit, `.` or `-` is the destination's call:
    // its row in `core::key_rules`, checked by `prepare_with` below.
    let unnamed: Vec<String> = options
        .filter_vars(&raw)
        .keys()
        .filter_map(|k| {
            let to = options.transform_key(k);
            (!is_env_key(&to)).then(|| format!("{k} → '{to}'"))
        })
        .collect();
    if !unnamed.is_empty() {
        anyhow::bail!(
            "these names are not variable names after renaming:\n  {}\n\n\
             No destination takes them. Leave them out with --exclude, or change \
             --strip-prefix. Nothing was printed or sent.",
            unnamed.join("\n  ")
        );
    }

    let renamed = options.prepare(&raw, &AsIs)?;

    // I-103: say which names leave the environment, before anything is printed.
    if source == "environment" {
        let names: Vec<&str> = renamed.iter().map(|(n, _)| n.as_str()).collect();
        ui::warning(format!(
            "Sending {} variable{} from the environment: {}",
            names.len(),
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
    }

    if renamed.len() != raw.len() {
        out.status(format!(
            "{} After filtering: {} of {} secrets remain.",
            "·".cyan(),
            renamed.len(),
            raw.len()
        ));
    }

    if renamed.is_empty() {
        // Same rule, and the same one `evnx cloud run` already follows: a
        // filter that matches nothing is a mistake in the filter, not a run
        // with zero secrets that happened to succeed.
        ui::print_docs_hint(&docs::MIGRATE);
        anyhow::bail!(
            "every one of the {} variable(s) in {} was filtered out, so \
             there is nothing to migrate.\n\n\
             Check --include / --exclude: a pattern that matches nothing leaves \
             the destination untouched.",
            raw.len(),
            args.source_file
        );
    }

    // ── Dispatch to destination registry ─────────────────────────────────
    let dest = destinations::get_destination(&destination, &args)?;

    // ── Collisions, by the destination's own naming (I-104) ──────────────
    // ⚠️ `--strip-prefix API_` on `API_X` and `X` printed one `X` and said
    // "1 of 2 secrets remain"; one value was gone. And Azure Key Vault and
    // GitHub ignore case, so `A_B` and `a_b` are one secret there.
    options.collisions = if args.allow_collisions {
        Collisions::LastWins
    } else {
        Collisions::Refuse
    };
    let secrets: IndexMap<String, String> = options
        .prepare_with(&raw, dest.as_ref(), "Nothing was printed or sent.")?
        .into_iter()
        .collect();
    options.announce_kept(&raw, dest.as_ref());

    // ── Build shared options ──────────────────────────────────────────────
    let opts = MigrationOptions {
        dry_run: args.dry_run,
        skip_existing: args.skip_existing,
        overwrite: args.overwrite,
        yes: args.yes,
        verbose: args.verbose,
        out,
        include: args.include.clone(),
        exclude: args.exclude.clone(),
        strip_prefix: args.strip_prefix.clone(),
        add_prefix: args.add_prefix.clone(),
        repo: args.repo.clone(),
        github_token: args.github_token.clone(),
        secret_name: args.secret_name.clone(),
        aws_profile: args.aws_profile.clone(),
        project: args.project.clone(),
        doppler_config: args.doppler_config.clone(),
        vault_name: args.vault_name.clone(),
        heroku_app: args.heroku_app.clone(),
        vercel_project: args.vercel_project.clone(),
        ..Default::default()
    };

    // In a terminal, without --print-only or --output, the commands are for
    // reading: values are hidden. A pipe or a file gets the real commands, as
    // it always did.
    let masked = !args.print_only
        && args.output.is_none()
        && !args.dry_run
        && dest.kind() == DestinationKind::EmitsCommands
        && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let hidden: IndexMap<String, String> = if masked {
        secrets
            .keys()
            .map(|k| (k.clone(), "<hidden>".to_string()))
            .collect()
    } else {
        IndexMap::new()
    };
    let shown = if masked { &hidden } else { &secrets };

    // `-o`: the commands are plain text for the file, never coloured.
    if args.output.is_some() {
        colored::control::set_override(false);
    }
    let result = dest.migrate(shown, &opts);
    if args.output.is_some() {
        colored::control::unset_override();
    }
    let result = result?;

    if let Some(path) = args.output.as_deref() {
        let commands = Out::take_commands();
        let written = crate::core::safe_write::write_with_mode(
            path,
            commands.as_bytes(),
            crate::core::permissions::PRIVATE,
        )
        .with_context(|| format!("Failed to write output to '{path}'"))?;
        out.result(format!("Commands written to: {path}"));
        if written.replaced {
            out.result(format!("Replaced {path}"));
        }
        crate::core::safe_write::report(&written);
        crate::utils::protect::protect_output(path, &args.gitignore)?;
    }

    result.print_summary(dest.kind(), args.dry_run, out);
    if masked {
        out.result(format!(
            "     {}",
            "values are hidden here: use --output FILE, or --print-only | sh, for the runnable form"
                .yellow()
        ));
    }
    if args.dry_run {
        // ⚠️ Deliberately *not* `print_next_steps()`. Every destination's next
        // steps are written for a migration that happened — "verify secrets
        // were set", "redeploy your project" — and printing them directly under
        // "nothing was uploaded" contradicts the line above them (F12).
        let needed = destinations::flags_a_real_run_needs(&destination, &args);
        if !needed.is_empty() {
            // The preview shows a placeholder where the name goes; say what
            // replaces it, because a real run refuses to start without it.
            out.result(format!(
                "\n{} A real run needs {}.",
                "·".cyan(),
                needed.join(" and ")
            ));
        }
        out.status("\nNext step:");
        out.status("  Re-run without --dry-run to apply this.");
    } else if result.failed == 0 {
        // Status lines: `-q` hides them (I-105), `--print-only` moves them to
        // stderr (I-111).
        dest.print_next_steps(out);
    }
    ui::print_docs_hint(&docs::MIGRATE);
    // ⚠️ A run with failed uploads is a "no" (I-106). It printed "✗ 1 failed"
    // and exited 0, so a pipeline step that migrated half its secrets passed.
    // 1, not 2: the run finished and the summary names each failure; 2 stays
    // for a run that could not start (no token, GitHub unreachable).
    Ok(if result.failed > 0 {
        Verdict::Findings
    } else {
        Verdict::Clean
    })
}

// ─── Private helpers ─────────────────────────────────────────────────────────

fn print_banner() {
    // ⚠️ This was three hand-typed box-drawing literals, and the bottom border
    // was one column longer than the other two — 56 against 55. It escaped the
    // v0.5.0 header pass because it never called `print_box`, so fixing that
    // helper's arithmetic did not reach it.
    //
    // ⚠️ This comment claimed `migrate` was "the last command still drawing a
    // box at all". It was not — `convert`'s interactive format selector had an
    // identical one, with the identical off-by-one. Both were invisible to a
    // search for `print_box`, which is what made the claim easy to make twice.
    ui::print_header(
        "evnx migrate",
        Some("convert writes a file; migrate talks to a provider"),
    );
}

/// Ask which source, or take `env-file` when there is no terminal to ask —
/// what `--from`'s help says (I-105).
///
/// ⚠️ This ended `.interact().unwrap_or(0)`, the shape `select_destination`
/// below shed: any failure of the menu became "env-file", silently. The
/// terminal decision is now made first, so a failure at the menu is an error.
fn select_source() -> Result<String> {
    if !crate::utils::term::can_prompt() {
        return Ok("env-file".to_string());
    }
    let options: &[(&str, &str)] = &[
        ("env-file", "env-file    — Read from .env file"),
        (
            "environment",
            "environment — Read from current environment variables",
        ),
    ];
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let idx = Select::with_theme(&crate::utils::prompt::MENU)
        .with_prompt("What are you migrating from?")
        .items(&labels)
        .default(0)
        .interact()
        .context("choosing what to migrate from")?;
    Ok(options[idx].0.to_string())
}

/// Interactive destination picker — delegates to the registry so new
/// destinations appear automatically once added to `destinations/mod.rs`.
/// Ask which destination, or refuse.
///
/// ⚠️ This used to end `.interact().unwrap_or(0)`. With no terminal —
/// every CI runner — `interact()` fails and `unwrap_or(0)` **silently selected
/// the first destination in the list**. So `evnx migrate --dry-run` with no
/// `--to` printed a plan for somewhere the user had never chosen and exited 0.
///
/// A missing required argument is "could not run", not "pick something".
fn select_destination() -> Result<String> {
    let options = destinations::available_destinations();

    if !crate::utils::term::can_prompt() {
        let ids: Vec<&str> = options.iter().map(|(id, _)| *id).collect();
        return Err(crate::utils::prompt::choice_needed(
            "--to",
            "evnx migrate --to <DESTINATION>",
            &ids,
        ));
    }

    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let idx = Select::with_theme(&crate::utils::prompt::MENU)
        .with_prompt("What are you migrating to?")
        .items(&labels)
        .default(0)
        .interact()
        .context("choosing a migration destination")?;
    Ok(options[idx].0.to_string())
}
