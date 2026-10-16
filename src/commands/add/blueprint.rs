//! Handle `evnx add blueprint <blueprint_id>`
//!
//! Adds variables from a pre-combined stack WITHOUT overwriting existing vars.

use anyhow::{Context, Result};
use colored::*;
use std::path::Path;

use super::shared::{confirm_and_apply, count, Flow, Mode, Outcome};
use crate::schema::{formatter, loader, resolver};

/// Handle blueprint addition
pub fn handle(blueprint_id: Option<&str>, output_path: &Path, mode: &Mode) -> Result<Outcome> {
    // No id: pick one.
    let picked;
    let blueprint_id = match blueprint_id {
        Some(id) => id,
        None => {
            let items: Vec<(String, String)> = loader::list_blueprints()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect();
            match super::pick(mode, "blueprint", &items)? {
                Some(id) => {
                    picked = id;
                    picked.as_str()
                }
                None => return Ok(super::shared::aborted()),
            }
        }
    };

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

    if mode.verbose {
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

    // 3. Skip what is there, list what differs, ask, write: the same path as
    // every other subcommand (AD-S). This command used to be the only one that
    // skipped anything, and only names whose value differed.
    let name = blueprint.name.clone();
    let heading = || println!("\n{}", "Preview:".bold());
    let render = |v: &crate::schema::models::VarCollection| {
        Ok(format!(
            "\n# ── Blueprint: {name} ──\n{}",
            formatter::format_addition(v)?
        ))
    };
    let flow = Flow {
        noun: "variable",
        what: format!("from blueprint '{name}'"),
        terminal_example: Some(format!("evnx add blueprint {blueprint_id} --yes")),
        heading: &heading,
        preview: &formatter::generate_preview,
        question: &|n, file| format!("Add these {} to {file}?", count(n, "variable")),
        render: &render,
        example_render: None,
        env_from_values: false,
    };
    confirm_and_apply(output_path, &vars, &flow, mode)
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
        handle(
            Some("t3_modern"),
            dir.path(),
            &crate::commands::add::shared::Mode {
                yes: true,
                ..Default::default()
            },
        )
        .unwrap();

        // Verify conflict was skipped
        let content = fs::read_to_string(&example_path).unwrap();
        assert!(content.contains("DATABASE_URL=existing_value")); // Original preserved

        // But other vars from blueprint were added
        assert!(content.contains("NEXTAUTH_SECRET=")); // Next.js var added
    }
}
