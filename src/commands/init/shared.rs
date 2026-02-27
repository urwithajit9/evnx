use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::Path;

/// Write .env.example and .env files, update .gitignore
pub fn write_env_files(
    output_path: &Path,
    example_content: &str,
    template_content: &str,
) -> Result<()> {
    // Ensure directory exists
    fs::create_dir_all(output_path)
        .with_context(|| format!("Failed to create directory: {}", output_path.display()))?;

    // Write .env.example
    let example_path = output_path.join(".env.example");
    fs::write(&example_path, example_content.trim())
        .with_context(|| format!("Failed to write: {}", example_path.display()))?;

    // Write .env (only if doesn't exist)
    let env_path = output_path.join(".env");
    if !env_path.exists() {
        fs::write(&env_path, template_content)
            .with_context(|| format!("Failed to write: {}", env_path.display()))?;
        println!("{} Created .env from template", "✓".green());
    }

    // Update .gitignore
    update_gitignore(output_path)?;

    Ok(())
}

/// Add .env patterns to .gitignore if not present
fn update_gitignore(output_path: &Path) -> Result<()> {
    let gitignore_path = output_path.join(".gitignore");
    let gitignore_entry = "\n# Environment files\n.env\n.env.local\n.env.*.local\n";

    if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path).context("Failed to read .gitignore")?;

        if !content.contains(".env\n") && !content.ends_with(".env") {
            let mut updated = content;
            updated.push_str(gitignore_entry);
            fs::write(&gitignore_path, updated).context("Failed to update .gitignore")?;
            println!("{} Added .env to .gitignore", "✓".green());
        }
    } else {
        fs::write(&gitignore_path, gitignore_entry.trim_start())
            .context("Failed to create .gitignore")?;
        println!("{} Created .gitignore", "✓".green());
    }
    Ok(())
}
