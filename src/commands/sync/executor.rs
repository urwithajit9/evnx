//! Execution logic for sync operations: forward, reverse, previews, and file writes.

use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::Parser;
use crate::utils::ui;

use super::models::{PlaceholderConfig, SyncAction, SyncPreview, VarChange};
use super::placeholder::{generate_placeholder, is_placeholder_value};
use super::security::{check_env_permissions, log_security_event, validate_var_name};

/// Atomic write: write to temp file, then rename to target
fn atomic_write<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    let path = path.as_ref();
    let temp = NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;

    temp.as_file().write_all(content.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;

    // Set restrictive permissions on .env files
    if path.file_name() == Some(std::ffi::OsStr::new(".env")) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

/// Internal context struct to reduce parameter passing
pub(super) struct SyncCtx {
    pub direction: SyncDirection,
    pub placeholder: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub force: bool,
    pub template_config: Option<PathBuf>,
    pub naming_policy: NamingPolicy,
}

/// Main execution entry point (called from mod.rs)
pub fn execute(ctx: SyncCtx) -> Result<()> {
    if ctx.verbose {
        println!(
            "{}",
            format!(
                "Running sync in verbose mode [direction={:?}, dry_run={}, force={}]",
                ctx.direction, ctx.dry_run, ctx.force
            )
            .dimmed()
        );
    }

    let config = ctx
        .template_config
        .as_ref()
        .map(PlaceholderConfig::from_path)
        .transpose()?
        .unwrap_or_default();

    ui::print_header(
        "Sync .env ↔ .env.example",
        Some(&format!("Direction: {}", ctx.direction)),
    );

    check_env_permissions(".env")?;

    match ctx.direction {
        SyncDirection::Forward => sync_forward(
            ctx.placeholder,
            ctx.verbose,
            ctx.dry_run,
            ctx.force,
            &config,
            ctx.naming_policy,
        ),
        SyncDirection::Reverse => sync_reverse(
            ctx.placeholder,
            ctx.verbose,
            ctx.dry_run,
            ctx.force,
            &config,
            ctx.naming_policy,
        ),
    }
}

// ─────────────────────────────────────────────────────────────
// Forward Sync: .env → .env.example
// ─────────────────────────────────────────────────────────────

fn sync_forward(
    use_placeholders: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    config: &PlaceholderConfig,
    naming_policy: NamingPolicy,
) -> Result<()> {
    let parser = Parser::default();

    if !Path::new(".env").exists() {
        ui::error("File not found: .env");
        ui::print_box(
            "💡 Getting Started",
            "It looks like this project hasn't been initialized yet.\n\n\
             To create .env and .env.example files, run:\n\n\
             $ evnx init\n\n\
             Or if you have an existing .env file, place it in this directory \
             and try again.",
        );
        ui::print_next_steps(&[
            "Run 'evnx init' to set up a new project",
            "Or copy your existing .env file to this directory",
            "Then run 'evnx sync' again",
        ]);
        anyhow::bail!(".env file not found - run 'evnx init' to get started");
    }

    let env_file = parser.parse_file(".env").context("Failed to parse .env")?;
    let example_file = match parser.parse_file(".env.example") {
        Ok(f) => f,
        Err(_) => {
            ui::info(".env.example not found, creating from .env");
            return handle_new_example_file(&env_file.vars, use_placeholders, dry_run, config);
        }
    };

    let env_keys: HashSet<_> = env_file.vars.keys().collect();
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let missing: Vec<_> = env_keys.difference(&example_keys).cloned().collect();

    if missing.is_empty() {
        ui::success(".env.example is up to date");
        return Ok(());
    }

    // Validate naming conventions
    let mut warnings = Vec::new();
    for key in &missing {
        if let Err(msg) = validate_var_name(key, naming_policy) {
            if naming_policy == NamingPolicy::Error {
                ui::error(&msg);
                std::process::exit(1);
            }
            warnings.push(msg);
        }
    }

    ui::print_box(
        "Variables to Add",
        &format!(
            "Found {} variable(s) in .env missing from .env.example:\n{}",
            missing.len(),
            missing
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    for warning in &warnings {
        ui::warning(warning);
    }

    if force {
        ui::info("Force mode: using placeholder values and skipping prompts");
        let preview = add_with_placeholders_preview(&missing, &env_file.vars, config)?;
        if dry_run {
            print_preview(&SyncPreview {
                target_file: ".env.example".into(),
                action: SyncAction::Add,
                variables: preview,
                warnings: vec![],
            });
            return Ok(());
        }
        add_with_placeholders(&missing, &env_file.vars, config)?;
        ui::success(format!("Added {} variables to .env.example", missing.len()));
        return Ok(());
    }

    // Interactive mode
    let choices = vec![
        "Yes, add with placeholder values (recommended)",
        "Yes, add with actual values ⚠️ SECURITY RISK",
        "Let me choose individually",
        "No, skip",
    ];

    let selection = Select::new()
        .with_prompt("Add these to .env.example?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            let preview = add_with_placeholders_preview(&missing, &env_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: ".env.example".into(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(());
            }
            add_with_placeholders(&missing, &env_file.vars, config)?;
        }
        1 => {
            ui::print_box(
                "⚠️  SECURITY WARNING",
                "Adding actual values to .env.example may expose secrets if \
                 this file is committed to version control.\n\n\
                 Only proceed if you are certain .env.example is gitignored \
                 or contains no sensitive data.",
            );

            let confirm = Confirm::new()
                .with_prompt("Are you absolutely sure you want to add ACTUAL VALUES?")
                .default(false)
                .interact()?;

            if !confirm {
                ui::info("Aborted: values not added");
                return Ok(());
            }

            log_security_event(
                &missing.iter().map(|k| (*k).clone()).collect::<Vec<_>>(),
                verbose,
            );

            let preview = add_with_actual_values_preview(&missing, &env_file.vars)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: ".env.example".into(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec!["⚠️ Actual values would be written (security risk)".into()],
                });
                return Ok(());
            }
            add_with_actual_values(&missing, &env_file.vars)?;
        }
        2 => {
            let preview = add_interactively_preview(&missing, &env_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: ".env.example".into(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(());
            }
            add_interactively(&missing, &env_file.vars, config)?;
        }
        3 => {
            ui::info("No changes made");
            return Ok(());
        }
        _ => unreachable!(),
    }

    if !dry_run
        && Confirm::new()
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
        atomic_write(".env.example", &content)?;
    }

    ui::success(format!(
        "Updated .env.example (+{} variables)",
        missing.len()
    ));

    ui::print_next_steps(&[
        "Review .env.example to ensure no secrets were added",
        "Commit .env.example to Git (never commit .env)",
        "Share changes with your team",
    ]);

    Ok(())
}

