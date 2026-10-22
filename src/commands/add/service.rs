use anyhow::Result;
use std::path::Path;

use super::shared::{confirm_and_apply, kind_hint, Flow, Mode, Outcome};
use crate::schema::{component, formatter};
use crate::utils::string::count;
use crate::utils::ui::{info, print_header, print_preview_header};

pub fn handle(service_id: Option<&str>, path: &str, mode: &Mode) -> Result<Outcome> {
    // No id: pick one from the whole catalogue (one id space, I-018).
    let picked;
    let service_id = match service_id {
        Some(id) => id,
        None => {
            let items: Vec<(String, String)> = component::catalogue()?
                .into_iter()
                .map(|c| (c.id, format!("{} [{}]", c.display_name, c.kind.label())))
                .collect();
            match super::pick(mode, "component", &items)? {
                Some(id) => {
                    picked = id;
                    picked.as_str()
                }
                None => return Ok(super::shared::aborted()),
            }
        }
    };
    let mut vars = component::resolve(&[service_id.to_string()])?;
    add_browser_copies(&mut vars, Path::new(path), mode.verbose);
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
        example_render: None,
        env_from_values: false,
    };
    confirm_and_apply(Path::new(path), &vars, &flow, mode)
}

/// Browser copies of the public values, for the framework this project uses
/// (BR-1, DQ-A8).
///
/// `init --with vite,supabase` writes `VITE_SUPABASE_URL` beside
/// `SUPABASE_URL`, because Vite only exposes `VITE_*` to the browser.
/// `add service supabase` did not: it was never told the framework. It now
/// asks `init --detect`'s reading of the project (`package.json` and the
/// rest), and makes the same copies for each browser framework it finds. Only
/// the values their vendors design to be public get a copy
/// ([`component::CLIENT_SAFE`]); a service with none of them is left alone and
/// nothing is said.
fn add_browser_copies(vars: &mut crate::schema::models::VarCollection, root: &Path, verbose: bool) {
    let detected = crate::commands::init::detect::detect(root).unwrap_or_default();
    // A browser name for a framework this project does not use is dropped,
    // as `init` drops it (I-002b, DQ-23): Clerk's NEXT_PUBLIC_ key outside
    // Next.js.
    let found: Vec<&str> = detected.iter().map(|d| d.component.as_str()).collect();
    component::browser_names_for(vars, &found);
    let has_public = vars
        .vars
        .keys()
        .any(|k| component::CLIENT_SAFE.contains(&k.as_str()));
    if !has_public {
        return;
    }
    let mut found_any = false;
    for d in &detected {
        let Some((_, prefix)) = component::CLIENT_PREFIX
            .iter()
            .find(|(id, _)| *id == d.component)
        else {
            continue;
        };
        found_any = true;
        let before = vars.vars.len();
        component::add_client_copies(vars, &d.component);
        let added = vars.vars.len() - before;
        if added > 0 {
            let copies = if added == 1 { "copy" } else { "copies" };
            info(format!(
                "{} found ({}): adding {added} {prefix} {copies} for the browser",
                d.component, d.evidence
            ));
        }
    }
    // ⚠️ Only with `-v` (N-22, owner decision 2026-10-17). In a backend
    // project it was said on every add of Stripe, Sentry, Auth0 …: nothing to
    // act on, every time.
    if !found_any && verbose {
        info(
            "No browser framework found in this project, so no client-side copies \
             (VITE_, NEXT_PUBLIC_, ...) were added",
        );
    }
}
