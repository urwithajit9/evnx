use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::fs;
use std::path::Path;

use crate::config::registry;
use crate::generators::template::EnvTemplateBuilder;
use crate::utils::file_ops;

/// Interactive project setup — generates .env.example
#[allow(clippy::too_many_arguments)]
pub fn run(
    stack: Option<String>,
    services: Option<String>,
    path: String,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running init in verbose mode".dimmed());
    }

    print_header();

    let reg = registry();

    // Determine stack (interactive or from flag)
    let selected_stack_id = resolve_stack(stack, yes, reg)?;

    // Determine services (interactive or from flag)
    let selected_service_ids = resolve_services(services, yes, reg)?;

    // Determine output path
    let output_path = resolve_output_path(path, yes)?;

    // Ensure output directory exists
    file_ops::ensure_dir(Path::new(&output_path))
        .with_context(|| format!("Failed to create directory: {}", output_path))?;

    // Build environment template
    let stack_gen = reg
        .get_stack(&selected_stack_id)
        .context(format!("Unknown stack: {}", selected_stack_id))?;

    let service_gens: Vec<_> = selected_service_ids
        .iter()
        .filter_map(|id| reg.get_service(id))
        .collect();

    let env_example_content = EnvTemplateBuilder::new()
        .with_stack(stack_gen)
        .with_services(service_gens)
        .build()?;

    let env_example_path = Path::new(&output_path).join(".env.example");

    // Check if file already exists
    if env_example_path.exists() && !yes {
        let overwrite = Confirm::new()
            .with_prompt(format!(
                "{} already exists. Overwrite?",
                env_example_path.display()
            ))
            .default(false)
            .interact()?;

        if !overwrite {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Write .env.example
    fs::write(&env_example_path, env_example_content.trim())
        .context("Failed to write .env.example")?;

    let var_count = env_example_content
        .lines()
        .filter(|line| line.contains('='))
        .count();

    println!(
        "{} Created .env.example with {} variables",
        "✓".green(),
        var_count
    );

    // Create .env from template
    create_env_file(&output_path, &env_example_content)?;

    // Update .gitignore
    update_gitignore(&output_path)?;

    // Print next steps
    print_next_steps();

    Ok(())
}

fn print_header() {
    println!(
        "\n{}",
        "┌─ dotenv-space init ─────────────────────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Let's set up environment variables for your project │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );
}

fn resolve_stack(
    flag: Option<String>,
    yes: bool,
    reg: &crate::config::registry::GeneratorRegistry,
) -> Result<String> {
    if let Some(s) = flag {
        return Ok(s);
    }

    if yes {
        return Ok("python".to_string());
    }

    let stacks: Vec<_> = reg.list_stacks();
    let display_names: Vec<_> = stacks
        .iter()
        .filter_map(|&id| reg.get_stack(id).map(|g| g.display_name()))
        .collect();

    let selection = Select::new()
        .with_prompt("What's your primary stack?")
        .items(&display_names)
        .default(0)
        .interact()?;

    Ok(stacks[selection].to_string())
}

fn resolve_services(
    flag: Option<String>,
    yes: bool,
    reg: &crate::config::registry::GeneratorRegistry,
) -> Result<Vec<String>> {
    if let Some(s) = flag {
        return Ok(s.split(',').map(|s| s.trim().to_string()).collect());
    }

    if yes {
        return Ok(vec!["postgresql".to_string(), "redis".to_string()]);
    }

    let services: Vec<_> = reg.list_services();
    let display_names: Vec<_> = services
        .iter()
        .filter_map(|&id| reg.get_service(id).map(|g| g.display_name()))
        .collect();

    let selections = MultiSelect::new()
        .with_prompt("Which services will you use? (Space to select, Enter to confirm)")
        .items(&display_names)
        .interact()?;

    Ok(selections
        .iter()
        .map(|&idx| services[idx].to_string())
        .collect())
}

fn resolve_output_path(default_path: String, yes: bool) -> Result<String> {
    if yes {
        return Ok(default_path);
    }

    Input::new()
        .with_prompt("Where should I create .env.example?")
        .default(default_path)
        .interact_text()
        .context("Failed to read output path")
}

fn create_env_file(output_path: &str, template_content: &str) -> Result<()> {
    let env_path = Path::new(output_path).join(".env");

    if !env_path.exists() {
        let mut env_content =
            "# TODO: Replace all placeholder values with real credentials\n\n".to_string();
        env_content.push_str(template_content);

        fs::write(&env_path, env_content).context("Failed to write .env")?;

        println!(
            "{} Created .env from template (fill in real values)",
            "✓".green()
        );
    }
    Ok(())
}

fn update_gitignore(output_path: &str) -> Result<()> {
    let gitignore_path = Path::new(output_path).join(".gitignore");
    let gitignore_entry = "\n# Environment files\n.env\n.env.local\n.env.*.local\n";

    if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path).context("Failed to read .gitignore")?;

        if !content.contains(".env") {
            let mut updated = content;
            updated.push_str(gitignore_entry);
            fs::write(&gitignore_path, updated).context("Failed to update .gitignore")?;
            println!("{} Added .env to .gitignore", "✓".green());
        }
    } else {
        fs::write(&gitignore_path, gitignore_entry.trim_start())
            .context("Failed to create .gitignore")?;
        println!("{} Created .gitignore", "✓".green());
    }
    Ok(())
}

fn print_next_steps() {
    println!("\n{}", "Next steps:".bold());
    println!("  1. Edit .env and replace placeholder values");
    println!("  2. Never commit .env to Git");
    println!("  3. Run 'dotenv-space validate' to check for issues");
}
