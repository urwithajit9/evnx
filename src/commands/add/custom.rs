//! Handle `evnx add custom` — Interactive custom variable addition

use anyhow::Result;
use colored::*;
use dialoguer::{Input, Select};
use std::path::Path;

use super::shared::{aborted, append_to_env_files, count, Outcome};
use crate::utils::prompt::{require_terminal_or, yes_no};
use crate::utils::ui::{info, print_header, print_preview_header, success};

/// Handle interactive custom variable addition
pub fn handle(output_path: &Path, yes: bool, verbose: bool) -> Result<Outcome> {
    // Every variable is typed at a prompt, so `--yes` cannot stand in for a
    // terminal here; say so instead of dialoguer's raw error (I-020). The
    // argument form that works without one is AD-F (I-025).
    require_terminal_or("`evnx add custom` asks for each variable, so it needs a terminal.")?;

    print_header(
        "Custom Variables",
        Some("Add your own environment variables interactively"),
    );

    info("Enter variables one at a time. Empty name to finish.");

    let mut additions = Vec::new();
    let mut collected = crate::schema::models::VarCollection::default();

    loop {
        // Prompt for variable name
        let name: String = Input::new()
            .with_prompt("Variable name (or Enter to finish)")
            .allow_empty(true)
            .interact_text()?;

        if name.trim().is_empty() {
            break;
        }

        // ⚠️ Check before writing. `add` had no validation at all, so typing
        // `NODE_VERSION=22` at this prompt wrote `# TODO: NODE_VERSION=22=22`
        // — a line evnx's own parser refuses to read (issue #10, item 4).
        // Re-prompt rather than abort: the user is mid-loop and has already
        // entered other variables.
        let name = name.trim().to_string();
        if !crate::core::parser::is_valid_key(&name) {
            if name.contains('=') {
                // By far the most likely slip, and the one in the issue.
                let (key, _) = name.split_once('=').expect("contains checked");
                crate::utils::ui::warning(format!(
                    "Enter the name on its own — the value is the next prompt.\n                       Did you mean {}?",
                    key.trim()
                ));
            } else {
                crate::utils::ui::warning(format!(
                    "`{name}` is not a valid variable name.\n                       Names start with a letter or underscore and contain only \
                     letters, digits and underscores."
                ));
            }
            continue;
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
            let Some(add_desc) = yes_no()
                .with_prompt("Add a description comment?")
                .default(false)
                .interact_opt()?
            else {
                return Ok(aborted());
            };

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
                "Authentication",
                "Service",
                "Other",
            ];
            let Some(use_cat) = yes_no()
                .with_prompt("Assign to a category?")
                .default(false)
                .interact_opt()?
            else {
                return Ok(aborted());
            };

            if use_cat {
                // `.default(0)`: with no default the list opened with nothing
                // selected, Enter alone did nothing, and the first Down landed
                // on the first item (N-18).
                let Some(idx) = Select::new()
                    .with_prompt("Select category")
                    .items(&categories)
                    .default(0)
                    .interact_opt()?
                else {
                    return Ok(aborted());
                };
                Some(categories[idx].to_string())
            } else {
                None
            }
        };

        // Prompt for required flag
        let required = if yes {
            false
        } else {
            match yes_no()
                .with_prompt("Is this variable required?")
                .default(false)
                .interact_opt()?
            {
                Some(answer) => answer,
                None => return Ok(aborted()),
            }
        };

        additions.push((
            entry(
                name.trim(),
                &example,
                description.as_deref(),
                category.as_deref(),
                required,
            ),
            name.trim().to_string(),
            category.clone(),
        ));

        // ⚠️ Built alongside the text rather than parsed back out of it. `custom`
        // is the one `add` subcommand with no `VarCollection` of its own — it
        // renders straight to lines — and the spec write-back needs the same
        // structure every other subcommand already has.
        collected.vars.insert(
            name.trim().to_string(),
            crate::schema::models::VarMetadata {
                example_value: example.clone(),
                description: description.clone(),
                category,
                required,
                source: crate::schema::models::VarSource::BlueprintOverride,
            },
        );

        if !yes {
            let Some(continue_adding) = yes_no()
                .with_prompt("Add another variable?")
                .default(true)
                .interact_opt()?
            else {
                return Ok(aborted());
            };

            if !continue_adding {
                break;
            }
        }
    }

    if additions.is_empty() {
        println!("{}", "No variables added.".yellow());
        return Ok(Outcome::Aborted);
    }

    // Format all additions
    let mut content = String::from("\n# ── Custom Variables ──\n");
    for (line, _name, category) in &additions {
        // A categorised entry keeps the blank line above it that it always had.
        if category.is_some() {
            content.push('\n');
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
        println!("  • {} to add", count(additions.len(), "variable"));
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
        let confirm = yes_no()
            .with_prompt("Append these variables to .env.example?")
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
        }
    }

    // Append to files
    append_to_env_files(
        output_path,
        &content,
        super::shared::AppendMode::WithConflictWarning,
        verbose,
        &collected,
    )?;

    // println!(
    //     "{} Added {} custom variables",
    //     "✓".green(),
    //     additions.len()
    // );
    success(format!(
        "Added {}",
        count(additions.len(), "custom variable")
    ));

    Ok(Outcome::Written)
}

/// One variable as it is written: an optional comment line, then `NAME=value`.
///
/// Same shape as `shared::format_var_line`: the marker belongs above the
/// variable, not on a detached line after it. The category goes *inside* the
/// comment line (`# [Security] API key (required)`, or `# [Security]` alone).
///
/// ⚠️ It used to be prefixed to the entry's first line. With no description
/// and not required, that first line is the assignment, so a category turned
/// `CAT=v1` into `# [Application] CAT=v1`: commented out, invisible to every
/// parser, while the run said "Added" (N-17).
fn entry(
    name: &str,
    example: &str,
    description: Option<&str>,
    category: Option<&str>,
    required: bool,
) -> String {
    let mut words = Vec::new();
    if let Some(cat) = category {
        words.push(format!("[{cat}]"));
    }
    if let Some(desc) = description {
        words.push(desc.to_string());
    }
    if required {
        words.push("(required)".to_string());
    }
    let assignment = format!("{name}={example}");
    if words.is_empty() {
        assignment
    } else {
        format!("# {}\n{assignment}", words.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::entry;

    #[test]
    fn plain_entry_is_one_assignment() {
        assert_eq!(entry("A", "1", None, None, false), "A=1");
    }

    #[test]
    fn description_and_required_go_above() {
        assert_eq!(
            entry("A", "1", Some("Key"), None, true),
            "# Key (required)\nA=1"
        );
        assert_eq!(entry("A", "1", None, None, true), "# (required)\nA=1");
    }

    /// N-17: every combination with a category leaves the assignment active.
    #[test]
    fn a_category_never_comments_out_the_assignment() {
        for (desc, required, comment) in [
            (None, false, "# [Security]"),
            (None, true, "# [Security] (required)"),
            (Some("Key"), false, "# [Security] Key"),
            (Some("Key"), true, "# [Security] Key (required)"),
        ] {
            let text = entry("A", "1", desc, Some("Security"), required);
            assert_eq!(text, format!("{comment}\nA=1"));
            assert!(text.lines().any(|l| l == "A=1"), "{text}");
        }
    }
}
