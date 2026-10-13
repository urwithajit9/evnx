//! Handle `evnx add blueprint <blueprint_id>`
//!
//! Adds variables from a pre-combined stack WITHOUT overwriting existing vars.

use anyhow::{Context, Result};
use colored::*;
use std::path::Path;

use super::shared::{aborted, append_to_env_files, count, detect_conflicts, AppendMode, Outcome};
use crate::schema::{formatter, loader, resolver};
use crate::utils::prompt::{require_terminal, yes_no};
use crate::utils::ui::{info, success, warning};

/// Handle blueprint addition
pub fn handle(blueprint_id: &str, output_path: &Path, yes: bool, verbose: bool) -> Result<Outcome> {
    // 1. Find blueprint
    //
    // ⚠️ The list is printed here rather than pointing at another command.
    // This used to say "Run 'evnx init' to see available blueprints" — which
    // sends you to an *interactive* prompt, so the advice is unusable in the
    // scripts and CI steps where `--yes` is given. `evnx init --blueprint`
    // already answers an unknown id by listing them; the same error deserves
    // the same quality of answer from whichever command produced it.
    let blueprint = match loader::get_blueprint(blueprint_id) {
        Some(found) => found,
        None => {
            let available = loader::list_blueprints();
            return Err(anyhow::anyhow!(
                "Unknown blueprint '{}'.\n\nAvailable blueprints:\n{}",
                blueprint_id,
                available
                    .iter()
                    .map(|(bid, name)| format!("  {bid:<22} {name}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    };

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

    // 4. Preview, unless --yes (DQ-A6: `--yes` never previews; I-019, N-15).
    if !yes {
        require_terminal(&format!("evnx add blueprint {blueprint_id} --yes"))?;
        println!("\n{}", "Preview:".bold());
        println!("{}", formatter::generate_preview(&vars).dimmed());
    }

    // Conflicts are a finding, not chatter: they print under `--yes` and `-q`.
    if !conflicts.is_empty() {
        warning(format!("{} detected", count(conflicts.len(), "conflict")));
        for conflict in &conflicts {
            println!(
                "  • {} (existing: \"{}\", new: \"{}\")",
                conflict.var_name.bold(),
                conflict.existing_value.dimmed(),
                conflict.new_value.dimmed()
            );
        }
        info("Conflicting variables will be SKIPPED (not overwritten)");
    }

    // 5. Confirm (unless --yes)
    if !yes {
        let prompt = if conflicts.is_empty() {
            format!(
                "Add these {} to .env.example?",
                count(vars.vars.len(), "variable")
            )
        } else {
            format!(
                "Add {} (skipping {})?",
                count(vars.vars.len() - conflicts.len(), "new variable"),
                count(conflicts.len(), "conflict")
            )
        };

        let confirm = yes_no().with_prompt(&prompt).default(true).interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
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
    append_to_env_files(
        output_path,
        &content,
        AppendMode::SkipConflicts,
        verbose,
        &filtered_vars,
    )?;

    success(format!(
        "Added {} from blueprint '{}'",
        count(filtered_vars.vars.len(), "variable"),
        blueprint.name
    ));

    if !conflicts.is_empty() {
        info(format!(
            "Skipped {} (preserved existing values)",
            count(conflicts.len(), "conflicting variable")
        ));
    }

    Ok(Outcome::Written)
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
