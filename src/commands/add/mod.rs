//! Add command: Append environment variables to existing .env files.
//!
//! Subcommands:
//! - `service <id>`    — Add variables for a service (e.g., postgresql)
//! - `framework <id>`  — Add variables for a framework (e.g., django)
//! - `blueprint <id>`  — Add variables from a stack blueprint
//! - `custom`          — Interactive custom variable addition

use anyhow::Result;
// use colored::*;
use std::path::Path;

pub mod blueprint;
pub mod custom;
pub mod framework;
pub mod service;
pub mod shared;
use crate::utils::ui::{print_header, print_next_steps};

// use shared::{AppendMode, append_to_env_files};

/// Main entry point for `evnx add`
pub fn run(
    target: crate::cli::AddTarget,
    path: String,
    yes: bool,
    verbose: bool, // Keep but don't pass to handlers yet
) -> Result<()> {
    if !yes {
        print_header(
            "evnx add",
            Some("Add environment variables to existing .env files"),
        );
    }

    let output_path = Path::new(&path); // Convert to &str

    match target {
        crate::cli::AddTarget::Service { service } => {
            //service::handle(&service, output_path, yes)?;  // Remove verbose
            // service::handle expects (&str, &str, bool, bool)
            service::handle(&service, &path, yes, verbose)?;
        }
        crate::cli::AddTarget::Framework {
            language,
            framework,
        } => {
            framework::handle(&language, &framework, output_path, yes, verbose)?;
        }
        crate::cli::AddTarget::Blueprint { blueprint } => {
            blueprint::handle(&blueprint, output_path, yes, verbose)?;
        }
        crate::cli::AddTarget::Custom => {
            custom::handle(output_path, yes, verbose)?;
        }
    }

    // Common next steps at end
    if !yes {
        print_next_steps(&[
            "Edit .env and replace placeholder values",
            "Run 'evnx validate' to check for issues",
            "Use 'evnx add' again to add more variables",
        ]);
    }
    Ok(())
}
