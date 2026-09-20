use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
use std::path::Path;

use super::shared::{write_env_files, WriteOptions};
use crate::schema::{formatter, loader, resolver};
use crate::utils::ui::{print_header, print_preview_header, success};

/// Handle Blueprint mode: use `requested` when given, otherwise ask.
///
/// # Why an explicit id matters
///
/// The non-interactive path used to take `blueprints[0]`, and the blueprint list
/// came out of a `HashMap` — so `evnx init --yes` chose a *different* stack on
/// every run. The ordering is fixed now (see `schema::models::Ordered`), but a
/// positional default is still the wrong contract for a script: "whatever happens
/// to be first" silently changes meaning the day a blueprint is added. A script
/// that wants a stack should name it.
pub fn handle(
    path: String,
    opts: WriteOptions,
    verbose: bool,
    requested: Option<&str>,
) -> Result<()> {
    if !opts.yes {
        print_header("Blueprint Mode", Some("Select a pre-configured stack"));
    }
    let blueprints = loader::list_blueprints();

    if blueprints.is_empty() {
        return Err(anyhow::anyhow!("No blueprints available in schema"));
    }

    let selected_id = match requested {
        Some(id) => {
            if loader::get_blueprint(id).is_none() {
                return Err(anyhow::anyhow!(
                    "Unknown blueprint '{}'.\n\nAvailable blueprints:\n{}",
                    id,
                    blueprints
                        .iter()
                        .map(|(bid, name)| format!("  {bid:<22} {name}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            id
        }
        None if opts.yes => {
            return Err(anyhow::anyhow!(
                "Blueprint mode needs a blueprint when there is nobody to ask.\n\n\
                 Pass one explicitly:\n  evnx init --yes --blueprint <ID>\n\n\
                 Available blueprints:\n{}",
                blueprints
                    .iter()
                    .map(|(bid, name)| format!("  {bid:<22} {name}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        None => {
            let display_items: Vec<String> = blueprints
                .iter()
                .map(|(id, name)| {
                    let bp = loader::get_blueprint(id).unwrap();
                    format!("{}\n   {}", name.bold(), bp.description.dimmed())
                })
                .collect();

            let selection = Select::new()
                .with_prompt("Choose a stack blueprint:")
                .items(&display_items)
                .default(0)
                .interact()?;
            blueprints[selection].0
        }
    };

    let blueprint = loader::get_blueprint(selected_id)
        .context(format!("Blueprint '{}' not found", selected_id))?;

    if verbose {
        println!(
            "{} Resolving blueprint: {} ({})",
            "[DEBUG]".dimmed(),
            blueprint.name,
            selected_id
        );
    }

    // Resolve variables
    let vars =
        resolver::resolve_blueprint(blueprint).context("Failed to resolve blueprint variables")?;

    // Show preview
    // println!("\n{}", "📋 Preview:".bold());
    // println!("{}", formatter::generate_preview(&vars).dimmed());
    // Preview section
    if !opts.yes {
        print_preview_header(); // 📋 Preview:
        println!("{}", formatter::generate_preview(&vars).dimmed());
    }

    if !opts.yes {
        let confirm = dialoguer::Confirm::new()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact()?;

        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Format and write files
    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;

    let output_path = Path::new(&path);
    write_env_files(output_path, &example_content, &template_content, opts)?;

    success(format!(
        "Created .env.example with {} variables",
        vars.vars.len()
    ));

    Ok(())
}
