use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
use std::path::Path;

use super::shared::write_env_files;
use crate::schema::{formatter, loader, resolver};
use crate::utils::ui::{print_header, print_preview_header, success};

/// Handle Blueprint mode: select pre-combined stack
pub fn handle(path: String, yes: bool, verbose: bool) -> Result<()> {
    if !yes {
        print_header("Blueprint Mode", Some("Select a pre-configured stack"));
    }
    let blueprints = loader::list_blueprints();

    if blueprints.is_empty() {
        return Err(anyhow::anyhow!("No blueprints available in schema"));
    }

    // Display blueprints with descriptions
    let display_items: Vec<String> = blueprints
        .iter()
        .map(|(id, name)| {
            let bp = loader::get_blueprint(id).unwrap();
            format!("{}\n   {}", name.bold(), bp.description.dimmed())
        })
        .collect();

    let selection = if yes {
        0 // Default to first blueprint in non-interactive mode
    } else {
        Select::new()
            .with_prompt("Choose a stack blueprint:")
            .items(&display_items)
            .default(0)
            .interact()?
    };

    let (selected_id, _) = blueprints[selection];
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
    if !yes {
        print_preview_header(); // 📋 Preview:
        println!("{}", formatter::generate_preview(&vars).dimmed());
    }

    if !yes {
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
    write_env_files(output_path, &example_content, &template_content)?;

    success(&format!(
        "Created .env.example with {} variables",
        vars.vars.len()
    ));

    Ok(())
}
