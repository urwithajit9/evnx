/// Convert command - transform .env to different formats
///
/// Orchestrates format conversion using the converter infrastructure
use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
use std::fs;

use crate::core::{
    converter::{ConvertOptions, Converter, KeyTransform},
    Parser,
};
use crate::formats;

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

    // Build conversion options
    let mut options = ConvertOptions::default();
    options.include_pattern = include;
    options.exclude_pattern = exclude;
    options.base64 = base64;
    options.prefix = prefix;
    options.transform = transform.as_ref().and_then(|t| match t.as_str() {
        "uppercase" => Some(KeyTransform::Uppercase),
        "lowercase" => Some(KeyTransform::Lowercase),
        "camelCase" => Some(KeyTransform::CamelCase),
        "snake_case" => Some(KeyTransform::SnakeCase),
        _ => None,
    });

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
                "json - Generic JSON key-value",
                "aws-secrets - AWS Secrets Manager JSON",
                "github-actions - GitHub Actions secrets (ready to paste)",
                "docker-compose - Docker Compose YAML environment",
                "kubernetes - Kubernetes Secret YAML",
                "shell - Shell export script",
                "terraform - Terraform .tfvars",
                "yaml - Generic YAML key-value",
            ];

            let selection = Select::new()
                .with_prompt("Select output format")
                .items(&formats)
                .default(0)
                .interact()?;

            formats[selection]
                .split('-')
                .next()
                .unwrap()
                .trim()
                .to_string()
        }
    };

    // Get converter
    let converter: Box<dyn Converter> = match format.as_str() {
        "json" => Box::new(formats::json::JsonConverter),
        "aws" | "aws-secrets" => Box::new(formats::aws::AwsSecretsConverter),
        "github" | "github-actions" => Box::new(formats::github::GitHubActionsConverter::default()),
        "docker" | "docker-compose" => Box::new(formats::docker::DockerComposeConverter),
        "kubernetes" | "k8s" => Box::new(formats::kubernetes::KubernetesSecretConverter::default()),
        "shell" | "bash" => Box::new(formats::shell::ShellExportConverter),
        "terraform" | "tfvars" => Box::new(formats::terraform::TerraformConverter),
        "yaml" | "yml" => Box::new(formats::yaml::YamlConverter),
        _ => {
            eprintln!("{} Unknown format: {}", "✗".red(), format);
            eprintln!("Supported formats: json, aws-secrets, github-actions, docker-compose, kubernetes, shell, terraform, yaml");
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
            fs::write(&path, &result)?;
            println!("{} Converted successfully", "✓".green());
            println!("Output written to: {}", path);
        }
        None => {
            println!("{}", result);
        }
    }

    Ok(())
}