fn handle_new_example_file(
    vars: &IndexMap<String, String>,
    use_placeholders: bool,
    dry_run: bool,
    config: &PlaceholderConfig,
) -> Result<()> {
    let preview = if use_placeholders {
        convert_to_example_preview(vars, config)?
    } else {
        vars.iter()
            .map(|(k, v)| VarChange {
                key: k.clone(),
                old_value: None,
                new_value: v.clone(),
                is_placeholder: false,
            })
            .collect()
    };

    if dry_run {
        print_preview(&SyncPreview {
            target_file: ".env.example".into(),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["New file would be created".into()],
        });
        return Ok(());
    }

    if use_placeholders {
        convert_to_example(vars, config)?;
    } else {
        atomic_write(".env.example", &fs::read_to_string(".env")?)?;
    }

    ui::success("Created .env.example");
    ui::print_next_steps(&[
        "Review .env.example to ensure placeholders are appropriate",
        "Commit .env.example to Git (never commit .env)",
    ]);
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Reverse Sync: .env.example → .env
// ─────────────────────────────────────────────────────────────

fn sync_reverse(
    _use_placeholders: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    config: &PlaceholderConfig,
    naming_policy: NamingPolicy,
) -> Result<()> {
    if verbose {
        eprintln!("[DEBUG] Running reverse sync");
    }
    let parser = Parser::default();

    if !Path::new(".env.example").exists() {
        ui::error("File not found: .env.example");
        ui::print_box(
            "💡 Getting Started",
            "The template file .env.example is missing.\n\n\
             To create it, either:\n\n\
             1. Run 'evnx init' to generate from a blueprint, OR\n\
             2. Create .env.example manually with your variable names\n\n\
             Example .env.example:\n\
             DATABASE_URL=your_database_url\n\
             API_KEY=your_api_key",
        );
        ui::print_next_steps(&[
            "Run 'evnx init' to generate .env.example from a template",
            "Or create .env.example manually with your variable names",
            "Then run 'evnx sync --direction reverse' again",
        ]);
        anyhow::bail!(".env.example not found - run 'evnx init' or create it manually");
    }

    let example_file = parser
        .parse_file(".env.example")
        .context("Failed to parse .env.example")?;

    let env_file = match parser.parse_file(".env") {
        Ok(f) => f,
        Err(_) => {
            ui::info(".env not found, creating from .env.example");
            return handle_new_env_file(&example_file.vars, dry_run, config);
        }
    };

    let env_keys: HashSet<_> = env_file.vars.keys().collect();
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let missing: Vec<_> = example_keys.difference(&env_keys).cloned().collect();

    if missing.is_empty() {
        ui::success(".env is up to date");
        return Ok(());
    }

    for key in &missing {
        if let Err(msg) = validate_var_name(key, naming_policy) {
            if naming_policy == NamingPolicy::Error {
                ui::error(&msg);
                std::process::exit(1);
            }
            ui::warning(&msg);
        }
    }

    ui::print_box(
        "Missing Variables",
        &format!(
            "Found {} variable(s) in .env.example missing from .env:\n{}",
            missing.len(),
            missing
                .iter()
                .map(|k| format!("  • {}", key_highlight(k)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    if force {
        ui::info("Force mode: adding with placeholder values, skipping prompts");
        let preview = add_from_example_preview(&missing, &example_file.vars, true)?;
        if dry_run {
            print_preview(&SyncPreview {
                target_file: ".env".into(),
                action: SyncAction::Add,
                variables: preview,
                warnings: vec!["Remember to replace placeholders with real values".into()],
            });
            return Ok(());
        }
        add_from_example(&missing, &example_file.vars, true)?;
        ui::success(format!("Added {} variables to .env", missing.len()));
        return Ok(());
    }

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

    let use_placeholders = match selection {
        0 => true,
        1 => false,
        2 => {
            let preview =
                add_from_example_interactive_preview(&missing, &example_file.vars, config)?;
            if dry_run {
                print_preview(&SyncPreview {
                    target_file: ".env".into(),
                    action: SyncAction::Add,
                    variables: preview,
                    warnings: vec![],
                });
                return Ok(());
            }
            add_from_example_interactive(&missing, &example_file.vars, config)?;
            ui::success("Added selected variables to .env");
            return Ok(());
        }
        3 => {
            ui::info("No changes made");
            return Ok(());
        }
        _ => unreachable!(),
    };

    let preview = add_from_example_preview(&missing, &example_file.vars, use_placeholders)?;
    if dry_run {
        print_preview(&SyncPreview {
            target_file: ".env".into(),
            action: SyncAction::Add,
            variables: preview,
            warnings: if use_placeholders {
                vec!["Remember to replace placeholders with real values".into()]
            } else {
                vec![]
            },
        });
        return Ok(());
    }

    add_from_example(&missing, &example_file.vars, use_placeholders)?;
    ui::success(format!("Added {} variables to .env", missing.len()));

    if use_placeholders {
        ui::warning("Remember to replace placeholder values with real credentials!");
    }

    Ok(())
}

fn handle_new_env_file(
    example_vars: &IndexMap<String, String>,
    dry_run: bool,
    config: &PlaceholderConfig,
) -> Result<()> {
    if dry_run {
        let preview: Vec<VarChange> = example_vars
            .iter()
            .map(|(k, v)| VarChange {
                key: k.clone(),
                old_value: None,
                new_value: v.clone(),
                is_placeholder: is_placeholder_value(v, config),
            })
            .collect();
        print_preview(&SyncPreview {
            target_file: ".env".into(),
            action: SyncAction::Add,
            variables: preview,
            warnings: vec!["Replace placeholder values with real credentials!".into()],
        });
        return Ok(());
    }

    atomic_write(".env", &fs::read_to_string(".env.example")?)?;
    ui::success("Created .env");
    ui::warning("Replace placeholder values with real credentials!");
    ui::print_next_steps(&[
        "Edit .env and replace placeholder values with real credentials",
        "Run 'chmod 600 .env' to secure the file",
        "Never commit .env to version control",
    ]);
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Add Functions (Forward Sync Helpers)
// ─────────────────────────────────────────────────────────────

fn add_with_placeholders(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let mut content = fs::read_to_string(".env.example").context("Failed to read .env.example")?;
    content.push_str("\n# Synced from .env\n");

    for key in keys {
        let placeholder = generate_placeholder(key, values.get(*key), config);
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    atomic_write(".env.example", &content)?;
    Ok(())
}

fn add_with_placeholders_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let placeholder = generate_placeholder(key, values.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

fn add_with_actual_values(keys: &[&String], values: &IndexMap<String, String>) -> Result<()> {
    let mut content = fs::read_to_string(".env.example")?;
    content.push_str("\n# Synced from .env [⚠️ ACTUAL VALUES]\n");

    for key in keys {
        if let Some(value) = values.get(*key) {
            content.push_str(&format!("{}={}\n", key, value));
        }
    }

    atomic_write(".env.example", &content)?;
    Ok(())
}

fn add_with_actual_values_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .filter_map(|key| {
            values.get(*key).map(|v| VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: v.clone(),
                is_placeholder: false,
            })
        })
        .collect())
}

fn add_interactively(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;

    let mut content = fs::read_to_string(".env.example")?;
    content.push_str("\n# Synced from .env\n");

    for &idx in &selected {
        let key = keys[idx];
        let placeholder = generate_placeholder(key, values.get(key), config);
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    atomic_write(".env.example", &content)?;
    Ok(())
}

fn add_interactively_preview(
    keys: &[&String],
    values: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let placeholder = generate_placeholder(key, values.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

fn convert_to_example(vars: &IndexMap<String, String>, config: &PlaceholderConfig) -> Result<()> {
    let mut content = String::new();
    content.push_str("# Generated from .env\n");
    content.push_str("# Replace all placeholder values with real credentials\n\n");

    for (key, value) in vars {
        let placeholder = generate_placeholder(key, Some(value), config);
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    atomic_write(".env.example", &content)?;
    Ok(())
}

fn convert_to_example_preview(
    vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(vars
        .iter()
        .map(|(k, v)| {
            let placeholder = generate_placeholder(k, Some(v), config);
            VarChange {
                key: k.clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────
// Reverse Sync Add Functions
// ─────────────────────────────────────────────────────────────

fn add_from_example(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    use_placeholders: bool,
) -> Result<()> {
    let mut content = fs::read_to_string(".env").unwrap_or_default();
    content.push_str("\n# Synced from .env.example\n");

    for key in keys {
        let value = if use_placeholders {
            generate_placeholder(key, example_vars.get(*key), &PlaceholderConfig::default())
        } else {
            let default = example_vars.get(*key).map(|s| s.as_str()).unwrap_or("");
            Input::new()
                .with_prompt(format!("Value for {}", key))
                .default(default.to_string())
                .interact_text()?
        };
        content.push_str(&format!("{}={}\n", key, value));
    }

    atomic_write(".env", &content)?;
    Ok(())
}

fn add_from_example_preview(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    use_placeholders: bool,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let example_val = example_vars.get(*key).cloned().unwrap_or_default();
            let new_val = if use_placeholders {
                generate_placeholder(key, Some(&example_val), &PlaceholderConfig::default())
            } else {
                example_val.clone()
            };
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: new_val,
                is_placeholder: use_placeholders,
            }
        })
        .collect())
}

fn add_from_example_interactive(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Select variables to add")
        .items(keys)
        .interact()?;

    let mut content = fs::read_to_string(".env").unwrap_or_default();
    content.push_str("\n# Synced from .env.example\n");

    for &idx in &selected {
        let key = keys[idx];
        let placeholder = generate_placeholder(key, example_vars.get(key), config);
        content.push_str(&format!("{}={}\n", key, placeholder));
    }

    atomic_write(".env", &content)?;
    Ok(())
}

fn add_from_example_interactive_preview(
    keys: &[&String],
    example_vars: &IndexMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let placeholder = generate_placeholder(key, example_vars.get(*key), config);
            VarChange {
                key: (*key).clone(),
                old_value: None,
                new_value: placeholder,
                is_placeholder: true,
            }
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────
// Preview Output Helpers
// ─────────────────────────────────────────────────────────────

fn print_preview(preview: &SyncPreview) {
    ui::print_preview_header();

    println!(
        "{}\n",
        format!(
            "Target: {} (action: {:?})",
            preview.target_file, preview.action
        )
        .dimmed()
    );

    if preview.variables.is_empty() {
        println!("{}", "  No changes".dimmed());
    } else {
        for var in &preview.variables {
            let marker = if var.is_placeholder { "🔒" } else { "⚠️" };
            println!(
                "  {} {} = {}",
                marker,
                key_highlight(&var.key),
                value_preview(&var.new_value, var.is_placeholder)
            );
        }
    }

    if !preview.warnings.is_empty() {
        println!();
        for warning in &preview.warnings {
            ui::warning(warning);
        }
    }

    println!("\n{}", "(dry-run: no files were modified)".dimmed());
}

fn key_highlight(key: &str) -> ColoredString {
    key.cyan()
}

fn value_preview(value: &str, is_placeholder: bool) -> ColoredString {
    if is_placeholder {
        value.dimmed()
    } else {
        value.yellow()
    }
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_sets_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        atomic_write(&env_path, "TEST=value\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&env_path).unwrap().permissions().mode();
            assert_eq!(perms & 0o777, 0o600, "File should have 0o600 permissions");
        }
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("subdir").join(".env");

        // Should not panic even if subdir doesn't exist
        let result = atomic_write(&nested_path, "TEST=value\n");
        // This will fail because parent doesn't exist, but shouldn't panic
        assert!(result.is_err());
    }

    #[test]
    fn test_atomic_write_ensures_env_file_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        atomic_write(&env_path, "TEST=value\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&env_path).unwrap().permissions().mode();
            // This is the guarantee: .env files MUST have 0o600
            assert_eq!(
                perms & 0o777,
                0o600,
                ".env files must have restrictive permissions"
            );
        }
    }

    // Preview helper tests
    #[test]
    fn test_key_highlight_returns_colored() {
        // Just ensure it doesn't panic and returns a ColoredString
        let result = key_highlight("TEST_KEY");
        assert!(!format!("{}", result).is_empty());
    }

    #[test]
    fn test_value_preview_placeholder_vs_actual() {
        let placeholder = value_preview("YOUR_VALUE_HERE", true);
        let actual = value_preview("secret123", false);

        // Both should render, but with different styling (can't easily test colors in unit test)
        assert!(!format!("{}", placeholder).is_empty());
        assert!(!format!("{}", actual).is_empty());
    }

    #[test]
    fn test_add_with_placeholders_preview() {
        let mut values = IndexMap::new();
        values.insert("API_KEY".to_string(), "sk_live_123".to_string());
        values.insert("PORT".to_string(), "3000".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["API_KEY".to_string(), "PORT".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let config = PlaceholderConfig::default();
        let preview = add_with_placeholders_preview(&keys, &values, &config).unwrap();

        assert_eq!(preview.len(), 2);
        assert!(preview.iter().all(|v| v.is_placeholder));
        assert!(preview
            .iter()
            .any(|v| v.key == "API_KEY" && v.new_value == "YOUR_KEY_HERE"));
        assert!(preview
            .iter()
            .any(|v| v.key == "PORT" && v.new_value == "8000"));
    }

    #[test]
    fn test_add_with_actual_values_preview() {
        let mut values = IndexMap::new();
        values.insert("API_KEY".to_string(), "sk_live_123".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["API_KEY".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview = add_with_actual_values_preview(&keys, &values).unwrap();

        assert_eq!(preview.len(), 1);
        assert!(!preview[0].is_placeholder);
        assert_eq!(preview[0].new_value, "sk_live_123");
    }

    // NEW: Integration-style tests with temp files
    #[test]
    fn test_convert_to_example_preview() {
        let mut vars = IndexMap::new();
        // ✅ FIXED: Use "postgresql://" to match the built-in rule
        vars.insert(
            "DB_URL".to_string(),
            "postgresql://user:pass@localhost/db".to_string(),
        );
        vars.insert("API_KEY".to_string(), "secret".to_string());

        let config = PlaceholderConfig::default();
        let preview = convert_to_example_preview(&vars, &config).unwrap();

        assert_eq!(preview.len(), 2);
        assert!(preview.iter().all(|v| v.is_placeholder));

        // ✅ FIXED: Check for the exact expected placeholder
        assert!(preview.iter().any(|v| v.key == "DB_URL"
            && v.new_value == "postgresql://user:password@localhost:5432/dbname"));
    }

    #[test]
    fn test_add_from_example_preview_with_placeholders() {
        let mut example_vars = IndexMap::new();
        example_vars.insert("NEW_VAR".to_string(), "placeholder_value".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["NEW_VAR".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview = add_from_example_preview(&keys, &example_vars, true).unwrap();

        assert_eq!(preview.len(), 1);
        assert!(preview[0].is_placeholder);
        assert_eq!(preview[0].new_value, "YOUR_VALUE_HERE");
    }

    #[test]
    fn test_add_from_example_preview_without_placeholders() {
        let mut example_vars = IndexMap::new();
        example_vars.insert("NEW_VAR".to_string(), "example_default".to_string());

        // ✅ FIXED: Create Vec<&String> properly
        let keys_vec: Vec<String> = vec!["NEW_VAR".to_string()];
        let keys: Vec<&String> = keys_vec.iter().collect();

        let preview = add_from_example_preview(&keys, &example_vars, false).unwrap();

        assert_eq!(preview.len(), 1);
        assert!(!preview[0].is_placeholder);
        assert_eq!(preview[0].new_value, "example_default");
    }
}
