//! Handle `evnx add blueprint <blueprint_id>`
//!
//! Adds variables from a pre-combined stack WITHOUT overwriting existing vars.

use anyhow::{Context, Result};
use colored::*;
use dialoguer::Confirm;
use std::path::Path;

use super::shared::{append_to_env_files, detect_conflicts, AppendMode};
use crate::schema::{formatter, loader, resolver};
use crate::utils::ui::{info, warning};

/// Handle blueprint addition
pub fn handle(blueprint_id: &str, output_path: &Path, yes: bool, verbose: bool) -> Result<()> {
    // 1. Find blueprint
    let blueprint = loader::get_blueprint(blueprint_id).context(format!(
        "Unknown blueprint: '{}'. Run 'evnx init' to see available blueprints.",
        blueprint_id
    ))?;

    if verbose {
        println!(
            "{} Resolving blueprint: {} ({})",
            "[DEBUG]".dimmed(),
            blueprint.name,
            blueprint_id
        );
    }

    // 2. Resolve to variables
    let vars =
        resolver::resolve_blueprint(blueprint).context("Failed to resolve blueprint variables")?;

    // 3. Check for conflicts with existing .env.example
    let example_path = output_path.join(".env.example");
    let conflicts = if example_path.exists() {
        let existing_content = std::fs::read_to_string(&example_path)?;
        detect_conflicts(&existing_content, &vars)
    } else {
        Vec::new()
    };

    // 4. Show preview with conflict warnings
    println!("\n{}", "📋 Preview:".bold());
    println!("{}", formatter::generate_preview(&vars).dimmed());

    if !conflicts.is_empty() {
        //println!("\n{}", "⚠️  Conflicts detected:".yellow().bold());
        warning(&format!("{} conflicts detected", conflicts.len()));
        for conflict in &conflicts {
            println!(
                "  • {} (existing: \"{}\", new: \"{}\")",
                conflict.var_name.bold(),
                conflict.existing_value.dimmed(),
                conflict.new_value.dimmed()
            );
        }
        // println!("\n{}", "Conflicting variables will be SKIPPED (not overwritten).".yellow());
        info("Conflicting variables will be SKIPPED (not overwritten)");
    }

    // 5. Confirm (unless --yes)
    if !yes {
        let prompt = if conflicts.is_empty() {
            format!("Add these {} variables to .env.example?", vars.vars.len())
        } else {
            format!(
                "Add {} new variables (skipping {} conflicts)?",
                vars.vars.len() - conflicts.len(),
                conflicts.len()
            )
        };

        let confirm = Confirm::new()
            .with_prompt(&prompt)
            .default(true)
            .interact()?;

        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // 6. Format addition (excluding conflicts)
    let filtered_vars = if conflicts.is_empty() {
        vars.clone()
    } else {
        let mut filtered = vars.clone();
        for conflict in &conflicts {
            filtered.vars.remove(&conflict.var_name);
        }
        filtered
    };

    let addition = formatter::format_addition(&filtered_vars)?;
    let header = format!("\n# ── Blueprint: {} ──\n", blueprint.name);
    let content = format!("{}{}", header, addition);

    // 7. Append to files (skip conflicts mode)
    append_to_env_files(output_path, &content, AppendMode::SkipConflicts, verbose)?;

    let added_count = filtered_vars.vars.len();
    println!(
        "{} Added {} variables from blueprint '{}'",
        "✓".green(),
        added_count,
        blueprint.name
    );

    if !conflicts.is_empty() {
        println!(
            "{} Skipped {} conflicting variables (preserved existing values)",
            "ℹ".blue(),
            conflicts.len()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_handle_skips_conflicting_vars() {
        let dir = TempDir::new().unwrap();
        let example_path = dir.path().join(".env.example");

        // Create file with conflicting var
        fs::write(&example_path, "DATABASE_URL=existing_value\n").unwrap();

        // Handle blueprint that includes postgresql (which defines DATABASE_URL)
        handle("t3_modern", dir.path(), true, false).unwrap();

        // Verify conflict was skipped
        let content = fs::read_to_string(&example_path).unwrap();
        assert!(content.contains("DATABASE_URL=existing_value")); // Original preserved

        // But other vars from blueprint were added
        assert!(content.contains("NEXTAUTH_SECRET=")); // Next.js var added
    }
}
