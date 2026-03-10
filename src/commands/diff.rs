/// Diff command - compare .env and .env.example
///
/// Shows missing, extra, and different variables between two env files
///
///  Features: Exit codes for CI/CD, auto-redaction of sensitive values
///  Features: Key order preservation, --ignore-keys filtering
/// Features: JSON statistics, interactive merge mode
use anyhow::{Context, Result};
use colored::*;
use indexmap::IndexMap; // Preserve insertion order
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::core::Parser;
use crate::utils::patterns; //  Reuse existing sensitive-key detection
                            // use crate::utils::ui; //   Reuse UI helpers if available

// ─────────────────────────────────────────────────────────────
// Data Structures (enhanced with redaction + stats)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffResult {
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub different: Vec<DiffItem>,
    ///  Optional statistics for JSON output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<DiffStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffItem {
    pub key: String,
    pub example_value: String,
    pub env_value: String,
    ///  Redacted versions for safe display (auto-populated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_value_redacted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_value_redacted: Option<String>,
}

///  Statistics for programmatic consumption
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffStats {
    pub total_keys_env: usize,
    pub total_keys_example: usize,
    pub overlap_count: usize,
    pub similarity_percent: f64,
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point ( returns exit code for CI/CD)
// ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    example: String,
    show_values: bool,
    format: String,
    reverse: bool,
    verbose: bool,
    ignore_keys: Vec<String>,
    with_stats: bool,
    interactive: bool,
) -> Result<i32> {
    //  CHANGED: Return exit code
    if verbose {
        eprintln!("{}", "🔍 Running diff in verbose mode".dimmed());
    }

    // Only print header for pretty output
    if format == "pretty" {
        println!(
            "\n{}",
            "┌─ Comparing .env ↔ .env.example ─────────────────────┐".cyan()
        );
        println!(
            "{}\n",
            "└──────────────────────────────────────────────────────┘".cyan()
        );
    }

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

    //  Convert ignore list to HashSet for O(1) lookups
    let ignore_set: HashSet<_> = ignore_keys.into_iter().collect();

    //  Compute diff with filtering + redaction
    let mut diff = compute_diff(left, right, &ignore_set);

    //  Add statistics if requested (JSON mode)
    if with_stats && format == "json" {
        diff.stats = Some(compute_stats(left, right, &diff));
    }

    // Route to output formatter
    match format.as_str() {
        "json" => output_json(&diff)?,
        "patch" => {
            //  Interactive mode only applies to patch format
            if interactive {
                output_patch_interactive(&diff, right, right_name)?;
            } else {
                output_patch(&diff, left, right)?;
            }
        }
        _ => output_pretty(&diff, left, right, left_name, right_name, show_values)?,
    }

    // Return appropriate exit code for CI/CD
    Ok(if diff.has_changes() { 1 } else { 0 })
}

// ─────────────────────────────────────────────────────────────
// Core Diff Logic ( uses IndexMap,  adds redaction)
// ─────────────────────────────────────────────────────────────

