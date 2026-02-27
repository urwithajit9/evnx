use crate::utils::ui::{info, print_header, success};
use anyhow::Result;
// use colored::*;
use std::path::Path;

use super::shared::write_env_files;

/// Handle Blank mode: create minimal .env files
pub fn handle(path: String, yes: bool, _verbose: bool) -> Result<()> {
    if !yes {
        print_header("Blank Template", Some("Creating minimal .env files"));
    }

    let example_content = "# Add your environment variables here\n# Format: KEY=value\n\n";
    let template_content = "# TODO: Replace with real values\n# Format: KEY=value\n\n";

    let output_path = Path::new(&path);
    write_env_files(output_path, example_content, template_content)?;

    success("Created empty .env.example");
    info("Tip: Run 'evnx add' to add variables interactively");

    Ok(())
}
