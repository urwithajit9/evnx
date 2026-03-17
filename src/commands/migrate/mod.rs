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
//!   ├── filtering::apply_filters()
//!   ├── destinations::get_destination()          ← registry in destinations/mod.rs
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

use anyhow::Result;
use colored::Colorize;
use dialoguer::Select;

use destination::MigrationOptions;
use filtering::apply_filters;
use sources::load_secrets;

use crate::docs;
use crate::utils::ui;

// ─── Args struct ──────────────────────────────────────────────────────────────

/// All CLI flags forwarded from `main.rs` into the migrate command.
/// See `cli_patch.rs` for the clap definitions.
pub struct MigrateArgs {
    // ── Source ────────────────────────────────────────────────────────────
    pub from: Option<String>,
    pub source_file: String,

    // ── Destination ───────────────────────────────────────────────────────
    pub to: Option<String>,

    // ── Behaviour ─────────────────────────────────────────────────────────
    pub dry_run: bool,
    pub skip_existing: bool,
    pub overwrite: bool,
    pub verbose: bool,

    // ── Filtering / key transforms ────────────────────────────────────────
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub strip_prefix: Option<String>,
    pub add_prefix: Option<String>,

    // ── Destination-specific flags ────────────────────────────────────────
    pub repo: Option<String>, // GitHub owner/repo
    pub github_token: Option<String>,
    pub secret_name: Option<String>, // AWS secret name
    pub aws_profile: Option<String>,
    pub project: Option<String>,        // Doppler / Infisical project
    pub doppler_config: Option<String>, // Doppler config (dev/staging/prd)
    pub infisical_env: Option<String>,  // Infisical environment
    pub vault_name: Option<String>,     // Azure Key Vault name
    pub heroku_app: Option<String>,
    pub vercel_project: Option<String>,
    pub railway_project: Option<String>,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(args: MigrateArgs) -> Result<()> {
    if args.verbose {
        println!("{}", "Running migrate in verbose mode".dimmed());
    }

    print_banner();

    // ── Resolve source and destination ────────────────────────────────────
    let source = args.from.clone().unwrap_or_else(select_source);
    let destination = args.to.clone().unwrap_or_else(select_destination);

    // ── Load secrets from source ──────────────────────────────────────────
    let raw = load_secrets(&source, &args.source_file, args.verbose)?;

    println!(
        "\n{} Loaded {} secret{} from {}",
        "✓".green(),
        raw.len(),
        if raw.len() == 1 { "" } else { "s" },
        source,
    );

    if raw.is_empty() {
        println!("{} No secrets found to migrate.", "⚠️".yellow());
        return Ok(());
    }

    // ── Apply filters / transforms ────────────────────────────────────────
    let secrets = apply_filters(
        &raw,
        args.include.as_deref(),
        args.exclude.as_deref(),
        args.strip_prefix.as_deref(),
        args.add_prefix.as_deref(),
    );

    if secrets.len() != raw.len() {
        println!(
            "{} After filtering: {} of {} secrets remain.",
            "ℹ️".cyan(),
            secrets.len(),
            raw.len()
        );
    }

    if secrets.is_empty() {
        println!(
            "{} All secrets were filtered out — nothing to migrate.",
            "⚠️".yellow()
        );
        return Ok(());
    }

    // ── Build shared options ──────────────────────────────────────────────
    let opts = MigrationOptions {
        dry_run: args.dry_run,
        skip_existing: args.skip_existing,
        overwrite: args.overwrite,
        verbose: args.verbose,
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

    // ── Dispatch to destination registry ─────────────────────────────────
    let dest = destinations::get_destination(&destination, &args)?;
    let result = dest.migrate(&secrets, &opts)?;

    result.print_summary();
    if result.failed == 0 {
        dest.print_next_steps();
    }
    ui::print_docs_hint(&docs::MIGRATE);
    Ok(())
}

// ─── Private helpers ─────────────────────────────────────────────────────────

fn print_banner() {
    println!(
        "\n{}",
        "┌─ Migrate secrets to a new system ───────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Move from .env to cloud secret managers             │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );
}

/// Interactive source picker — stays in mod.rs because sources are not
/// user-extensible in the same way destinations are.
fn select_source() -> String {
    let options: &[(&str, &str)] = &[
        ("env-file", "env-file    — Read from .env file"),
        (
            "environment",
            "environment — Read from current environment variables",
        ),
    ];
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let idx = Select::new()
        .with_prompt("What are you migrating from?")
        .items(&labels)
        .default(0)
        .interact()
        .unwrap_or(0);
    options[idx].0.to_string()
}

/// Interactive destination picker — delegates to the registry so new
/// destinations appear automatically once added to `destinations/mod.rs`.
fn select_destination() -> String {
    let options = destinations::available_destinations();
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let idx = Select::new()
        .with_prompt("What are you migrating to?")
        .items(&labels)
        .default(0)
        .interact()
        .unwrap_or(0);
    options[idx].0.to_string()
}
