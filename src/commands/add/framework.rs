//! Handle `evnx add framework --language <lang> <framework>`

use anyhow::{Context, Result};
use colored::*;
use dialoguer::Confirm;
use std::path::Path;

use super::shared::{append_to_env_files, AppendMode};
use crate::schema::{formatter, loader, resolver};

/// Handle framework addition
pub fn handle(
    language_id: &str,
    framework_id: &str,
    output_path: &Path,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    // 1. Validate language exists
    let schema = loader::schema()?;
    let language = schema.languages.get(language_id).context(format!(
        "Unknown language: '{}'. Available: {}",
        language_id,
        schema
            .languages
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ))?;

    // 2. Validate framework exists for this language
    let framework = language.frameworks.get(framework_id).context(format!(
        "Unknown framework '{}' for language '{}'. Available: {}",
        framework_id,
        language_id,
        language
            .frameworks
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ))?;

    if verbose {
        println!(
            "{} Resolving framework: {}/{}",
            "[DEBUG]".dimmed(),
            language_id,
            framework_id
        );
    }

    // 3. Resolve to variables
    let vars = resolver::resolve_framework(language_id, framework_id, framework)
        .context("Failed to resolve framework variables")?;

    // 4. Show preview
    println!("\n{}", "📋 Preview:".bold());
    println!("{}", formatter::generate_preview(&vars).dimmed());

    // 5. Confirm (unless --yes)
    if !yes {
        let confirm = Confirm::new()
            .with_prompt(format!(
                "Add these {} variables for {} to .env.example?",
                vars.vars.len(),
                framework.display_name.as_deref().unwrap_or(framework_id)
            ))
            .default(true)
            .interact()?;

        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // 6. Format as addition with framework header
    let addition = formatter::format_addition(&vars)?;
    let header = format!(
        "\n# ── Framework: {} ──\n",
        framework.display_name.as_deref().unwrap_or(framework_id)
    );
    let content = format!("{}{}", header, addition);

    // 7. Append to files
    append_to_env_files(
        output_path,
        &content,
        AppendMode::WithConflictWarning,
        verbose,
    )?;

    println!(
        "{} Added {} variables for {}",
        "✓".green(),
        vars.vars.len(),
        framework.display_name.as_deref().unwrap_or(framework_id)
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_handle_appends_framework_vars() {
        let dir = TempDir::new().unwrap();
        let example_path = dir.path().join(".env.example");

        // Create initial file
        fs::write(&example_path, "# Existing\nAPP_NAME=test\n").unwrap();

        // Handle framework addition
        handle("python", "django", dir.path(), true, false).unwrap();

        // Verify append (not overwrite)
        let content = fs::read_to_string(&example_path).unwrap();
        assert!(content.contains("APP_NAME=test")); // Original preserved
        assert!(content.contains("SECRET_KEY=")); // Django var added
        assert!(content.contains("Framework: Django")); // Section header
    }
}
