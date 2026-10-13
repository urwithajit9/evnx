//! Handle `evnx add framework --language <lang> <framework>`

use anyhow::{Context, Result};
use colored::*;
use std::path::Path;

use super::shared::{aborted, append_to_env_files, count, AppendMode, Outcome};
use crate::schema::{component, formatter, loader};
use crate::utils::prompt::{require_terminal, yes_no};
use crate::utils::ui::success;

/// Handle framework addition
pub fn handle(
    language_id: &str,
    framework_id: &str,
    output_path: &Path,
    yes: bool,
    verbose: bool,
) -> Result<Outcome> {
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

    // 2. Validate framework exists for this language. An alias (`express`,
    // `next`) names the same framework as its id, as it does for `init --with`.
    let framework_id = crate::schema::component::canonical(framework_id);
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

    // ⚠️ Through the shared catalogue, like `add service` and `init --with`.
    // Framework ids are globally unique, so `--language` is now redundant for
    // resolution — it is still validated above, which keeps the existing error
    // for a framework that belongs to a different language.
    let vars = component::resolve(&[framework_id.to_string()])?;

    // 4–5. Preview and confirm, unless --yes. `--yes` never previews (DQ-A6):
    // it used to here and in `blueprint` but not in `service`, and `-q` did
    // not hide it (I-019, N-15).
    if !yes {
        require_terminal(&format!(
            "evnx add framework --language {language_id} {framework_id} --yes"
        ))?;
        println!("\n{}", "Preview:".bold());
        println!("{}", formatter::generate_preview(&vars).dimmed());

        let confirm = yes_no()
            .with_prompt(format!(
                "Add these {} for {} to .env.example?",
                count(vars.vars.len(), "variable"),
                framework.display_name.as_deref().unwrap_or(framework_id)
            ))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
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
        &vars,
    )?;

    success(format!(
        "Added {} for {}",
        count(vars.vars.len(), "variable"),
        framework.display_name.as_deref().unwrap_or(framework_id)
    ));

    Ok(Outcome::Written)
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
