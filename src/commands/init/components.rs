//! `evnx init --with <a,b,c>` — name the pieces directly.
//!
//! The same resolve → preview → confirm → write shape as [`super::blueprint`],
//! over an explicit component list rather than a named stack. A blueprint is
//! structurally one of these lists, so this is the general case and blueprints
//! are the shorthand.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

use super::shared::{write_env_files, WriteOptions};
use crate::schema::{component, formatter};
use crate::utils::ui::{self, glyph};

/// Generate from an explicit component list.
pub fn handle(
    path: String,
    opts: WriteOptions,
    verbose: bool,
    components: &[String],
) -> Result<()> {
    // ⚠️ Resolve before anything is printed or written. An unknown name fails
    // here, with suggestions, rather than after a preview has implied it worked.
    let vars = component::resolve(components).context("Failed to resolve components")?;

    if verbose {
        println!(
            "{} {} components → {} variables",
            "[DEBUG]".dimmed(),
            components.len(),
            vars.vars.len()
        );
    }

    if !opts.yes {
        println!();
        for id in components {
            if let Some(c) = component::find(id)? {
                println!(
                    "  {}  {:<22} {}",
                    glyph::OK.green(),
                    c.display_name,
                    c.kind.label().dimmed()
                );
            }
        }
        println!("\n{}", formatter::generate_preview(&vars).dimmed());

        let confirm = dialoguer::Confirm::new()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact()?;
        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;
    write_env_files(Path::new(&path), &example_content, &template_content, opts)?;

    ui::success(format!(
        "Created .env.example with {} variables from {} components",
        vars.vars.len(),
        components.len()
    ));
    Ok(())
}

/// Print every component `--with` accepts, grouped by kind.
///
/// This is the list the error message points at, so it has to be reachable
/// without a project, without a prompt, and without side effects.
pub fn list() -> Result<()> {
    let all = component::catalogue()?;

    for kind in [
        component::Kind::Framework,
        component::Kind::Service,
        component::Kind::Infrastructure,
    ] {
        let of_kind: Vec<_> = all.iter().filter(|c| c.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }

        println!("\n  {}", kind.label().to_uppercase().bold());
        let mut group = "";
        for c in of_kind {
            if c.group != group {
                group = &c.group;
                println!("  {}", group.dimmed());
            }
            println!("    {:<24} {}", c.id, c.display_name.dimmed());
        }
    }

    println!(
        "\n  {}  evnx init --with {}\n",
        glyph::ARROW.dimmed(),
        "nextjs,postgresql,stripe".dimmed()
    );
    Ok(())
}
