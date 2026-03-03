//! Sync command — keep `.env` and `.env.example` in sync.
//!
//! # Purpose
//!
//! This command helps maintain consistency between local environment files
//! (`.env`) and team template files (`.env.example`). It supports bidirectional
//! synchronization with smart placeholder generation and safety features.
//!
//! # Safety Features
//!
//! - **Default behavior uses placeholders**: Never accidentally commit secrets
//! - **Atomic file writes**: Prevents corruption from interrupted operations
//! - **Permission checks**: Warns if `.env` has overly permissive file modes
//! - **Dry-run mode**: Preview changes before applying them
//! - **CI-friendly flags**: `--force` skips prompts for automation
//!
//! # ⚠️ Security Warning
//!
//! The "add with actual values" option in forward sync can expose secrets
//! if `.env.example` is committed to version control. This option:
//!
//! 1. Shows an explicit red warning prompt before proceeding
//! 2. Is automatically disabled when `--force` is used in CI environments
//! 3. Logs a security audit message when selected
//!
//! Always prefer placeholder values for template files.
//!
//! # Usage Examples
//!
//! ```bash
//! # Basic forward sync (add new .env vars to .env.example with placeholders)
//! evnx sync --direction forward
//!
//! # Reverse sync with interactive value prompts
//! evnx sync --direction reverse
//!
//! # Preview changes without writing (dry-run)
//! evnx sync --dry-run
//!
//! # CI/CD usage: skip prompts, use placeholders
//! evnx sync --force --placeholder
//!
//! # Custom placeholder templates
//! evnx sync --template-config ./placeholders.json
//! ```
//!
//! # File Permissions
//!
//! The `.env` file should have restrictive permissions (`0o600` or `rw-------`).
//! This command will warn if permissions are too permissive, as loose permissions
//! could allow other users on the system to read sensitive credentials.

use anyhow::{Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::cli::{NamingPolicy, SyncDirection};
use crate::core::Parser;
use crate::utils::ui;

/// Configuration for custom placeholder templates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderConfig {
    /// Pattern-to-placeholder mappings (regex pattern → placeholder string)
    #[serde(default)]
    pub patterns: HashMap<String, String>,

    /// Default placeholder for unmatched keys
    #[serde(default = "default_placeholder")]
    pub default: String,

    /// Keys that should always use actual values (use with extreme caution)
    #[serde(default)]
    pub allow_actual: Vec<String>,
}

fn default_placeholder() -> String {
    String::from("YOUR_VALUE_HERE")
}

impl PlaceholderConfig {
    /// Load configuration from a JSON file
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).context("Failed to read placeholder config file")?;
        serde_json::from_str(&content).context("Failed to parse placeholder config as JSON")
    }
}

impl Default for PlaceholderConfig {
    fn default() -> Self {
        Self {
            patterns: HashMap::new(),
            default: String::from("YOUR_VALUE_HERE"),
            allow_actual: Vec::new(),
        }
    }
}

/// Result of a sync operation (for dry-run preview)
#[derive(Debug, Clone)]
pub struct SyncPreview {
    pub target_file: String,
    pub action: SyncAction,
    pub variables: Vec<VarChange>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncAction {
    Add,
    Update,
    Remove,
    NoChange,
}

#[derive(Debug, Clone)]
pub struct VarChange {
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: String,
    pub is_placeholder: bool,
}

/// Main entry point for the sync command
pub fn run(
    direction: SyncDirection,
    placeholder: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    template_config: Option<PathBuf>,
    naming_policy: NamingPolicy,
) -> Result<()> {
    if verbose {
        println!(
            "{}",
            format!("Running sync in verbose mode [direction={direction}, dry_run={dry_run}, force={force}]").dimmed()
        );
    }

    // Load custom placeholder config if provided
    let config = template_config
        .as_ref()
        .map(PlaceholderConfig::from_path)
        .transpose()?
        .unwrap_or_default();

    ui::print_header(
        "Sync .env ↔ .env.example",
        Some(&format!("Direction: {}", direction)),
    );

    // Check file permissions before proceeding
    check_env_permissions(".env")?;

    match direction {
        SyncDirection::Forward => {
            sync_forward(placeholder, verbose, dry_run, force, &config, naming_policy)
        }
        SyncDirection::Reverse => {
            sync_reverse(placeholder, verbose, dry_run, force, &config, naming_policy)
        }
    }
}

/// Check if .env file has appropriate permissions (0o600 recommended)
fn check_env_permissions(path: &str) -> Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()), // File doesn't exist yet, skip check
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = metadata.permissions().mode();
        // Check if group or others have any permissions
        if perms & 0o077 != 0 {
            ui::warning(format!(
                "{} has permissive permissions (0o{:o}). Consider: chmod 600 {}",
                path,
                perms & 0o777,
                path
            ));
            ui::info("Overly permissive .env files may expose secrets to other users");
        }
    }

    // On Windows, we could check ACLs, but that's complex; skip for now
    Ok(())
}

