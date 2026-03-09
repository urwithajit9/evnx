/// Diff command - compare .env and .env.example
///
/// Shows missing, extra, and different variables between two env files
use anyhow::{Context, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::core::Parser;

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffResult {
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub different: Vec<DiffItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffItem {
    pub key: String,
    pub example_value: String,
    pub env_value: String,
}

pub fn run(
    env: String,
    example: String,
    show_values: bool,
    format: String,
    reverse: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running diff in verbose mode".dimmed());
    }

    println!(
        "\n{}",
        "┌─ Comparing .env ↔ .env.example ─────────────────────┐".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    let parser = Parser::default();

    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse {}", env))?;

    let example_file = parser
        .parse_file(&example)
        .with_context(|| format!("Failed to parse {}", example))?;

    let (left, right, left_name, right_name) = if reverse {
        (&example_file.vars, &env_file.vars, &example, &env)
    } else {
        (&env_file.vars, &example_file.vars, &env, &example)
    };

    let diff = compute_diff(left, right);

    match format.as_str() {
        "json" => output_json(&diff)?,
        "patch" => output_patch(&diff, left, right)?,
        _ => output_pretty(&diff, left, right, left_name, right_name, show_values)?,
    }

    Ok(())
}

fn compute_diff(left: &HashMap<String, String>, right: &HashMap<String, String>) -> DiffResult {
    let left_keys: HashSet<_> = left.keys().cloned().collect();
    let right_keys: HashSet<_> = right.keys().cloned().collect();

    let missing: Vec<String> = right_keys.difference(&left_keys).cloned().collect();

    let extra: Vec<String> = left_keys.difference(&right_keys).cloned().collect();

    let mut different = Vec::new();
    for key in left_keys.intersection(&right_keys) {
        let left_val = left.get(key).unwrap();
        let right_val = right.get(key).unwrap();

        if left_val != right_val {
            different.push(DiffItem {
                key: key.clone(),
                example_value: right_val.clone(),
                env_value: left_val.clone(),
            });
        }
    }

    DiffResult {
        missing,
        extra,
        different,
    }
}

fn output_pretty(
    diff: &DiffResult,
    left: &HashMap<String, String>,
    right: &HashMap<String, String>,
    left_name: &str,
    right_name: &str,
    show_values: bool,
) -> Result<()> {
    let has_changes =
        !diff.missing.is_empty() || !diff.extra.is_empty() || !diff.different.is_empty();

    if !has_changes {
        println!("{} Files are identical", "✓".green());
        return Ok(());
    }

    if !diff.missing.is_empty() {
        println!(
            "{}",
            format!("Missing from {} (present in {}):", left_name, right_name).bold()
        );
        for key in &diff.missing {
            if show_values {
                if let Some(val) = right.get(key) {
                    println!("  {} {} = {}", "+".green(), key.bold(), val.dimmed());
                }
            } else {
                println!("  {} {}", "+".green(), key.bold());
            }
        }
        println!();
    }

    if !diff.extra.is_empty() {
        println!(
            "{}",
            format!("Extra in {} (not in {}):", left_name, right_name).bold()
        );
        for key in &diff.extra {
            if show_values {
                if let Some(val) = left.get(key) {
                    println!("  {} {} = {}", "-".red(), key.bold(), val.dimmed());
                }
            } else {
                println!("  {} {}", "-".red(), key.bold());
            }
        }
        println!();
    }

    if !diff.different.is_empty() {
        println!("{}", "Different values:".bold());
        for item in &diff.different {
            println!("  {} {}", "~".yellow(), item.key.bold());
            if show_values {
                println!("    {}: {}", right_name, item.example_value.dimmed());
                println!("    {}: {}", left_name, item.env_value.dimmed());
            }
        }
        println!();
    }

    println!("{}", "Summary:".bold());
    println!("  {} missing (add to {})", diff.missing.len(), left_name);
    println!(
        "  {} extra (consider removing or adding to {})",
        diff.extra.len(),
        right_name
    );
    println!("  {} different values", diff.different.len());

    Ok(())
}

fn output_json(diff: &DiffResult) -> Result<()> {
    let json = serde_json::to_string_pretty(diff)?;
    println!("{}", json);
    Ok(())
}

fn output_patch(
    diff: &DiffResult,
    left: &HashMap<String, String>,
    right: &HashMap<String, String>,
) -> Result<()> {
    println!("# Add these to .env:");
    for key in &diff.missing {
        if let Some(val) = right.get(key) {
            println!("+ {}={}", key, val);
        }
    }

    println!("\n# Remove these from .env:");
    for key in &diff.extra {
        if let Some(val) = left.get(key) {
            println!("- {}={}", key, val);
        }
    }

    println!("\n# Update these in .env:");
    for item in &diff.different {
        println!("- {}={}", item.key, item.env_value);
        println!("+ {}={}", item.key, item.example_value);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_diff() {
        let mut left = HashMap::new();
        left.insert("KEY1".to_string(), "value1".to_string());
        left.insert("KEY2".to_string(), "value2".to_string());
        left.insert("EXTRA".to_string(), "extra".to_string());

        let mut right = HashMap::new();
        right.insert("KEY1".to_string(), "value1".to_string());
        right.insert("KEY2".to_string(), "different".to_string());
        right.insert("MISSING".to_string(), "missing".to_string());

        let diff = compute_diff(&left, &right);

        assert_eq!(diff.missing, vec!["MISSING"]);
        assert_eq!(diff.extra, vec!["EXTRA"]);
        assert_eq!(diff.different.len(), 1);
        assert_eq!(diff.different[0].key, "KEY2");
    }
}
