use crate::utils::ui::{info, print_header, success};
use anyhow::Result;
// use colored::*;
use std::path::Path;

use super::shared::{write_env_files, WriteOptions};

/// Handle Blank mode: create minimal .env files
pub fn handle(path: String, opts: WriteOptions, _verbose: bool) -> Result<()> {
    if !opts.yes {
        print_header("Blank Template", Some("Creating minimal .env files"));
    }

    let example_content = "# Add your environment variables here\n# Format: KEY=value\n\n";
    let template_content = "# TODO: Replace with real values\n# Format: KEY=value\n\n";

    let output_path = Path::new(&path);
    write_env_files(output_path, example_content, template_content, opts)?;

    success("Created empty .env.example");
    info("Tip: Run 'evnx add' to add variables interactively");

    Ok(())
}
