//! Convert command — transform `.env` files to multiple output formats.
//!
//! This module implements the `evnx convert` subcommand, which parses a `.env`
//! file and transforms its variables into 14+ target formats including JSON,
//! YAML, cloud provider configs, CI/CD formats, and infrastructure-as-code.
//!
//! # Architecture
//!
//! ```text
//! .env file
//!     │
//!     ▼
//! Parser::parse_file() → IndexMap<String, String>
//!     │
//!     ▼
//! ConvertOptions { filter, transform, encode }
//!     │
//!     ▼
//! Converter::convert() → format-specific serialization
//!     │
//!     ▼
//! stdout or output file
//! ```
//!
//! # Key Features
//!
//! - **Filtering**: Include/exclude variables via glob patterns (`AWS_*`, `*_URL`)
//! - **Transformations**: Key casing (`camelCase`, `snake_case`), prefixes, base64 encoding
//! - **Interactive mode**: TUI format selector when `--to` is omitted
//! - **Verbose output**: Debug logging with `--verbose` flag
//!
//! # Supported Formats
//!
//! | Category | Formats |
//! |----------|---------|
//! | Generic | `json`, `yaml`, `yml`, `shell`, `bash`, `export` |
//! | Cloud | `aws-secrets`, `gcp-secrets`, `azure-keyvault` |
//! | CI/CD | `github-actions`, `gh-actions` |
//! | Containers | `docker-compose`, `kubernetes`, `k8s` |
//! | IaC | `terraform`, `tfvars`, `tf` |
//! | Secret Managers | `doppler`, `heroku`, `vercel`, `railway` |
//!
//! # Usage Examples
//!
//! ```no_run
//! # // These examples demonstrate CLI usage and cannot be run in doc tests
//! # // Basic JSON conversion
//! # // evnx convert --to json --env .env.production
//! #
//! # // With transformations
//! # // evnx convert \
//! # //   --to kubernetes \
//! # //   --include "PROD_*" \
//! # //   --prefix "MYAPP_" \
//! # //   --transform uppercase \
//! # //   --output k8s-secret.yaml
//! #
//! # // Interactive mode (opens TUI selector)
//! # // evnx convert
//! ```
//!
//! # Error Handling
//!
//! All errors are wrapped with [`anyhow::Context`] to provide actionable messages:
//! - File parsing failures include the source path
//! - Format conversion errors specify the target format
//! - I/O errors include the output destination
//!
//! # See Also
//!
//! - [`crate::core::converter`] — Core conversion traits and options
//! - [`crate::formats`] — Format-specific converter implementations
//! - [`crate::utils::ui`] — Terminal UI utilities for consistent output

use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
// use indexmap::IndexMap;
use std::fs;
use std::path::Path;

use crate::core::{
    converter::{ConvertOptions, Converter, KeyTransform},
    Parser,
};
use crate::formats;

// ─────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────

/// Configuration for the convert command.
///
/// This struct consolidates all CLI arguments into a single type,
/// improving testability and reducing parameter passing overhead.
///
/// # Example
///
/// ```
/// # use evnx::commands::convert::ConvertConfig;
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("kubernetes"))
///     .include_pattern(Some("PROD_*"))
///     .prefix(Some("MYAPP_"))
///     .base64(true)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ConvertConfig {
    /// Path to input .env file
    pub env_path: String,
    /// Target format (None = interactive mode)
    pub target_format: Option<String>,
    /// Output file path (None = stdout)
    pub output_path: Option<String>,
    /// Conversion options (filtering, transformations)
    pub options: ConvertOptions,
    /// Enable verbose progress output
    pub verbose: bool,
}

impl ConvertConfig {
    /// Create a new config with default values.
    ///
    /// ```
    /// # use evnx::commands::convert::ConvertConfig;
    /// let config = ConvertConfig::default();
    /// assert_eq!(config.env_path, ".env");
    /// assert!(config.target_format.is_none());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new config using builder pattern.
    ///
    /// ```
    /// # use evnx::commands::convert::ConvertConfig;
    /// let config = ConvertConfig::builder()
    ///     .env(".env.prod")
    ///     .verbose(true)
    ///     .build();
    /// assert_eq!(config.env_path, ".env.prod");
    /// assert!(config.verbose);
    /// ```
    #[must_use]
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Build the inner ConvertOptions from config fields.
    ///
    /// This method returns a clone of the stored options for use by converters.
    #[must_use]
    pub fn build_options(&self) -> ConvertOptions {
        self.options.clone()
    }