fn compute_diff(
    left: &IndexMap<String, String>,  //  CHANGED: HashMap → IndexMap
    right: &IndexMap<String, String>, //  CHANGED: HashMap → IndexMap
    ignore_keys: &HashSet<String>,    //  NEW: Filter parameter
) -> DiffResult {
    //  Filter out ignored keys first (preserves order via IndexMap)
    let left_filtered: IndexMap<_, _> = left
        .iter()
        .filter(|(k, _)| !ignore_keys.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let right_filtered: IndexMap<_, _> = right
        .iter()
        .filter(|(k, _)| !ignore_keys.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let left_keys: HashSet<_> = left_filtered.keys().cloned().collect();
    let right_keys: HashSet<_> = right_filtered.keys().cloned().collect();

    //  Preserve order: iterate through right_filtered for "missing"
    let missing: Vec<String> = right_filtered
        .keys()
        .filter(|k| !left_keys.contains(*k))
        .cloned()
        .collect();

    //  Preserve order: iterate through left_filtered for "extra"
    let extra: Vec<String> = left_filtered
        .keys()
        .filter(|k| !right_keys.contains(*k))
        .cloned()
        .collect();

    let mut different = Vec::new();
    for key in left_filtered.keys() {
        if let (Some(left_val), Some(right_val)) = (left_filtered.get(key), right_filtered.get(key))
        {
            if left_val != right_val {
                // Auto-redact sensitive values
                let (ex_redacted, env_redacted) = redact_if_sensitive(key, left_val, right_val);

                different.push(DiffItem {
                    key: key.clone(),
                    example_value: right_val.clone(),
                    env_value: left_val.clone(),
                    example_value_redacted: ex_redacted,
                    env_value_redacted: env_redacted,
                });
            }
        }
    }

    DiffResult {
        missing,
        extra,
        different,
        stats: None,
    }
}

// Security: Auto-redact values for sensitive keys
fn redact_if_sensitive(
    key: &str,
    env_val: &str,
    example_val: &str,
) -> (Option<String>, Option<String>) {
    // Reuse your existing utils::patterns::is_sensitive_key if available
    // Otherwise, inline simple pattern matching:
    let is_sensitive = patterns::is_sensitive_key(key); // ✅ Reuse existing util

    if is_sensitive {
        let redact = |v: &str| {
            if v.is_empty() {
                "***".to_string()
            } else {
                // Show first 2 chars + mask rest: "ab***"
                format!("{}***", &v[..v.len().min(2)])
            }
        };
        (Some(redact(example_val)), Some(redact(env_val)))
    } else {
        (None, None)
    }
}

// Compute statistics for JSON output
fn compute_stats(
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    _diff: &DiffResult,
) -> DiffStats {
    let left_keys: HashSet<_> = left.keys().collect();
    let right_keys: HashSet<_> = right.keys().collect();

    // Count keys present in BOTH maps
    let overlap_count = left_keys.intersection(&right_keys).count();

    let total_left = left.len();
    let total_right = right.len();

    // ✅ Sørensen-Dice coefficient: 2*|A∩B| / (|A|+|B|) * 100
    // This gives 50% for 1 overlap out of 2 keys each, which is intuitive
    let similarity = if total_left + total_right > 0 {
        (2.0 * overlap_count as f64 / (total_left + total_right) as f64) * 100.0
    } else {
        100.0
    };

    DiffStats {
        total_keys_env: total_left,
        total_keys_example: total_right,
        overlap_count,
        similarity_percent: (similarity * 100.0).round() / 100.0,
    }
}

// ─────────────────────────────────────────────────────────────
// Output Formatters
// ─────────────────────────────────────────────────────────────

fn output_pretty(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    left_name: &str,
    right_name: &str,
    show_values: bool,
) -> Result<()> {
    if !diff.has_changes() {
        println!("{} Files are identical", "✓".green());
        return Ok(());
    }

    if !diff.missing.is_empty() {
        println!(
            "{}",
            format!("Missing from {} (present in {}):", left_name, right_name).bold()
        );
        for key in &diff.missing {
            if let Some(val) = right.get(key) {
                // Use redacted value if available, else original (if show_values)
                let display_val = diff
                    .different
                    .iter()
                    .find(|d| &d.key == key)
                    .and_then(|d| {
                        if show_values {
                            d.example_value_redacted.as_ref()
                        } else {
                            None
                        }
                    })
                    .or(if show_values { Some(val) } else { None });

                if let Some(display) = display_val {
                    println!("  {} {} = {}", "+".green(), key.bold(), display.dimmed());
                } else {
                    println!("  {} {}", "+".green(), key.bold());
                }
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
            if let Some(val) = left.get(key) {
                let display_val = diff
                    .different
                    .iter()
                    .find(|d| &d.key == key)
                    .and_then(|d| {
                        if show_values {
                            d.env_value_redacted.as_ref()
                        } else {
                            None
                        }
                    })
                    .or(if show_values { Some(val) } else { None });

                if let Some(display) = display_val {
                    println!("  {} {} = {}", "-".red(), key.bold(), display.dimmed());
                } else {
                    println!("  {} {}", "-".red(), key.bold());
                }
            }
        }
        println!();
    }

    if !diff.different.is_empty() {
        println!("{}", "Different values:".bold());
        for item in &diff.different {
            println!("  {} {}", "~".yellow(), item.key.bold());
            // Always show redacted values if available, regardless of show_values
            if item.example_value_redacted.is_some() || show_values {
                let ex_display = item
                    .example_value_redacted
                    .as_ref()
                    .unwrap_or(&item.example_value);
                let env_display = item.env_value_redacted.as_ref().unwrap_or(&item.env_value);
                println!("    {}: {}", right_name, ex_display.dimmed());
                println!("    {}: {}", left_name, env_display.dimmed());
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

    //  Show stats in pretty mode if available
    if let Some(stats) = &diff.stats {
        println!("\n{}", "Statistics:".bold());
        println!("  • Similarity: {:.1}%", stats.similarity_percent);
    }

    Ok(())
}

fn output_json(diff: &DiffResult) -> Result<()> {
    let json = serde_json::to_string_pretty(diff)?;
    println!("{}", json);
    Ok(())
}

fn output_patch(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
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

// Interactive merge mode for patch format
fn output_patch_interactive(
    diff: &DiffResult,
    right: &IndexMap<String, String>,
    _target_file: &str,
) -> Result<()> {
    use std::io::{self, Write};

    println!("\n{}", "🔧 Interactive Merge Mode".bold());
    println!("Prompts: (y)es, (n)o, (s)kip rest\n");

    let mut applied = Vec::new();

    // Handle missing keys
    for key in &diff.missing {
        if let Some(val) = right.get(key) {
            print!("➕ Add {}={}? [y/n/s]: ", key, val);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_lowercase().as_str() {
                "y" | "yes" => {
                    println!("   ✓ Queued: + {}={}", key, val);
                    applied.push(('+', key.clone(), val.clone()));
                }
                "s" | "skip" => break,
                _ => println!("   ✗ Skipped"),
            }
        }
    }

    // Handle different values
    for item in &diff.different {
        print!(
            "✏️  Update {}?\n   - {}\n   + {}? [y/n/s]: ",
            item.key, item.env_value, item.example_value
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => {
                println!("   ✓ Queued: + {}={}", item.key, item.example_value);
                applied.push(('~', item.key.clone(), item.example_value.clone()));
            }
            "s" | "skip" => break,
            _ => println!("   ✗ Skipped"),
        }
    }

    // Summary of applied changes
    if !applied.is_empty() {
        println!("\n{}", "📋 Applied changes:".bold());
        for (op, key, val) in &applied {
            let icon = match op {
                '+' => "+".green(),
                '~' => "~".yellow(),
                _ => "-".red(),
            };
            println!("  {} {}={}", icon, key, val);
        }
        println!("\n💡 Tip: Save output and apply with: patch -p1 < changes.patch");
    } else {
        println!("\n{}", "ℹ️  No changes applied".dimmed());
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Methods
// ─────────────────────────────────────────────────────────────

impl DiffResult {
    /// Check if any differences were found (for exit code logic)
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.missing.is_empty() || !self.extra.is_empty() || !self.different.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────
// Tests (enhanced for new features)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::indexmap;

    #[test]
    fn test_compute_diff_basic() {
        let left = indexmap! {
            "KEY1".to_string() => "value1".to_string(),
            "KEY2".to_string() => "value2".to_string(),
            "EXTRA".to_string() => "extra".to_string(),
        };
        let right = indexmap! {
            "KEY1".to_string() => "value1".to_string(),
            "KEY2".to_string() => "different".to_string(),
            "MISSING".to_string() => "missing".to_string(),
        };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);

        assert_eq!(diff.missing, vec!["MISSING"]);
        assert_eq!(diff.extra, vec!["EXTRA"]);
        assert_eq!(diff.different.len(), 1);
        assert_eq!(diff.different[0].key, "KEY2");
    }

    #[test]
    fn test_ignore_keys_filtering() {
        let left = indexmap! { "IGNORED".to_string() => "a".to_string() };
        let right = indexmap! { "IGNORED".to_string() => "b".to_string() };
        let mut ignore = HashSet::new();
        ignore.insert("IGNORED".to_string());

        let diff = compute_diff(&left, &right, &ignore);

        assert!(diff.missing.is_empty());
        assert!(diff.extra.is_empty());
        assert!(diff.different.is_empty());
    }

    #[test]
    fn test_redact_sensitive_values() {
        let (ex_red, env_red) = redact_if_sensitive("DB_PASSWORD", "secret123", "secret456");
        assert!(ex_red.as_ref().unwrap().contains("***"));
        assert!(env_red.as_ref().unwrap().contains("***"));
        assert!(!ex_red.as_ref().unwrap().contains("secret123"));

        // Non-sensitive key should not be redacted
        let (ex_red, env_red) = redact_if_sensitive("APP_NAME", "MyApp", "MyApp");
        assert!(ex_red.is_none());
        assert!(env_red.is_none());
    }

    #[test]
    fn test_order_preservation() {
        // IndexMap should preserve insertion order
        let left = indexmap! {
            "A".to_string() => "1".to_string(),
            "B".to_string() => "2".to_string(),
            "C".to_string() => "3".to_string(),
        };
        let right = indexmap! {
            "C".to_string() => "3".to_string(),
            "D".to_string() => "4".to_string(),  // Missing in left
            "A".to_string() => "1".to_string(),
        };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);

        // Missing should follow right's order: D appears after C in right
        assert_eq!(diff.missing, vec!["D"]);
        // Extra should follow left's order: B
        assert_eq!(diff.extra, vec!["B"]);
    }

    #[test]
    fn test_has_changes() {
        let diff_empty = DiffResult {
            missing: vec![],
            extra: vec![],
            different: vec![],
            stats: None,
        };
        assert!(!diff_empty.has_changes());

        let diff_with_missing = DiffResult {
            missing: vec!["NEW_KEY".to_string()],
            extra: vec![],
            different: vec![],
            stats: None,
        };
        assert!(diff_with_missing.has_changes());
    }

    #[test]
    fn test_compute_stats() {
        let left =
            indexmap! { "A".to_string() => "1".to_string(), "B".to_string() => "2".to_string() };
        let right =
            indexmap! { "A".to_string() => "1".to_string(), "C".to_string() => "3".to_string() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.total_keys_env, 2);
        assert_eq!(stats.total_keys_example, 2);
        assert_eq!(stats.overlap_count, 1); // Only "A" overlaps
        assert!((stats.similarity_percent - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_stats_identical_files() {
        let left = indexmap! { "A".into() => "1".into(), "B".into() => "2".into() };
        let right = indexmap! { "A".into() => "1".into(), "B".into() => "2".into() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 2);
        assert_eq!(stats.similarity_percent, 100.0);
    }

    #[test]
    fn test_compute_stats_no_overlap() {
        let left = indexmap! { "A".into() => "1".into() };
        let right = indexmap! { "B".into() => "2".into() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 0);
        assert_eq!(stats.similarity_percent, 0.0);
    }

    #[test]
    fn test_compute_stats_empty_files() {
        let left = indexmap! {};
        let right = indexmap! {};
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.similarity_percent, 100.0); // Empty files are "identical"
    }
}
