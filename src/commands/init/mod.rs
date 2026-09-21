//! Interactive project setup — generates .env.example.
//!
//! Implements breadth-first selection:
//! 1. Choose mode: Blank / Blueprint / Architect
//! 2. If Blueprint: select pre-combined stack
//! 3. If Architect: step through language → framework → services → infra
//! 4. Generate .env.example and .env with deduplicated, categorized variables

use crate::{
    docs,
    utils::ui::{print_docs_hint, print_header, print_next_steps},
};
use anyhow::Result;
use colored::*;
use dialoguer::Select;
use std::path::Path;

mod architect;
mod blank;
mod blueprint;
mod components;
mod detect;
mod shared;

pub use shared::{write_env_files, WriteOptions};

/// Main entry point for `evnx init`
///
/// # `--yes` means Blank
///
/// It used to mean Blueprint, on the reasoning that a populated template is the
/// friendlier default. Two things make that wrong for a non-interactive run.
/// It picked `blueprints[0]` out of a `HashMap`, so the stack it chose changed on
/// every invocation — six runs in one empty directory produced Laravel, Next.js,
/// Laravel, Go, Rust and MERN. And even made deterministic, guessing *someone
/// else's* stack in a Python project is a worse failure than writing nothing:
/// `--yes` is the flag a CI script reaches for, and the only defensible default
/// for a tool that has not been told the stack is an empty scaffold.
///
/// A blueprint is still available non-interactively — by name:
/// `evnx init --yes --blueprint t3_modern`.
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: String,
    yes: bool,
    force: bool,
    blueprint: Option<String>,
    with: Vec<String>,
    list_components: bool,
    detect_flag: bool,
    verbose: bool,
) -> Result<()> {
    // ⚠️ Before the header and before any project is touched. Listing is a
    // question about evnx, not about this directory — it has to work with no
    // project, no prompt and no side effects, because it is what the "unknown
    // component" error tells people to run.
    if list_components {
        return components::list();
    }

    if verbose {
        println!("{}", "Running init in verbose mode".dimmed());
    }

    print_init_header();

    // `--detect` accepts the proposal without asking, which is the same
    // no-prompt contract `--yes` carries everywhere else.
    let opts = WriteOptions {
        yes: yes || detect_flag,
        force,
    };

    // ⚠️ Detection runs before the menu, and only when nothing was asked for
    // explicitly. `--with`, `--blueprint` and `--yes` all say what the user
    // wants; proposing something else over the top of that would be worse than
    // not detecting at all.
    //
    // Nothing detected falls through to exactly the menu that was there before,
    // so a project evnx cannot read is no worse off than it was.
    let detected = if with.is_empty() && blueprint.is_none() && (!yes || detect_flag) {
        detect::detect(Path::new(&path)).unwrap_or_default()
    } else {
        Vec::new()
    };

    // An explicit --with or --blueprint selects the mode; nothing left to ask.
    let mode = if !with.is_empty() {
        Mode::Components
    } else if blueprint.is_some() {
        Mode::Blueprint
    } else if detect_flag || !detected.is_empty() {
        Mode::Detected
    } else if yes {
        Mode::Blank
    } else {
        let modes = [
            "📄 Blank (create empty .env files)",
            "🔷 Blueprint (use pre-configured stack)",
            "🏗️  Architect (build custom stack)",
        ];

        let selection = Select::new()
            .with_prompt("How do you want to start?")
            .items(&modes)
            .default(1)
            .interact()?;

        match selection {
            0 => Mode::Blank,
            1 => Mode::Blueprint,
            2 => Mode::Architect,
            _ => Mode::Blank,
        }
    };

    // Step 2: Route to handler
    match mode {
        Mode::Blank => blank::handle(path, opts, verbose)?,
        Mode::Components => components::handle(path, opts, verbose, &with)?,
        Mode::Detected => components::handle_detected(path, opts, verbose, &detected)?,
        Mode::Blueprint => self::blueprint::handle(path, opts, verbose, blueprint.as_deref())?,
        Mode::Architect => architect::handle(path, opts, verbose)?,
    }

    print_next_steps_ui();
    print_docs_hint(&docs::INIT);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Blank,
    /// An explicit component list: `--with nextjs,postgresql`.
    Components,
    /// What the project already declares, proposed for confirmation.
    Detected,
    /// A named stack, which is a shorthand for a component list.
    Blueprint,
    Architect,
}

fn print_init_header() {
    print_header(
        "evnx init",
        Some("Set up environment variables for your project"),
    );
}

fn print_next_steps_ui() {
    print_next_steps(&[
        "Edit .env and replace placeholder values",
        "Never commit .env to version control",
        "Run 'evnx validate' to check configuration",
        "Use 'evnx add' to add more variables later",
    ]);
}