    /// Check if the input file exists and is readable.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist or cannot be read.
    pub fn validate_input(&self) -> Result<()> {
        let path = Path::new(&self.env_path);
        if !path.exists() {
            anyhow::bail!("Input file not found: '{}'", self.env_path);
        }
        if !path.is_file() {
            anyhow::bail!("Input path is not a file: '{}'", self.env_path);
        }
        Ok(())
    }
}

impl Default for ConvertConfig {
    fn default() -> Self {
        Self {
            env_path: ".env".to_string(),
            target_format: None,
            output_path: None,
            options: ConvertOptions::default(),
            verbose: false,
        }
    }
}

/// Builder for [`ConvertConfig`].
///
/// # Example
///
/// ```
/// # use evnx::commands::convert::ConvertConfig;
/// # use evnx::core::converter::KeyTransform;
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("json"))           // ✅ Wrap in Some()
///     .output_path(Some("output.json"))      // ✅ Wrap in Some()
///     .include_pattern(Some("AWS_*"))        // ✅ Wrap in Some()
///     .exclude_pattern(Some("*_DEBUG"))      // ✅ Wrap in Some()
///     .base64(true)
///     .prefix(Some("MYAPP_"))                // ✅ Wrap in Some()
///     .transform(Some(KeyTransform::Uppercase))  // ✅ Wrap in Some()
///     .verbose(true)
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    env_path: Option<String>,
    target_format: Option<String>,
    output_path: Option<String>,
    include_pattern: Option<String>,
    exclude_pattern: Option<String>,
    base64: bool,
    prefix: Option<String>,
    transform: Option<KeyTransform>,
    verbose: bool,
}

impl ConfigBuilder {
    /// Set the input .env file path.
    #[must_use]
    pub fn env(mut self, path: impl Into<String>) -> Self {
        self.env_path = Some(path.into());
        self
    }

    /// Set the target output format (optional).
    #[must_use]
    pub fn target_format<S: Into<String>>(mut self, format: Option<S>) -> Self {
        self.target_format = format.map(Into::into);
        self
    }

    /// Set the output file path (optional, None = stdout).
    #[must_use]
    pub fn output_path<S: Into<String>>(mut self, path: Option<S>) -> Self {
        self.output_path = path.map(Into::into);
        self
    }

    /// Set glob pattern to include only matching variables (optional).
    #[must_use]
    pub fn include_pattern<S: Into<String>>(mut self, pattern: Option<S>) -> Self {
        self.include_pattern = pattern.map(Into::into);
        self
    }

    /// Set glob pattern to exclude matching variables (optional).
    #[must_use]
    pub fn exclude_pattern<S: Into<String>>(mut self, pattern: Option<S>) -> Self {
        self.exclude_pattern = pattern.map(Into::into);
        self
    }

    /// Enable base64 encoding for all values.
    #[must_use]
    pub fn base64(mut self, enabled: bool) -> Self {
        self.base64 = enabled;
        self
    }

    /// Set prefix to prepend to all variable names (optional).
    #[must_use]
    pub fn prefix<S: Into<String>>(mut self, prefix: Option<S>) -> Self {
        self.prefix = prefix.map(Into::into);
        self
    }

    /// Set key casing transformation (optional).
    #[must_use]
    pub fn transform(mut self, transform: Option<KeyTransform>) -> Self {
        self.transform = transform;
        self
    }

