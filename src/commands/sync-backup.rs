/// Sync command - keep .env and .env.example in sync
///
/// Helps maintain consistency between development and template files
use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::collections::HashSet;
use std::fs;

use crate::core::Parser;

pub fn run(direction: String, placeholder: bool, verbose: bool) -> Result<()> {
    if verbose {
        println!("{}", "Running sync in verbose mode".dimmed());
    }

    println!(
        "\n{}",
        "┌─ Sync .env ↔ .env.example ──────────────────────────┐".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    match direction.as_str() {
        "forward" => sync_forward(placeholder, verbose),
        "reverse" => sync_reverse(placeholder, verbose),
        _ => {
            eprintln!("{} Invalid direction: {}", "✗".red(), direction);
            eprintln!("Valid directions: forward, reverse");
            std::process::exit(1);
        }
    }
}

/// Sync .env → .env.example (add new vars to example)
fn sync_forward(use_placeholders: bool, _verbose: bool) -> Result<()> {
    // Parse both files
    let parser = Parser::default();

    let env_file = parser.parse_file(".env").context("Failed to parse .env")?;

    let example_file = match parser.parse_file(".env.example") {
        Ok(f) => f,
        Err(_) => {
            println!("{} .env.example not found, creating from .env", "ℹ️".cyan());

            if use_placeholders {
                convert_to_example(&env_file.vars)?;
            } else {
                fs::copy(".env", ".env.example")?;
            }

            println!("{} Created .env.example", "✓".green());
            return Ok(());
        }
    };

    // Find variables in .env but not in .env.example
    let env_keys: HashSet<_> = env_file.vars.keys().collect();
    let example_keys: HashSet<_> = example_file.vars.keys().collect();

    let missing: Vec<_> = env_keys.difference(&example_keys).cloned().collect();

    if missing.is_empty() {
        println!("{} .env.example is up to date", "✓".green());
        return Ok(());
    }

    println!(
        "{}",
        "Found variables in .env that are missing from .env.example:".bold()
    );
    for key in &missing {
        println!("  • {}", key.yellow());
    }
    println!();

    // Ask how to add them
    let choices = vec![
        "Yes, add with placeholder values",
        "Yes, add with actual values (not recommended)",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt("Add these to .env.example?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => add_with_placeholders(&missing, &env_file.vars)?,
        1 => add_with_actual_values(&missing, &env_file.vars)?,
        2 => add_interactively(&missing, &env_file.vars)?,
        3 => {
            println!("{} No changes made", "ℹ️".cyan());
            return Ok(());
        }
        _ => unreachable!(),
    }

    // Optionally add a comment
    if Confirm::new()
        .with_prompt("Add a comment explaining these variables?")
        .default(true)
        .interact()?
    {
        let comment: String = Input::new()
            .with_prompt("Comment")
            .default("Additional configuration".to_string())
            .interact_text()?;

        let mut content = fs::read_to_string(".env.example")?;
        content.push_str(&format!("\n# {}\n", comment));
        fs::write(".env.example", content)?;
    }

    println!("\n{} Updated .env.example", "✓".green());
    println!("{} Added {} variables", "✓".green(), missing.len());
    println!("\n{}", "Remember to commit .env.example to Git.".yellow());

    Ok(())
}

/// Sync .env.example → .env (add missing vars to .env)
fn sync_reverse(_use_placeholders: bool, _verbose: bool) -> Result<()> {
    let parser = Parser::default();

    let example_file = parser
        .parse_file(".env.example")
        .context("Failed to parse .env.example")?;

    let env_file = match parser.parse_file(".env") {
        Ok(f) => f,
        Err(_) => {
            println!("{} .env not found, creating from .env.example", "ℹ️".cyan());
            fs::copy(".env.example", ".env")?;
            println!("{} Created .env", "✓".green());
            println!(
                "\n{}",
                "Replace placeholder values with real credentials!"
                    .yellow()
                    .bold()
            );
            return Ok(());
        }
    };

    // Find variables in .env.example but not in .env
    let env_keys: HashSet<_> = env_file.vars.keys().collect();
    let example_keys: HashSet<_> = example_file.vars.keys().collect();

    let missing: Vec<_> = example_keys.difference(&env_keys).cloned().collect();

    if missing.is_empty() {
        println!("{} .env is up to date", "✓".green());
        return Ok(());
    }

    println!(
        "{}",
        "Found variables in .env.example that are missing from .env:".bold()
    );
    for key in &missing {
        println!("  • {}", key.yellow());
    }
    println!();

    // Ask how to add them
    let choices = vec![
        "Yes, with placeholder values",
        "Yes, prompt me for real values",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt("Add to .env?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            // Add with placeholder values
            let mut content = fs::read_to_string(".env")?;
            content.push_str("\n# Synced from .env.example\n");
            for key in &missing {
                if let Some(value) = example_file.vars.get(*key) {
                    content.push_str(&format!("{}={}\n", key, value));
                }
            }
            fs::write(".env", content)?;
        }
        1 => {
            // Prompt for real values
            let mut content = fs::read_to_string(".env")?;
            content.push_str("\n# Synced from .env.example\n");
            for key in &missing {
                let example_value = example_file
                    .vars
                    .get(*key)
                    .map(|s| s.as_str())
                    .unwrap_or("");

                let value: String = Input::new()
                    .with_prompt(format!("Value for {}", key))
                    .default(example_value.to_string())
                    .interact_text()?;

                content.push_str(&format!("{}={}\n", key, value));
            }
            fs::write(".env", content)?;
        }
        2 => {
            // Choose individually
            let selected = MultiSelect::new()
                .with_prompt("Select variables to add")
                .items(&missing)
                .interact()?;

            let mut content = fs::read_to_string(".env")?;
            content.push_str("\n# Synced from .env.example\n");
            for &idx in &selected {
                let key = missing[idx];
                if let Some(value) = example_file.vars.get(key) {
                    content.push_str(&format!("{}={}\n", key, value));
                }
            }
            fs::write(".env", content)?;
        }
        3 => {
            println!("{} No changes made", "ℹ️".cyan());
            return Ok(());
        }
        _ => unreachable!(),
    }

    println!("\n{} Updated .env", "✓".green());
    println!("{} Added {} variables", "✓".green(), missing.len());

    Ok(())
}

/// Add variables with placeholder values
fn add_with_placeholders(
    keys: &[&String],
    values: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let mut content = fs::read_to_string(".env.example")?;
    content.push_str("\n# Synced from .env\n");

    for key in keys {
        let placeholder = generate_placeholder(key, values.get(*key));
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    fs::write(".env.example", content)?;
    Ok(())
}

/// Add variables with actual values
fn add_with_actual_values(
    keys: &[&String],
    values: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let mut content = fs::read_to_string(".env.example")?;
    content.push_str("\n# Synced from .env\n");

    for key in keys {
        if let Some(value) = values.get(*key) {
            content.push_str(&format!("{}={}\n", key, value));
        }
    }

    fs::write(".env.example", content)?;
    Ok(())
}

/// Add variables interactively
fn add_interactively(
    keys: &[&String],
    values: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;

    let mut content = fs::read_to_string(".env.example")?;
    content.push_str("\n# Synced from .env\n");

    for &idx in &selected {
        let key = keys[idx];
        let placeholder = generate_placeholder(key, values.get(key));
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    fs::write(".env.example", content)?;
    Ok(())
}

/// Convert actual values to placeholders for .env.example
fn convert_to_example(vars: &std::collections::HashMap<String, String>) -> Result<()> {
    let mut content = String::new();
    content.push_str("# Generated from .env\n");
    content.push_str("# Replace all placeholder values with real credentials\n\n");

    for (key, value) in vars {
        let placeholder = generate_placeholder(key, Some(value));
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    fs::write(".env.example", content)?;
    Ok(())
}

/// Generate a placeholder value based on the key name
fn generate_placeholder(key: &str, value: Option<&String>) -> String {
    let key_upper = key.to_uppercase();

    // Check for specific patterns
    if key_upper.contains("SECRET") || key_upper.contains("KEY") || key_upper.contains("TOKEN") {
        return "YOUR_KEY_HERE".to_string();
    }

    if key_upper.contains("PASSWORD") || key_upper.contains("PASS") {
        return "YOUR_PASSWORD_HERE".to_string();
    }

    if key_upper.contains("URL") {
        if let Some(v) = value {
            // Try to preserve structure
            if v.contains("postgresql://") {
                return "postgresql://user:password@localhost:5432/dbname".to_string();
            }
            if v.contains("redis://") {
                return "redis://localhost:6379/0".to_string();
            }
            if v.contains("http://") || v.contains("https://") {
                return "https://your-api-url-here.com".to_string();
            }
        }
        return "YOUR_URL_HERE".to_string();
    }

    if key_upper.contains("PORT") {
        return "8000".to_string();
    }

    if key_upper.contains("DEBUG") {
        return "True".to_string();
    }

    if key_upper.contains("HOST") {
        return "localhost".to_string();
    }

    // Default placeholder
    "YOUR_VALUE_HERE".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_placeholder() {
        assert_eq!(generate_placeholder("SECRET_KEY", None), "YOUR_KEY_HERE");
        assert_eq!(generate_placeholder("API_TOKEN", None), "YOUR_KEY_HERE");
        assert_eq!(
            generate_placeholder("DB_PASSWORD", None),
            "YOUR_PASSWORD_HERE"
        );
        assert_eq!(generate_placeholder("PORT", None), "8000");
        assert_eq!(generate_placeholder("DEBUG", None), "True");
        assert_eq!(generate_placeholder("RANDOM_VAR", None), "YOUR_VALUE_HERE");
    }

    #[test]
    fn test_generate_placeholder_with_value() {
        let url = "postgresql://user:pass@localhost:5432/db".to_string();
        assert!(generate_placeholder("DATABASE_URL", Some(&url)).contains("postgresql"));

        let redis = "redis://localhost:6379/0".to_string();
        assert!(generate_placeholder("REDIS_URL", Some(&redis)).contains("redis"));
    }
}
