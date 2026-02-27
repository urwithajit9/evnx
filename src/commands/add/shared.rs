//! Shared utilities for add subcommands

use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::Path;

use crate::schema::models::VarCollection;

/// How to handle conflicts when appending
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppendMode {
    /// Warn about conflicts but still append (user sees warning)
    WithConflictWarning,
    /// Automatically skip conflicting variables
    SkipConflicts,
    /// Overwrite existing values (use with caution!)
    Overwrite,
}

/// Represents a variable conflict
#[derive(Debug, Clone)]
pub struct Conflict {
    pub var_name: String,
    pub existing_value: String,
    pub new_value: String,
    pub existing_line: usize,
}

/// Detect conflicts between existing content and new variables
pub fn detect_conflicts(existing_content: &str, new_vars: &VarCollection) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    // Parse existing KEY=value lines
    for (line_num, line) in existing_content.lines().enumerate() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Parse KEY=value
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            // Check if this key is in new vars
            if let Some(new_meta) = new_vars.vars.get(key) {
                if value != new_meta.example_value {
                    conflicts.push(Conflict {
                        var_name: key.to_string(),
                        existing_value: value.to_string(),
                        new_value: new_meta.example_value.clone(),
                        existing_line: line_num + 1,
                    });
                }
            }
        }
    }

    conflicts
}

/// Append content to .env.example and .env files
pub fn append_to_env_files(
    output_path: &Path,
    addition: &str,
    mode: AppendMode,
    verbose: bool,
) -> Result<()> {
    let example_path = output_path.join(".env.example");
    let env_path = output_path.join(".env");

    // Handle .env.example
    if example_path.exists() {
        let existing = fs::read_to_string(&example_path).context("Failed to read .env.example")?;

        let updated = match mode {
            AppendMode::SkipConflicts => {
                // This is handled upstream by filtering vars
                format!("{}\n{}", existing.trim_end(), addition)
            }
            _ => format!("{}\n{}", existing.trim_end(), addition),
        };

        fs::write(&example_path, updated).context("Failed to write .env.example")?;

        if verbose {
            println!(
                "{} Appended to {}",
                "[DEBUG]".dimmed(),
                example_path.display()
            );
        }
    } else {
        // Create new file
        fs::write(&example_path, addition.trim()).context("Failed to create .env.example")?;

        if verbose {
            println!("{} Created {}", "[DEBUG]".dimmed(), example_path.display());
        }
    }

    // Handle .env (add TODO placeholders)
    if env_path.exists() {
        let existing = fs::read_to_string(&env_path)?;

        // Convert addition to TODO format
        let todo_addition = addition
            .lines()
            .map(|line| {
                // Preserve comments and section headers
                if line.trim().is_empty() || line.trim().starts_with('#') || line.contains("──")
                {
                    line.to_string()
                } else if let Some(eq_pos) = line.find('=') {
                    let _key = &line[..eq_pos];
                    let _value = &line[eq_pos + 1..];
                    format!("# TODO: {}  # <-- Fill in real value", line)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let updated = format!("{}\n\n{}", existing.trim_end(), todo_addition);
        fs::write(&env_path, updated)?;

        if verbose {
            println!(
                "{} Updated {} with TODO placeholders",
                "[DEBUG]".dimmed(),
                env_path.display()
            );
        }
    }

    Ok(())
}

/// Format a single variable as a .env line with optional metadata
pub fn format_var_line(
    name: &str,
    example_value: &str,
    description: Option<&str>,
    required: bool,
    _category: Option<&str>,
) -> String {
    let mut lines = Vec::new();

    if let Some(desc) = description {
        lines.push(format!("# {}", desc));
    }

    lines.push(format!("{}={}", name, example_value));

    if required {
        lines.push("  # (required)".to_string());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::models::{VarMetadata, VarSource};
    // use std::collections::HashMap;

    #[test]
    fn test_detect_conflicts_finds_mismatched_values() {
        let existing = "DATABASE_URL=old_value\nREDIS_URL=redis://localhost\n";

        let mut vars = VarCollection::default();
        vars.vars.insert(
            "DATABASE_URL".to_string(),
            VarMetadata {
                example_value: "new_value".to_string(),
                description: None,
                category: None,
                required: false,
                source: VarSource::Service("postgresql".to_string()),
            },
        );
        vars.vars.insert(
            "REDIS_URL".to_string(),
            VarMetadata {
                example_value: "redis://localhost".to_string(), // Same value = no conflict
                description: None,
                category: None,
                required: false,
                source: VarSource::Service("redis".to_string()),
            },
        );

        let conflicts = detect_conflicts(existing, &vars);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].var_name, "DATABASE_URL");
        assert_eq!(conflicts[0].existing_value, "old_value");
        assert_eq!(conflicts[0].new_value, "new_value");
    }

    #[test]
    fn test_format_var_line_with_metadata() {
        let line = format_var_line(
            "API_KEY",
            "sk_test_xxx",
            Some("API key for testing"),
            true,
            Some("Auth"),
        );

        assert!(line.contains("# API key for testing"));
        assert!(line.contains("API_KEY=sk_test_xxx"));
        assert!(line.contains("(required)"));
    }
}