/// Validate environment variable naming convention
fn validate_var_name(key: &str, policy: NamingPolicy) -> Result<(), String> {
    // Standard convention: UPPERCASE with underscores, optional prefix
    // Examples: DATABASE_URL, API_SECRET_KEY, MY_APP_DEBUG
    let is_standard = regex::Regex::new(r"^[A-Z][A-Z0-9_]*$")
        .ok()
        .map(|re| re.is_match(key))
        .unwrap_or(true); // If regex fails, be permissive

    if !is_standard {
        match policy {
            NamingPolicy::Error => {
                return Err(format!(
                    "Non-standard variable name '{}'. Expected: UPPERCASE_WITH_UNDERSCORES",
                    key
                ));
            }
            NamingPolicy::Warn => {
                ui::warning(format!(
                    "Variable '{}' doesn't follow convention (UPPERCASE_WITH_UNDERSCORES)",
                    key
                ));
            }
            NamingPolicy::Ignore => {} // Silent
        }
    }
    Ok(())
}

/// Atomic write: write to temp file, then rename to target
fn atomic_write<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    let path = path.as_ref();
    let temp = NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;

    // Write content to temp file
    temp.as_file().write_all(content.as_bytes())?;
    temp.as_file().sync_all()?; // Ensure data is on disk

    // Atomic rename
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

// /// Check if a file exists and provide friendly error if not
// fn require_file(path: &str, suggestion: &str) -> Result<()> {
//     if !Path::new(path).exists() {
//         ui::error(&format!("File not found: {}", path));
//         ui::info(suggestion);
//         anyhow::bail!("Missing required file: {}", path);
//     }
//     Ok(())
// }

/// Forward sync: .env → .env.example
#[allow(clippy::too_many_arguments)]
fn sync_forward(
    use_placeholders: bool,
    verbose: bool,
    dry_run: bool,
    force: bool,
    config: &PlaceholderConfig,
    naming_policy: NamingPolicy,
) -> Result<()> {
    let parser = Parser::default();

    // ✅ Check if .env exists with friendly message
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

            let preview = if use_placeholders {
                convert_to_example_preview(&env_file.vars, config)?
            } else {
                // Direct copy preview
                env_file
                    .vars
                    .iter()
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
                convert_to_example(&env_file.vars, config)?;
            } else {
                atomic_write(".env.example", &fs::read_to_string(".env")?)?;
            }

            ui::success("Created .env.example");
            ui::print_next_steps(&[
                "Review .env.example to ensure placeholders are appropriate",
                "Commit .env.example to Git (never commit .env)",
            ]);
            return Ok(());
        }
    };

    // Find variables in .env but not in .env.example
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

    // Handle CI/CD mode (force)
    if force {
        // In force mode, always use placeholders for safety
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
            // Placeholder values (safe default)
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
            // ⚠️ Actual values — show prominent warning
            ui::print_box(
                "⚠️  SECURITY WARNING",
                "Adding actual values to .env.example may expose secrets if \
                 this file is committed to version control.\n\n\
                 Only proceed if you are certain .env.example is gitignored \
                 or contains no sensitive data.",
            );

            // Require explicit confirmation
            let confirm = Confirm::new()
                .with_prompt("Are you absolutely sure you want to add ACTUAL VALUES?")
                .default(false)
                .interact()?;

            if !confirm {
                ui::info("Aborted: values not added");
                return Ok(());
            }

            // Log security event (could send to audit log in production)
            if verbose {
                eprintln!(
                    "[AUDIT] User added actual values to .env.example for keys: {:?}",
                    missing
                );
            }

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
            // Interactive selection
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

    // Skip comment prompt in dry-run
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

/// Reverse sync: .env.example → .env
#[allow(clippy::too_many_arguments)]
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

    // ✅ Check if .env.example exists with friendly message
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

            if dry_run {
                let preview: Vec<VarChange> = example_file
                    .vars
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
            return Ok(());
        }
    };

    // Find variables in .env.example but not in .env
    let env_keys: HashSet<_> = env_file.vars.keys().collect();
    let example_keys: HashSet<_> = example_file.vars.keys().collect();
    let missing: Vec<_> = example_keys.difference(&env_keys).cloned().collect();

    if missing.is_empty() {
        ui::success(".env is up to date");
        return Ok(());
    }

    // Validate naming conventions
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

    // Handle CI/CD mode
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

    // Interactive mode
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
            // Individual selection preview
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

