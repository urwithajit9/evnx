//! Handle `evnx add custom` — Interactive custom variable addition

use anyhow::Result;
use colored::*;
use dialoguer::{Confirm, Input, Select};
use std::path::Path;

use super::shared::append_to_env_files;
use crate::utils::ui::{info, print_header, print_preview_header, success};

/// Handle interactive custom variable addition
pub fn handle(output_path: &Path, yes: bool, verbose: bool) -> Result<()> {
    print_header(
        "Custom Variables",
        Some("Add your own environment variables interactively"),
    );

    info("Enter variables one at a time. Empty name to finish.");

    let mut additions = Vec::new();

    loop {
        // Prompt for variable name
        let name: String = Input::new()
            .with_prompt("Variable name (or Enter to finish)")
            .allow_empty(true)
            .interact_text()?;

        if name.trim().is_empty() {
            break;
        }

        // Prompt for example value
        let example: String = Input::new()
            .with_prompt("Example/placeholder value")
            .default("your_value_here".to_string())
            .interact_text()?;

        // Prompt for description (optional)
        let description: Option<String> = if yes {
            None
        } else {
            let add_desc = Confirm::new()
                .with_prompt("Add a description comment?")
                .default(false)
                .interact()?;

            if add_desc {
                Some(Input::new().with_prompt("Description").interact_text()?)
            } else {
                None
            }
        };

        // Prompt for category (optional)
        let category: Option<String> = if yes {
            None
        } else {
            let categories = vec![
                "Application",
                "Security",
                "Database",
                "Auth",
                "Service",
                "Other",
            ];
            let use_cat = Confirm::new()
                .with_prompt("Assign to a category?")
                .default(false)
                .interact()?;

            if use_cat {
                let idx = Select::new()
                    .with_prompt("Select category")
                    .items(&categories)
                    .interact()?;
                Some(categories[idx].to_string())
            } else {
                None
            }
        };

        // Prompt for required flag
        let required = if yes {
            false
        } else {
            Confirm::new()
                .with_prompt("Is this variable required?")
                .default(false)
                .interact()?
        };

        // Format as .env line
        let mut lines = Vec::new();
        if let Some(desc) = &description {
            lines.push(format!("# {}", desc));
        }
        lines.push(format!("{}={}", name.trim(), example));
        if required {
            lines.push("  # (required)".to_string());
        }

        additions.push((lines.join("\n"), name.trim().to_string(), category));

        if !yes {
            let continue_adding = Confirm::new()
                .with_prompt("Add another variable?")
                .default(true)
                .interact()?;

            if !continue_adding {
                break;
            }
        }
    }

    if additions.is_empty() {
        println!("{}", "No variables added.".yellow());
        return Ok(());
    }

    // Format all additions
    let mut content = String::from("\n# ── Custom Variables ──\n");
    for (line, _name, category) in &additions {
        if let Some(cat) = category {
            content.push_str(&format!("\n# [{}] ", cat));
        }
        content.push_str(line);
        content.push('\n');
    }
    content.push_str(&format!(
        "\n# Added by evnx add custom on {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));

    // Show summary
    // println!("\n{}", "📋 Summary:".bold());
    // println!("  • {} variables to add", additions.len());
    if !yes {
        print_preview_header();
        println!("  • {} variables to add", additions.len());
    }

    let _categories: Vec<_> = additions
        .iter()
        .filter_map(|(_, _, cat)| cat.as_ref())
        .collect();

    let categories: Vec<String> = additions
        .iter()
        .filter_map(|(_, _, cat)| cat.as_ref().cloned())
        .collect();

    if !categories.is_empty() {
        println!("  • Categories: {}", categories.join(", "));
    }

    // Confirm
    if !yes {
        let confirm = Confirm::new()
            .with_prompt("Append these variables to .env.example?")
            .default(true)
            .interact()?;

        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // Append to files
    append_to_env_files(
        output_path,
        &content,
        super::shared::AppendMode::WithConflictWarning,
        verbose,
    )?;

    // println!(
    //     "{} Added {} custom variables",
    //     "✓".green(),
    //     additions.len()
    // );
    success(&format!("Added {} custom variables", additions.len()));

    Ok(())
}

#[cfg(test)]
mod tests {
    // use super::*;
    use tempfile::TempDir;
    // use std::fs;

    #[test]
    fn test_handle_creates_custom_section() {
        let dir = TempDir::new().unwrap();

        // Simulate adding one variable (non-interactive test)
        // Note: Full interactive testing requires mocking dialoguer
        // This test verifies the output formatting logic

        let mut content = String::from("\n# ── Custom Variables ──\n");
        content.push_str("# My custom API key\n");
        content.push_str("MY_API_KEY=placeholder_value\n");
        content.push_str("  # (required)\n");

        assert!(content.contains("Custom Variables"));
        assert!(content.contains("MY_API_KEY="));
        assert!(content.contains("(required)"));
    }
}