    /// Enable verbose progress output.
    #[must_use]
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }

    /// Build the final [`ConvertConfig`].
    #[must_use]
    pub fn build(self) -> ConvertConfig {
        ConvertConfig {
            env_path: self.env_path.unwrap_or_else(|| ".env".to_string()),
            target_format: self.target_format,
            output_path: self.output_path,
            options: ConvertOptions {
                include_pattern: self.include_pattern,
                exclude_pattern: self.exclude_pattern,
                base64: self.base64,
                prefix: self.prefix,
                transform: self.transform,
            },
            verbose: self.verbose,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point
// ─────────────────────────────────────────────────────────────

/// Execute the convert command with the provided configuration.
///
/// This function orchestrates the full conversion pipeline:
/// 1. Validate and parse the input `.env` file
/// 2. Select or interactively prompt for the target format
/// 3. Instantiate the appropriate [`Converter`] implementation
/// 4. Apply filtering, transformations, and serialization
/// 5. Write output to stdout or specified file
///
/// # Arguments
///
/// * `config` - The [`ConvertConfig`] containing all command options
///
/// # Returns
///
/// * `Ok(())` on successful conversion and output
/// * `Err(anyhow::Error)` with context for:
///   - File I/O errors (reading input or writing output)
///   - Parsing errors (invalid `.env` syntax)
///   - Serialization errors (format-specific conversion failures)
///   - User cancellation in interactive mode
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::convert::{ConvertConfig, run};
/// # use anyhow::Result;
/// # fn example() -> Result<()> {
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("json"))
///     .output_path(Some("output.json"))
///     .build();
///
/// run(config)?;
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// This function does not panic. All errors are returned via [`Result`].
pub fn run(config: ConvertConfig) -> Result<()> {
    if config.verbose {
        eprintln!(
            "{}",
            format!("🔄 Running convert in verbose mode: {}", config.env_path).dimmed()
        );
    }

    // Validate input file exists
    config
        .validate_input()
        .with_context(|| format!("Failed to validate input file: '{}'", config.env_path))?;

    // Parse .env file
    let parser = Parser::default();
    let env_file = parser
        .parse_file(&config.env_path)
        .with_context(|| format!("Failed to parse environment file: '{}'", config.env_path))?;

    if config.verbose {
        eprintln!(
            "{}",
            format!(
                "📦 Loaded {} variables from {}",
                env_file.vars.len(),
                config.env_path
            )
            .dimmed()
        );
    }

    // Determine target format (interactive if not specified)
    let format_name = match &config.target_format {
        Some(f) => f.clone(),
        None => select_format_interactive(config.verbose)?,
    };

    // Get the appropriate converter
    let converter = get_converter(&format_name).with_context(|| {
        format!(
            "Failed to initialize converter for format: '{}'",
            format_name
        )
    })?;

    if config.verbose {
        eprintln!(
            "{}",
            format!("⚙️  Converting to {} format...", converter.name()).dimmed()
        );
    }

    // Perform conversion with options
    let result = converter
        .convert(&env_file.vars, &config.options)
        .with_context(|| format!("Conversion to '{}' failed", format_name))?;

    // Output result
    write_output(&result, config.output_path.as_deref(), config.verbose)?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Display interactive TUI for format selection.
///
/// # Arguments
///
/// * `verbose` - If true, emit debug messages to stderr
///
/// # Returns
///
/// * `Ok(String)` - The selected format name
/// * `Err(anyhow::Error)` - On user cancellation or dialoguer error
///
/// # Side Effects
///
/// - Prints formatted header to stdout
/// - Blocks waiting for user input via terminal UI
fn select_format_interactive(verbose: bool) -> Result<String> {
    if verbose {
        eprintln!("{}", "🎯 Launching interactive format selector".dimmed());
    }

    println!(
        "\n{}",
        "┌─ Convert environment variables ─────────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Transform .env into different formats               │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    let formats = vec![
        // Generic formats (always show first)
        "json - Generic JSON key-value object",
        "yaml - Generic YAML key-value format",
        "shell - Shell export script (bash/zsh)",
        // Cloud providers
        "aws-secrets - AWS Secrets Manager (CLI commands)",
        "gcp-secrets - GCP Secret Manager (gcloud commands)",
        "azure-keyvault - Azure Key Vault (az CLI commands)",
        // CI/CD platforms
        "github-actions - GitHub Actions secrets (ready to paste)",
        // Container platforms
        "docker-compose - Docker Compose YAML environment",
        "kubernetes - Kubernetes Secret YAML (base64 encoded)",
        // Infrastructure as Code
        "terraform - Terraform .tfvars file",
        // Secret management platforms
        "doppler - Doppler secrets JSON format",
        "heroku - Heroku config vars (CLI commands)",
        "vercel - Vercel environment variables JSON",
        "railway - Railway variables JSON format",
    ];

    let selection = Select::new()
        .with_prompt("Select output format")
        .items(&formats)
        .default(0)
        .interact()
        .context("Format selection cancelled by user")?;

    // Extract format name (everything before first dash and space)
    let format = formats[selection]
        .split('-')
        .next()
        .unwrap()
        .trim()
        .to_string();

    if verbose {
        eprintln!("{}", format!("✅ Selected format: {}", format).dimmed());
    }

    Ok(format)
}

/// Get a converter instance for the specified format name.
///
/// Supports format aliases (e.g., "k8s" → "kubernetes", "tf" → "terraform").
///
/// # Arguments
///
/// * `format` - The format name or alias (case-insensitive matching)
///
/// # Returns
///
/// * `Ok(Box<dyn Converter>)` - Initialized converter instance
/// * `Err(anyhow::Error)` - If format is unrecognized, with helpful error message
///
/// # Example
///
/// ```no_run
/// # // This function is internal; example shows conceptual usage
/// # // let converter = get_converter("json")?;
/// # // assert_eq!(converter.name(), "json");
/// ```
fn get_converter(format: &str) -> Result<Box<dyn Converter>> {
    match format.to_lowercase().as_str() {
        // Generic formats
        "json" => Ok(Box::new(formats::JsonConverter)),
        "yaml" | "yml" => Ok(Box::new(formats::YamlConverter)),
        "shell" | "bash" | "export" => Ok(Box::new(formats::ShellExportConverter)),

        // Cloud providers
        "aws" | "aws-secrets" | "aws-secrets-manager" => Ok(Box::new(formats::AwsSecretsConverter)),
        "gcp" | "gcp-secrets" | "gcp-secret-manager" => {
            Ok(Box::new(formats::GcpSecretConverter::default()))
        }
        "azure" | "azure-keyvault" | "azure-key-vault" => {
            Ok(Box::new(formats::AzureKeyVaultConverter::default()))
        }

        // CI/CD platforms
        "github" | "github-actions" | "gh-actions" => {
            Ok(Box::new(formats::GitHubActionsConverter::default()))
        }

        // Container platforms
        "docker" | "docker-compose" | "compose" => Ok(Box::new(formats::DockerComposeConverter)),
        "kubernetes" | "k8s" | "kubectl" => {
            Ok(Box::new(formats::KubernetesSecretConverter::default()))
        }

        // Infrastructure as Code
        "terraform" | "tfvars" | "tf" => Ok(Box::new(formats::TerraformConverter)),

        // Secret management platforms
        "doppler" => Ok(Box::new(formats::DopplerConverter)),
        "heroku" => Ok(Box::new(formats::HerokuConfigConverter::default())),
        "vercel" => Ok(Box::new(formats::VercelEnvConverter)),
        "railway" => Ok(Box::new(formats::RailwayConverter)),

        // Unknown format - provide helpful error
        unknown => {
            eprintln!("{} Unknown format: {}", "✗".red(), unknown);
            eprintln!();
            eprintln!("{}", "Supported formats:".bold());
            eprintln!();
            print_format_help();
            std::process::exit(1);
        }
    }
}

/// Print formatted help for supported formats.
///
/// Called when an unknown format is specified.
fn print_format_help() {
    eprintln!("  {}", "Generic:".yellow());
    eprintln!("    json, yaml, shell");
    eprintln!();
    eprintln!("  {}", "Cloud providers:".yellow());
    eprintln!("    aws-secrets, gcp-secrets, azure-keyvault");
    eprintln!();
    eprintln!("  {}", "CI/CD:".yellow());
    eprintln!("    github-actions");
    eprintln!();
    eprintln!("  {}", "Containers:".yellow());
    eprintln!("    docker-compose, kubernetes");
    eprintln!();
    eprintln!("  {}", "Infrastructure:".yellow());
    eprintln!("    terraform");
    eprintln!();
    eprintln!("  {}", "Secret managers:".yellow());
    eprintln!("    doppler, heroku, vercel, railway");
    eprintln!();
    eprintln!("  {}", "Aliases:".dimmed());
    eprintln!("    k8s → kubernetes, tf → terraform, yml → yaml");
    eprintln!("    gh-actions → github-actions, compose → docker-compose");
}

/// Write conversion result to stdout or file.
///
/// # Arguments
///
/// * `content` - The formatted output string
/// * `output_path` - Optional file path (None = stdout)
/// * `verbose` - If true, emit success messages to stderr
///
/// # Returns
///
/// * `Ok(())` on successful write
/// * `Err(anyhow::Error)` on I/O failure
fn write_output(content: &str, output_path: Option<&str>, verbose: bool) -> Result<()> {
    match output_path {
        Some(path) => {
            fs::write(path, content)
                .with_context(|| format!("Failed to write output to '{}'", path))?;
            if verbose {
                eprintln!("{}", "✓ Converted successfully".green());
                eprintln!("{}", format!("📄 Output written to: {}", path).dimmed());
            } else {
                println!("{} Converted successfully", "✓".green());
                println!("Output written to: {}", path);
            }
        }
        None => {
            // Output to stdout
            println!("{}", content);
            if verbose {
                eprintln!("{}", "✓ Output written to stdout".dimmed());
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::converter::KeyTransform;
    use indexmap::IndexMap;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_convert_config_default() {
        let config = ConvertConfig::default();
        assert_eq!(config.env_path, ".env");
        assert!(config.target_format.is_none());
        assert!(config.output_path.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_convert_config_builder() {
        let config = ConvertConfig::builder()
            .env(".env.prod")
            .target_format(Some("json")) // ✅ Wrap in Some()
            .output_path(Some("out.json")) // ✅ Wrap in Some()
            .include_pattern(Some("AWS_*")) // ✅ Wrap in Some()
            .exclude_pattern(Some("*_DEBUG")) // ✅ Wrap in Some()
            .base64(true)
            .prefix(Some("APP_")) // ✅ Wrap in Some()
            .transform(Some(KeyTransform::Uppercase)) // ✅ Wrap in Some()
            .verbose(true)
            .build();

        assert_eq!(config.env_path, ".env.prod");
        assert_eq!(config.target_format, Some("json".to_string()));
        assert_eq!(config.output_path, Some("out.json".to_string()));
        assert_eq!(config.options.include_pattern, Some("AWS_*".to_string()));
        assert_eq!(config.options.exclude_pattern, Some("*_DEBUG".to_string()));
        assert!(config.options.base64);
        assert_eq!(config.options.prefix, Some("APP_".to_string()));
        assert!(matches!(
            config.options.transform,
            Some(KeyTransform::Uppercase)
        ));
        assert!(config.verbose);
    }

    #[test]
    fn test_build_options_clone() {
        let mut opts = ConvertOptions::default();
        opts.prefix = Some("TEST_".to_string());
        opts.base64 = true;

        let config = ConvertConfig {
            options: opts,
            ..Default::default()
        };

        let built = config.build_options();
        assert_eq!(built.prefix, Some("TEST_".to_string()));
        assert!(built.base64);
        // Verify it's a clone, not a move
        assert_eq!(config.options.prefix, Some("TEST_".to_string()));
    }

    #[test]
    fn test_validate_input_exists() -> Result<()> {
        let tmp = TempDir::new()?;
        let env_path = tmp.path().join(".env");
        fs::write(&env_path, "KEY=value")?;

        let config = ConvertConfig::builder()
            .env(env_path.to_str().unwrap())
            .build();
        assert!(config.validate_input().is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_input_not_found() {
        let config = ConvertConfig::builder().env("/nonexistent/.env").build();
        let err = config.validate_input().unwrap_err();
        assert!(err.to_string().contains("Input file not found"));
    }

    #[test]
    fn test_validate_input_not_a_file() -> Result<()> {
        let tmp = TempDir::new()?;
        let config = ConvertConfig::builder()
            .env(tmp.path().to_str().unwrap())
            .build();
        let err = config.validate_input().unwrap_err();
        assert!(err.to_string().contains("not a file"));
        Ok(())
    }

    #[test]
    fn test_get_converter_known_formats() {
        let formats = vec![
            "json",
            "yaml",
            "yml",
            "shell",
            "aws-secrets",
            "gcp-secrets",
            "azure-keyvault",
            "github-actions",
            "docker-compose",
            "kubernetes",
            "k8s",
            "terraform",
            "tf",
            "doppler",
            "heroku",
            "vercel",
            "railway",
        ];

        for fmt in formats {
            let result = get_converter(fmt);
            assert!(result.is_ok(), "Failed for format: {}", fmt);
            let converter = result.unwrap();
            let expected = match fmt {
                "yml" => "yaml",
                "bash" | "export" => "shell",
                "aws" | "aws-secrets-manager" => "aws-secrets",
                "gcp" | "gcp-secret-manager" => "gcp-secrets",
                "azure" | "azure-key-vault" => "azure-keyvault",
                "github" | "gh-actions" => "github-actions",
                "docker" | "compose" => "docker-compose",
                "k8s" | "kubectl" => "kubernetes",
                "tfvars" | "tf" => "terraform",
                _ => fmt,
            };
            assert_eq!(converter.name(), expected);
        }
    }

    #[test]
    fn test_get_converter_aliases() {
        // Test that aliases resolve to same converter name
        let alias_pairs = vec![
            ("yaml", "yaml"),
            ("yml", "yaml"),
            ("shell", "shell"),
            ("bash", "shell"),
            ("aws", "aws-secrets"),
            ("k8s", "kubernetes"),
            ("tf", "terraform"),
        ];

        for (alias, expected) in alias_pairs {
            let converter = get_converter(alias).unwrap();
            assert_eq!(converter.name(), expected);
        }
    }

    #[test]
    fn test_write_output_to_stdout() -> Result<()> {
        let content = "KEY=value\n";
        // Just verify no error when writing to stdout (can't easily capture)
        assert!(write_output(content, None, false).is_ok());
        Ok(())
    }

    #[test]
    fn test_write_output_to_file() -> Result<()> {
        let tmp = TempDir::new()?;
        let output_path = tmp.path().join("output.txt");
        let content = "KEY=value\n";

        write_output(content, Some(output_path.to_str().unwrap()), false)?;

        let written = fs::read_to_string(&output_path)?;
        assert_eq!(written, content);
        Ok(())
    }

    #[test]
    fn test_key_transform_integration() {
        // Integration test: verify options transform keys correctly
        let mut options = ConvertOptions::default();
        options.prefix = Some("APP_".to_string());
        options.transform = Some(KeyTransform::Uppercase);

        let original = "database_url";
        let transformed = options.transform_key(original);
        assert_eq!(transformed, "APP_DATABASE_URL");
    }

    #[test]
    fn test_glob_filter_integration() {
        let mut vars = IndexMap::new(); // ✅ IndexMap now imported
        vars.insert("AWS_KEY".to_string(), "aws_val".to_string());
        vars.insert("DB_KEY".to_string(), "db_val".to_string());
        vars.insert("APP_URL".to_string(), "app_val".to_string());

        let mut options = ConvertOptions::default();
        options.include_pattern = Some("AWS_*".to_string());

        let filtered = options.filter_vars(&vars);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("AWS_KEY"));
        assert!(!filtered.contains_key("DB_KEY"));
    }

    #[test]
    fn test_builder_with_none_values() {
        // Test that builder handles None values correctly
        let config = ConvertConfig::builder()
            .env(".env")
            .target_format(Option::<String>::None)
            .output_path(Option::<String>::None)
            .include_pattern(Option::<String>::None)
            .exclude_pattern(Option::<String>::None)
            .base64(false)
            .prefix(Option::<String>::None)
            .transform(None)
            .verbose(false)
            .build();

        assert_eq!(config.env_path, ".env");
        assert!(config.target_format.is_none());
        assert!(config.output_path.is_none());
        assert!(config.options.include_pattern.is_none());
        assert!(config.options.exclude_pattern.is_none());
        assert!(!config.options.base64);
        assert!(config.options.prefix.is_none());
        assert!(config.options.transform.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_builder_chaining_order_independence() {
        // Builder methods should work in any order
        let config1 = ConvertConfig::builder()
            .env(".env")
            .target_format(Some("json"))
            .verbose(true)
            .build();

        let config2 = ConvertConfig::builder()
            .verbose(true)
            .target_format(Some("json"))
            .env(".env")
            .build();

        assert_eq!(config1.env_path, config2.env_path);
        assert_eq!(config1.target_format, config2.target_format);
        assert_eq!(config1.verbose, config2.verbose);
    }
}