// ─────────────────────────────────────────────────────────────
// Placeholder Generation (with custom config support)
// ─────────────────────────────────────────────────────────────

/// Generate placeholder using built-in rules + custom config
fn generate_placeholder(key: &str, value: Option<&String>, config: &PlaceholderConfig) -> String {
    // First check custom patterns from config
    for (pattern, placeholder) in &config.patterns {
        // ✅ FIXED: Proper regex matching with case-insensitive flag
        if let Ok(re) = regex::Regex::new(&format!("(?i){}", pattern)) {
            if re.is_match(key) {
                return placeholder.clone();
            }
        }
    }

    // Built-in heuristics (fallback)
    let key_upper = key.to_uppercase();

    if key_upper.contains("SECRET") || key_upper.contains("KEY") || key_upper.contains("TOKEN") {
        return "YOUR_KEY_HERE".to_string();
    }

    if key_upper.contains("PASSWORD") || key_upper.contains("PASS") {
        return "YOUR_PASSWORD_HERE".to_string();
    }

    if key_upper.contains("URL") {
        if let Some(v) = value {
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
        return "true".to_string();
    }

    if key_upper.contains("HOST") || key_upper.contains("SERVER") {
        return "localhost".to_string();
    }

    // ✅ FIXED: Use config default, fallback to hardcoded default if empty
    if config.default.is_empty() {
        "YOUR_VALUE_HERE".to_string()
    } else {
        config.default.clone()
    }
}

/// Check if a value looks like a placeholder
fn is_placeholder_value(value: &str, config: &PlaceholderConfig) -> bool {
    value == config.default
        || value.contains("YOUR_")
        || value.contains("placeholder")
        || value == "localhost"
        || value == "8000"
        || value == "true"
        || value == "false"
}

// ─────────────────────────────────────────────────────────────
// Add Functions (with preview support)
// ─────────────────────────────────────────────────────────────

fn add_with_placeholders(
    keys: &[&String],
    values: &HashMap<String, String>,
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
    values: &HashMap<String, String>,
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

fn add_with_actual_values(keys: &[&String], values: &HashMap<String, String>) -> Result<()> {
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
    values: &HashMap<String, String>,
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
    values: &HashMap<String, String>,
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
    values: &HashMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    // For preview, assume all would be selected with placeholders
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

fn convert_to_example(vars: &HashMap<String, String>, config: &PlaceholderConfig) -> Result<()> {
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
    vars: &HashMap<String, String>,
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

// Reverse sync add functions
fn add_from_example(
    keys: &[&String],
    example_vars: &HashMap<String, String>,
    use_placeholders: bool,
) -> Result<()> {
    let mut content = fs::read_to_string(".env").unwrap_or_default();
    content.push_str("\n# Synced from .env.example\n");

    for key in keys {
        let value = if use_placeholders {
            // Generate safe placeholder
            generate_placeholder(key, example_vars.get(*key), &PlaceholderConfig::default())
        } else {
            // Prompt for real value
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
    example_vars: &HashMap<String, String>,
    use_placeholders: bool,
) -> Result<Vec<VarChange>> {
    Ok(keys
        .iter()
        .map(|key| {
            let example_val = example_vars.get(*key).cloned().unwrap_or_default();
            let new_val = if use_placeholders {
                generate_placeholder(key, Some(&example_val), &PlaceholderConfig::default())
            } else {
                example_val.clone() // In preview, show what would be prompted
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
    example_vars: &HashMap<String, String>,
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
    example_vars: &HashMap<String, String>,
    config: &PlaceholderConfig,
) -> Result<Vec<VarChange>> {
    // Preview assumes all selected with placeholders
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
// Preview Output
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

// src/commands/sync.rs - tests module

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_placeholder_builtin_rules() {
        let config = PlaceholderConfig::default();

        assert_eq!(
            generate_placeholder("SECRET_KEY", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("API_TOKEN", None, &config),
            "YOUR_KEY_HERE"
        );
        assert_eq!(
            generate_placeholder("DB_PASSWORD", None, &config),
            "YOUR_PASSWORD_HERE"
        );
        assert_eq!(generate_placeholder("PORT", None, &config), "8000");
        assert_eq!(generate_placeholder("DEBUG_MODE", None, &config), "true");
        assert_eq!(generate_placeholder("DB_HOST", None, &config), "localhost");
        // ✅ FIXED: This should now return "YOUR_VALUE_HERE"
        assert_eq!(
            generate_placeholder("RANDOM_VAR", None, &config),
            "YOUR_VALUE_HERE"
        );
    }

    #[test]
    fn test_generate_placeholder_with_url_values() {
        let config = PlaceholderConfig::default();

        let pg_url = "postgresql://user:pass@localhost:5432/db";
        assert!(
            generate_placeholder("DATABASE_URL", Some(&pg_url.to_string()), &config)
                .contains("postgresql")
        );

        let redis_url = "redis://localhost:6379/0";
        assert!(
            generate_placeholder("REDIS_URL", Some(&redis_url.to_string()), &config)
                .contains("redis")
        );
    }

    #[test]
    fn test_generate_placeholder_custom_config() {
        let mut config = PlaceholderConfig::default();
        // ✅ FIXED: Use proper regex pattern (without .* suffix in pattern itself)
        config
            .patterns
            .insert("AWS_.*".to_string(), "aws-placeholder".to_string());
        config.default = "CUSTOM_DEFAULT".to_string();

        // ✅ This should now match the custom pattern
        assert_eq!(
            generate_placeholder("AWS_SECRET", None, &config),
            "aws-placeholder"
        );
        // ✅ This should use custom default
        assert_eq!(
            generate_placeholder("UNKNOWN_VAR", None, &config),
            "CUSTOM_DEFAULT"
        );
    }

    #[test]
    fn test_validate_var_name() {
        assert!(validate_var_name("DATABASE_URL", NamingPolicy::Warn).is_ok());
        assert!(validate_var_name("API_KEY_123", NamingPolicy::Warn).is_ok());

        // Non-standard names
        assert!(validate_var_name("camelCase", NamingPolicy::Ignore).is_ok());
        assert!(validate_var_name("camelCase", NamingPolicy::Warn).is_ok()); // warns but ok
        assert!(validate_var_name("camelCase", NamingPolicy::Error).is_err());
    }

    #[test]
    fn test_is_placeholder_value() {
        let config = PlaceholderConfig::default();

        assert!(is_placeholder_value("YOUR_VALUE_HERE", &config));
        assert!(is_placeholder_value("YOUR_KEY_HERE", &config));
        assert!(is_placeholder_value("localhost", &config));
        assert!(!is_placeholder_value("sk_live_abc123", &config));
        assert!(!is_placeholder_value("production-db.example.com", &config));
    }

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
    fn test_sync_direction_display() {
        assert_eq!(SyncDirection::Forward.to_string(), "forward");
        assert_eq!(SyncDirection::Reverse.to_string(), "reverse");
    }

    #[test]
    fn test_placeholder_config_serialization() {
        let config = PlaceholderConfig {
            patterns: HashMap::from([("API_.*".to_string(), "api-key".to_string())]),
            default: "custom".to_string(),
            allow_actual: vec!["PUBLIC_KEY".to_string()],
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: PlaceholderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.default, "custom");
        assert!(parsed.patterns.contains_key("API_.*"));
        assert_eq!(parsed.allow_actual, vec!["PUBLIC_KEY"]);
    }

    // ✅ NEW: Test that default placeholder is never empty
    #[test]
    fn test_default_placeholder_not_empty() {
        let config = PlaceholderConfig::default();
        assert!(
            !config.default.is_empty(),
            "Default placeholder should not be empty"
        );
        assert_eq!(config.default, "YOUR_VALUE_HERE");
    }

    // ✅ NEW: Test regex pattern matching with various patterns
    #[test]
    fn test_custom_pattern_matching() {
        let mut config = PlaceholderConfig::default();
        config
            .patterns
            .insert("STRIPE_.*".to_string(), "stripe_test_key".to_string());
        config
            .patterns
            .insert(".*_PORT".to_string(), "3000".to_string());

        assert_eq!(
            generate_placeholder("STRIPE_API_KEY", None, &config),
            "stripe_test_key"
        );
        assert_eq!(
            generate_placeholder("STRIPE_SECRET", None, &config),
            "stripe_test_key"
        );
        assert_eq!(generate_placeholder("SERVER_PORT", None, &config), "3000");
        assert_eq!(generate_placeholder("APP_PORT", None, &config), "3000");
    }
}
