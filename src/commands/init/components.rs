//! `evnx init --with <a,b,c>` — name the pieces directly.
//!
//! The same resolve → preview → confirm → write shape as [`super::blueprint`],
//! over an explicit component list rather than a named stack. A blueprint is
//! structurally one of these lists, so this is the general case and blueprints
//! are the shorthand.

use anyhow::{Context, Result};
use colored::Colorize;
use std::io::IsTerminal;
use std::path::Path;

use super::shared::{write_env_files, WriteOptions};
use crate::schema::{component, formatter};
use crate::utils::ui::{self, glyph};

/// Generate from an explicit component list.
pub fn handle(
    path: String,
    opts: WriteOptions,
    verbose: bool,
    components: &[String],
) -> Result<()> {
    // ⚠️ Resolve before anything is printed or written. An unknown name fails
    // here, with suggestions, rather than after a preview has implied it worked.
    let vars = component::resolve(components).context("Failed to resolve components")?;

    if verbose {
        println!(
            "{} {} components → {} variables",
            "[DEBUG]".dimmed(),
            components.len(),
            vars.vars.len()
        );
    }

    if !opts.yes {
        println!();
        for id in components {
            if let Some(c) = component::find(id)? {
                println!(
                    "  {}  {:<22} {}",
                    glyph::OK.green(),
                    c.display_name,
                    c.kind.label().dimmed()
                );
            }
        }
        println!("\n{}", formatter::generate_preview(&vars).dimmed());

        let confirm = dialoguer::Confirm::new()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact()?;
        if !confirm {
            println!("{}", "Aborted.".yellow());
            return Ok(());
        }
    }

    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;
    write_env_files(Path::new(&path), &example_content, &template_content, opts)?;

    ui::success(format!(
        "Created .env.example with {} variables from {} components",
        vars.vars.len(),
        components.len()
    ));
    Ok(())
}

/// Print every component `--with` accepts, grouped by kind.
///
/// This is the list the error message points at, so it has to be reachable
/// without a project, without a prompt, and without side effects.
pub fn list() -> Result<()> {
    let all = component::catalogue()?;

    for kind in [
        component::Kind::Framework,
        component::Kind::Service,
        component::Kind::Infrastructure,
    ] {
        let of_kind: Vec<_> = all.iter().filter(|c| c.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }

        println!("\n  {}", kind.label().to_uppercase().bold());
        let mut group = "";
        for c in of_kind {
            if c.group != group {
                group = &c.group;
                println!("  {}", group.dimmed());
            }
            println!("    {:<24} {}", c.id, c.display_name.dimmed());
        }
    }

    println!(
        "\n  {}  evnx init --with {}\n",
        glyph::ARROW.dimmed(),
        "nextjs,postgresql,stripe".dimmed()
    );
    Ok(())
}

/// The detected path: propose what the project already declares, and confirm.
///
/// ⚠️ Proposed, never applied. A `stripe` dependency in a test fixture would
/// otherwise add payment variables to a project with no payments, and the
/// package table is wrong the week a framework renames itself. The evidence is
/// printed beside every component so the proposal can be checked rather than
/// taken on faith.
pub fn handle_detected(
    path: String,
    opts: WriteOptions,
    verbose: bool,
    found: &[super::detect::Detected],
) -> Result<()> {
    let ids: Vec<String> = found.iter().map(|d| d.component.clone()).collect();
    let vars = component::resolve(&ids).context("Failed to resolve detected components")?;

    println!("\n  {}", "Detected".bold());
    for d in found {
        let name = component::find(&d.component)?
            .map(|c| c.display_name)
            .unwrap_or_else(|| d.component.clone());
        // Truncated rather than allowed to push the evidence column out of
        // alignment: `valkey/valkey:9-alpine` is longer than the column and the
        // tail of an image tag is not what identifies it.
        let matched = if d.matched.chars().count() > 22 {
            format!("{}…", d.matched.chars().take(21).collect::<String>())
        } else {
            d.matched.clone()
        };
        println!(
            "  {}  {:<22} {:<23} {}",
            glyph::OK.green(),
            name,
            matched.dimmed(),
            d.evidence.dimmed()
        );
    }
    println!(
        "\n  {} from {}\n",
        format!("{} variables", vars.vars.len()).bold(),
        format!("{} components", found.len()).dimmed()
    );

    // ⚠️ Refuse before prompting, rather than letting `dialoguer` fail with
    // "IO error: not a terminal" — which says nothing about what to do instead.
    // Detection is a *proposal*, so accepting it without a human is opting in,
    // and there has to be a way to say so.
    if !opts.yes && !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "detected {} components but there is nobody to confirm with.\n\n\
             Accept them:   evnx init --detect\n\
             Name them:     evnx init --with <a,b,c>\n\
             Skip them:     evnx init --yes      (writes blank files)",
            found.len()
        );
    }

    if !opts.yes {
        let confirm = dialoguer::Confirm::new()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact()?;
        if !confirm {
            println!(
                "  {}  nothing written — name them yourself with {}",
                glyph::INFO.dimmed(),
                "evnx init --with <a,b,c>".cyan()
            );
            return Ok(());
        }
    }

    if verbose {
        println!("{} {} detected components", "[DEBUG]".dimmed(), ids.len());
    }

    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;
    write_env_files(Path::new(&path), &example_content, &template_content, opts)?;

    ui::success(format!(
        "Created .env.example with {} variables",
        vars.vars.len()
    ));
    Ok(())
}
