use anyhow::Result;
use colored::*;
use dialoguer::Confirm;
use std::path::Path;

use crate::schema::{component, formatter};
use crate::utils::ui::{print_header, print_preview_header, success};

/// Handle `evnx add service <service_id>`
pub fn handle(service_id: &str, path: &str, yes: bool, _verbose: bool) -> Result<()> {
    if !yes {
        print_header(
            "Add Service",
            Some(&format!("Adding variables for '{}'", service_id)),
        );
    }
    // ⚠️ Through the shared catalogue, so `evnx add service postgresql` and
    // `evnx init --with postgresql` resolve the same component by the same name.
    // The unknown-name error comes with suggestions and points at
    // `--list-components`, instead of at this subcommand's --help.
    let vars = component::resolve(&[service_id.to_string()])?;
    let display_name = component::find(service_id)?
        .map(|c| c.display_name)
        .unwrap_or_else(|| service_id.to_string());

    // 3. Show preview
    // println!("\n{}", "📋 Preview:".bold());
    // println!("{}", formatter::generate_preview(&vars).dimmed());
    if !yes {
        print_preview_header();
        println!("{}", formatter::generate_preview(&vars).dimmed());
    }

    // 4. Confirm (unless --yes)
    if !yes {
        let confirm = Confirm::new()
            .with_prompt(format!(
                "Add these {} variables to .env.example?",
                vars.vars.len()
            ))
            .default(true)
            .interact()?;

        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // 5. Format as addition
    let addition = formatter::format_addition(&vars)?;

    // ⚠️ Through `append_to_env_files`, like `add framework`, `add custom` and
    // `add blueprint` already do. This subcommand carried its own copy of the
    // same logic — which is why it silently missed the `.gitignore` check that
    // lives there, and would have kept missing anything else added to it.
    super::shared::append_to_env_files(
        Path::new(path),
        &addition,
        super::shared::AppendMode::WithConflictWarning,
        _verbose,
        &vars,
    )?;

    success(format!(
        "Added {} variables for {}",
        vars.vars.len(),
        display_name
    ));

    Ok(())
}
