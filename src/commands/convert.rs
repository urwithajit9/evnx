//! Convert command - transform .env to different formats
//!
//! # Overview
//!
//! Orchestrates format conversion using the converter infrastructure.
//! Supports 14+ output formats for various deployment targets and secret managers.
//!
//! # Architecture
//!
//! ```text
//! .env file → Parser → HashMap<K,V> → ConvertOptions → Converter → Output
//! ```
//!
//! # Adding New Formats
//!
//! To add a new format converter:
//!
//! ## 1. Create the converter file
//!
//! Create `src/formats/myformat.rs`:
//!
//! ```rust
//! use crate::core::converter::{Converter, ConvertOptions};
//! use anyhow::Result;
//! use std::collections::HashMap;
//!
//! pub struct MyFormatConverter;
//!
//! impl Converter for MyFormatConverter {
//!     fn convert(&self, vars: &HashMap<String, String>, options: &ConvertOptions) -> Result<String> {
//!         let filtered = options.filter_vars(vars);
//!         // ... your format logic
//!         Ok(output)
//!     }
//!
//!     fn name(&self) -> &str { "myformat" }
//!     fn description(&self) -> &str { "My format description" }
//! }
//! ```
//!
//! ## 2. Declare in mod.rs
//!
//! Add to `src/formats/mod.rs`:
//!
//! ```rust
//! pub mod myformat;
//! pub use myformat::MyFormatConverter;
//! ```
//!
//! ## 3. Add to this file (3 locations)
//!
//! **A. Interactive menu** (in `run()` function, `formats` vec):
//! ```rust
//! "myformat - Description of my format",
//! ```
//!
//! **B. Converter match** (in `run()` function, `match format.as_str()`):
//! ```rust
//! "myformat" | "alias" => Box::new(formats::MyFormatConverter),
//! ```
//!
//! **C. Error message** (in unknown format handler):
//! ```rust
//! eprintln!("    ..., myformat");
//! ```
//!
//! ## 4. Test
//!
//! ```bash
//! cargo run -- convert --to myformat
//! ```
//!
//! # Examples
//!
//! ```bash
//! # Interactive mode
//! dotenv-space convert
//!
//! # Direct conversion
//! dotenv-space convert --to json
//!
//! # With filtering
//! dotenv-space convert --to aws-secrets --include "AWS_*"
//!
//! # With transformation
//! dotenv-space convert --to kubernetes --base64
//! ```
use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
use std::fs;

use crate::core::{
    converter::{ConvertOptions, Converter, KeyTransform},
    Parser,
};
use crate::formats;

