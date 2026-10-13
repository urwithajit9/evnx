use anyhow::Result;
use colored::*;
use std::path::Path;

use super::shared::{aborted, count, kind_hint, Outcome};
use crate::schema::{component, formatter};
use crate::utils::prompt::{require_terminal, yes_no};
use crate::utils::ui::{info, print_header, print_preview_header, success};

pub fn handle(service_id: &str, path: &str, yes: bool, _verbose: bool) -> Result<Outcome> {
    let vars = component::resolve(&[service_id.to_string()])?;
    let found = component::find(service_id)?;
    // One id space (I-018): an infra or framework id is added, and named for
    // what it is. `add infra` is this same command.
    if let Some(c) = found
        .as_ref()
        .filter(|c| c.kind != component::Kind::Service)
    {
        info(kind_hint(c));
    }
    let display_name = found
        .map(|c| c.display_name)
        .unwrap_or_else(|| service_id.to_string());

    // `--yes` never previews (DQ-A6): the result line says what happened.
    if !yes {
        // Before the preview: without a terminal the question cannot be asked,
        // and dialoguer's own error says nothing about `--yes` (I-020).
        require_terminal(&format!("evnx add service {service_id} --yes"))?;
        print_header(
            "Add Service",
            Some(&format!("Adding variables for '{}'", service_id)),
        );
        print_preview_header();
        println!("{}", formatter::generate_preview(&vars).dimmed());

        let confirm = yes_no()
            .with_prompt(format!(
                "Add these {} to .env.example?",
                count(vars.vars.len(), "variable")
            ))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
        }
    }

    let addition = formatter::format_addition(&vars)?;

    super::shared::append_to_env_files(
        Path::new(path),
        &addition,
        super::shared::AppendMode::WithConflictWarning,
        _verbose,
        &vars,
    )?;

    success(format!(
        "Added {} for {}",
        count(vars.vars.len(), "variable"),
        display_name
    ));

    Ok(Outcome::Written)
}
