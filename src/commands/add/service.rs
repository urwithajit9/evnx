use anyhow::Result;
use std::path::Path;

use super::shared::{confirm_and_apply, count, kind_hint, Flow, Outcome};
use crate::schema::{component, formatter};
use crate::utils::ui::{info, print_header, print_preview_header};

pub fn handle(service_id: &str, path: &str, yes: bool, verbose: bool) -> Result<Outcome> {
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

    let heading = || {
        print_header(
            "Add Service",
            Some(&format!("Adding variables for '{}'", service_id)),
        );
        print_preview_header();
    };
    let flow = Flow {
        noun: "variable",
        what: format!("for {display_name}"),
        terminal_example: Some(format!("evnx add service {service_id} --yes")),
        heading: &heading,
        preview: &formatter::generate_preview,
        question: &|n, file| format!("Add these {} to {file}?", count(n, "variable")),
        render: &|v| formatter::format_addition(v),
    };
    confirm_and_apply(Path::new(path), &vars, &flow, yes, verbose)
}
