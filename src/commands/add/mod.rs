//! Add command: Append environment variables to existing .env files.
//!
//! Subcommands:
//! - `service <id>`    — Add variables for a service (e.g., postgresql)
//! - `framework <id>`  — Add variables for a framework (e.g., django)
//! - `blueprint <id>`  — Add variables from a stack blueprint
//! - `custom`          — Interactive custom variable addition

use anyhow::{bail, Result};
use colored::*;
use std::path::Path;

pub mod blueprint;
pub mod custom;
pub mod framework;
pub mod service;
pub mod shared;
use crate::{
    docs,
    utils::ui::{glyph, print_docs_hint, print_header, print_next_steps},
};
use shared::Mode;

/// Everything `evnx add` takes besides the subcommand.
pub struct Options {
    pub path: String,
    pub yes: bool,
    pub list: bool,
    pub dry_run: bool,
    pub json: bool,
    pub verbose: bool,
}

/// Main entry point for `evnx add`
pub fn run(target: Option<crate::cli::AddTarget>, opts: Options) -> Result<()> {
    if opts.list {
        if target.is_some() {
            bail!(
                "--list takes no subcommand: it lists every id. Run `evnx add --list` on its own."
            );
        }
        return list();
    }
    let Some(target) = target else {
        bail!(
            "choose what to add: `evnx add service <id>`, `framework <id>`, \
             `blueprint <id>` or `custom`.\n\nEvery id: evnx add --list"
        );
    };
    // `--json` never asks: a question would land in the middle of the
    // document a script is about to parse.
    if opts.json && !opts.yes && !opts.dry_run {
        bail!("--json needs --yes or --dry-run: it never asks.");
    }
    if opts.json {
        // Headers, hints and result lines would share stdout with the JSON.
        crate::utils::ui::set_quiet(true);
    }
    let mode = Mode {
        yes: opts.yes,
        dry_run: opts.dry_run,
        json: opts.json,
        verbose: opts.verbose,
    };

    if mode.interactive() {
        print_header(
            "evnx add",
            Some("Add environment variables to existing .env files"),
        );
    }

    let output_path = Path::new(&opts.path);

    let outcome = match target {
        crate::cli::AddTarget::Service { service } => {
            service::handle(service.as_deref(), &opts.path, &mode)?
        }
        crate::cli::AddTarget::Framework {
            language,
            framework,
        } => framework::handle(
            language.as_deref(),
            framework.as_deref(),
            output_path,
            &mode,
        )?,
        crate::cli::AddTarget::Blueprint { blueprint } => {
            blueprint::handle(blueprint.as_deref(), output_path, &mode)?
        }
        crate::cli::AddTarget::Custom {
            vars,
            desc,
            category,
            required,
        } => custom::handle(
            output_path,
            custom::Args {
                vars,
                desc,
                category,
                required,
            },
            &mode,
        )?,
    };

    // Next steps only after a write: they used to follow "Aborted." (N-12).
    if mode.interactive() && outcome == shared::Outcome::Written {
        print_next_steps(&[
            "Edit .env and replace placeholder values",
            "Run 'evnx validate' to check for issues",
            "Use 'evnx add' again to add more variables",
        ]);
    }
    if !mode.json {
        print_docs_hint(&docs::ADD);
    }
    Ok(())
}

/// `evnx add --list`: every id `add` takes (I-026, 7a). The catalogue is the
/// one `init --list-components` prints, from the same function, so the two
/// cannot drift (7f); blueprints follow.
fn list() -> Result<()> {
    crate::commands::init::components::print_catalogue()?;
    println!("\n  {}", "BLUEPRINT".bold());
    for (id, name) in crate::schema::loader::list_blueprints() {
        println!("    {:<24} {}", id, name.dimmed());
    }
    println!(
        "\n  {}  evnx add service {}   evnx add blueprint {}\n",
        glyph::ARROW.dimmed(),
        "postgresql".dimmed(),
        "t3_modern".dimmed()
    );
    Ok(())
}

/// Pick one item in a terminal, when no id was given (I-026, 7a). Without a
/// terminal, or under `--yes`, `--dry-run` or `--json`, there is no one to
/// ask: say where the ids are instead. `None`: the user pressed Esc.
pub(crate) fn pick(mode: &Mode, what: &str, items: &[(String, String)]) -> Result<Option<String>> {
    use std::io::IsTerminal;
    if !mode.interactive() || !std::io::stdin().is_terminal() {
        bail!("no {what} given.\n\nGive one, or list them all: evnx add --list");
    }
    let labels: Vec<String> = items
        .iter()
        .map(|(id, name)| format!("{name}  ({id})"))
        .collect();
    let picked = dialoguer::Select::new()
        .with_prompt(format!("Which {what}?"))
        .items(&labels)
        .default(0)
        .max_length(15)
        .interact_opt()?;
    Ok(picked.map(|i| items[i].0.clone()))
}