// Run the convert command
//
// Converts .env file to various output formats with optional filtering and transformation.
//
// # Arguments
//
// * `env` - Path to .env file
// * `to` - Target format (None = interactive mode)
// * `output` - Optional output file (None = stdout)
// * `include` - Include only matching vars (glob pattern)
// * `exclude` - Exclude matching vars (glob pattern)
// * `base64` - Base64-encode all values
// * `prefix` - Add prefix to all keys
// * `transform` - Key transformation (uppercase/lowercase/camelCase/snake_case)
// * `verbose` - Enable verbose output
//
// # Supported Formats (14)
//
// **Generic:** json, yaml, shell
// **Cloud:** aws-secrets, gcp-secrets, azure-keyvault
// **CI/CD:** github-actions
// **Containers:** docker-compose, kubernetes
// **IaC:** terraform
// **Secret Managers:** doppler, heroku, vercel, railway
#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    to: Option<String>,
    output: Option<String>,
    include: Option<String>,
    exclude: Option<String>,
    base64: bool,
    prefix: Option<String>,
    transform: Option<String>,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running convert in verbose mode".dimmed());
    }

    // Parse .env file
    let parser = Parser::default();
    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse {}", env))?;

    if verbose {
        println!("Loaded {} variables from {}", env_file.vars.len(), env);
    }

    // // Build conversion options
    // let mut options = ConvertOptions::default();
    // options.include_pattern = include;
    // options.exclude_pattern = exclude;
    // options.base64 = base64;
    // options.prefix = prefix;
    // options.transform = transform.as_ref().and_then(|t| match t.as_str() {
    //     "uppercase" => Some(KeyTransform::Uppercase),
    //     "lowercase" => Some(KeyTransform::Lowercase),
    //     "camelCase" => Some(KeyTransform::CamelCase),
    //     "snake_case" => Some(KeyTransform::SnakeCase),
    //     _ => None,
    // });
    let options = ConvertOptions {
        include_pattern: include,
        exclude_pattern: exclude,
        base64,
        prefix,
        transform: transform.as_ref().and_then(|t| match t.as_str() {
            "uppercase" => Some(KeyTransform::Uppercase),
            "lowercase" => Some(KeyTransform::Lowercase),
            "camelCase" => Some(KeyTransform::CamelCase),
            "snake_case" => Some(KeyTransform::SnakeCase),
            _ => None,
        }),
    };

    // Determine format
    let format = match to {
        Some(f) => f,
        None => {
            // Interactive mode
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
                .interact()?;

            // Extract format name (everything before first dash and space)
            formats[selection]
                .split('-')
                .next()
                .unwrap()
                .trim()
                .to_string()
        }
    };

    // Get converter
    // Note: Using qualified paths (formats::JsonConverter) instead of
    // full paths (formats::json::JsonConverter) to avoid import warnings
    let converter: Box<dyn Converter> = match format.as_str() {
        // Generic formats
        "json" => Box::new(formats::JsonConverter),
        "yaml" | "yml" => Box::new(formats::YamlConverter),
        "shell" | "bash" | "export" => Box::new(formats::ShellExportConverter),

        // Cloud providers
        "aws" | "aws-secrets" | "aws-secrets-manager" => Box::new(formats::AwsSecretsConverter),
        "gcp" | "gcp-secrets" | "gcp-secret-manager" => {
            Box::new(formats::GcpSecretConverter::default())
        }
        "azure" | "azure-keyvault" | "azure-key-vault" => {
            Box::new(formats::AzureKeyVaultConverter::default())
        }

        // CI/CD platforms
        "github" | "github-actions" | "gh-actions" => {
            Box::new(formats::GitHubActionsConverter::default())
        }

        // Container platforms
        "docker" | "docker-compose" | "compose" => Box::new(formats::DockerComposeConverter),
        "kubernetes" | "k8s" | "kubectl" => Box::new(formats::KubernetesSecretConverter::default()),

        // Infrastructure as Code
        "terraform" | "tfvars" | "tf" => Box::new(formats::TerraformConverter),

        // Secret management platforms
        "doppler" => Box::new(formats::DopplerConverter),
        "heroku" => Box::new(formats::HerokuConfigConverter::default()),
        "vercel" => Box::new(formats::VercelEnvConverter),
        "railway" => Box::new(formats::RailwayConverter),

        // Unknown format
        _ => {
            eprintln!("{} Unknown format: {}", "✗".red(), format);
            eprintln!();
            eprintln!("{}", "Supported formats:".bold());
            eprintln!();
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
            std::process::exit(1);
        }
    };

    if verbose {
        println!("Converting to {} format...", converter.name());
    }

    // Convert
    let result = converter.convert(&env_file.vars, &options)?;

    // Output
    match output {
        Some(path) => {
            fs::write(&path, &result).with_context(|| format!("Failed to write to {}", path))?;
            println!("{} Converted successfully", "✓".green());
            println!("Output written to: {}", path);
        }
        None => {
            println!("{}", result);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_transform_uppercase() {
        let mut options = ConvertOptions::default();
        options.transform = Some(KeyTransform::Uppercase);
        // ConvertOptions tests are in core/converter.rs
    }
}
