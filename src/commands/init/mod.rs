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
pub(crate) mod components;
mod contract;
pub(crate) mod detect;
mod report;
pub(crate) mod shared;
mod source;

pub use shared::{target_config, write_env_files, WriteOptions};

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
    from_source: bool,
    verbose: bool,
    config: crate::core::config::Config,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    // ⚠️ Before the header and before any project is touched. Listing is a
    // question about evnx, not about this directory — it has to work with no
    // project, no prompt and no side effects, because it is what the "unknown
    // component" error tells people to run.
    if list_components {
        return components::list();
    }

    // `--json` never asks, as for `add`: a question would land in the middle
    // of the document a script is about to parse (IN-16).
    if json && !yes && !dry_run {
        anyhow::bail!("--json needs --yes or --dry-run: it never asks.");
    }
    if json {
        // Headers, hints and result lines would share stdout with the JSON.
        crate::utils::ui::set_quiet(true);
    }
    // No confirmation under --dry-run or --json: the plan is printed instead
    // of asked about. A menu still asks under --dry-run when nothing was named.
    let no_questions = yes || json;

    // Source scanning answers a different question from the rest of `init`: not
    // "what stack is this?" but "what does this code read?". It runs before the
    // mode selection because it replaces it entirely.
    // The folder init writes to: its template (DQ-28) and the config `main`
    // loaded for it (I-298).
    let target = shared::Target::new(Path::new(&path), config, verbose);

    if from_source {
        print_init_header();
        let opts = WriteOptions {
            yes: yes || dry_run || json,
            force,
            dry_run,
            json,
        };
        return components::handle_from_source(&target, opts, verbose);
    }

    if verbose {
        println!("{}", "Running init in verbose mode".dimmed());
    }

    print_init_header();

    // `--detect` accepts the proposal without asking, which is the same
    // no-prompt contract `--yes` carries everywhere else.
    let opts = WriteOptions {
        yes: yes || detect_flag || dry_run || json,
        force,
        dry_run,
        json,
    };

    // ⚠️ Detection runs before the menu, and only when nothing was asked for
    // explicitly. `--with`, `--blueprint` and `--yes` all say what the user
    // wants; proposing something else over the top of that would be worse than
    // not detecting at all.
    //
    // Nothing detected falls through to exactly the menu that was there before,
    // so a project evnx cannot read is no worse off than it was.
    let detected = if with.is_empty() && blueprint.is_none() && (!no_questions || detect_flag) {
        detect::detect(&target.root).unwrap_or_default()
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
    } else if no_questions {
        Mode::Blank
    } else {
        // ⚠️ Check before drawing the menu. dialoguer cannot run without a
        // terminal and says only "IO error: not a terminal" (twice, once from
        // each layer), which tells a CI job or an agent nothing about what to do
        // instead. The detected and from-source paths already refuse this way;
        // this was the one door that did not (I-014). Exit code stays 2. "A
        // terminal" means stdin and stderr, outside CI (I-299, DQ-30).
        if !shared::can_ask() {
            anyhow::bail!(
                "there is no terminal to show the menu on.\n\n\
                 Name what you want instead:\n  \
                 evnx init --yes                     blank files\n  \
                 evnx init --yes --blueprint <id>    a named stack\n  \
                 evnx init --yes --with <a,b,c>      components (evnx init --list-components)\n  \
                 evnx init --detect                  what the project's manifests declare"
            );
        }

        let modes = [
            "Blank      — create empty .env files",
            "Blueprint  — use a pre-configured stack",
            "Architect  — build a custom stack",
        ];

        let selection = Select::with_theme(&crate::utils::prompt::MENU)
            .with_prompt("How do you want to start?")
            .items(&modes)
            // Blank, not Blueprint (DQ-6). Enter-Enter used to write a 27-variable
            // T3 stack into whatever directory this was run in.
            .default(0)
            .interact_opt()?;

        // Esc or q: leave before anything is written, the same way every
        // other prompt in init now does.
        let Some(selection) = selection else {
            shared::aborted();
            print_docs_hint(&docs::INIT);
            return Ok(());
        };

        match selection {
            0 => Mode::Blank,
            1 => Mode::Blueprint,
            2 => Mode::Architect,
            _ => Mode::Blank,
        }
    };

    // Step 2: Route to handler
    let outcome = match mode {
        Mode::Blank => blank::handle(&target, opts, verbose)?,
        Mode::Components => components::handle(&target, opts, verbose, &with)?,
        Mode::Detected => components::handle_detected(&target, opts, verbose, &detected)?,
        Mode::Blueprint => self::blueprint::handle(&target, opts, verbose, blueprint.as_deref())?,
        Mode::Architect => architect::handle(&target, opts, verbose)?,
    };

    // ⚠️ "Next steps: Edit .env" only when something was written. It used to
    // print after "Aborted." and after the user kept their own template, both
    // times about a `.env` that did not exist (I-013, N-01). Exit code is 0 in
    // all three cases (DQ-3).
    if outcome == shared::Outcome::Written {
        print_next_steps_ui(target.template_name());
    }
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

fn print_next_steps_ui(template: &str) {
    // The contract init now keeps is committed with the template (IN-15).
    let commit = format!("Commit {template} and .evnx.toml; never commit .env");
    print_next_steps(&[
        "Edit .env and replace placeholder values",
        &commit,
        "Run 'evnx validate' to check configuration",
        "Use 'evnx add' to add more variables later",
    ]);
}
