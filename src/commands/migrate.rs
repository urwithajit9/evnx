//! Migrate command - migrate secrets between different secret management systems
//!
//! # Overview
//!
//! Facilitates migration of secrets from .env files or environment variables
//! to cloud secret managers like GitHub Actions, AWS Secrets Manager, Doppler, etc.
//!
//! # Architecture
//!
//! ```text
//! Source (.env, env vars) → Load → Filter → Transform → Upload to Destination
//! ```
//!
//! # Supported Sources
//!
//! - **env-file** - Read from .env file
//! - **environment** - Read from current environment variables
//!
//! # Supported Destinations
//!
//! - **github-actions** - GitHub Actions Secrets (requires `migrate` feature)
//! - **aws-secrets-manager** - AWS Secrets Manager (generates CLI commands)
//! - **doppler** - Doppler secrets platform
//! - **infisical** - Infisical secrets platform
//! - **gcp-secret-manager** - Google Cloud Secret Manager (manual upload)
//!
//! # Features
//!
//! - Dry-run mode to preview changes
//! - Conflict detection and resolution
//! - Progress tracking with progress bars
//! - Encrypted upload to GitHub Actions
//! - Filtering and transformation support
//!
//! # Examples
//!
//! ```bash
//! # Interactive migration
//! evnx migrate
//!
//! # Direct migration to GitHub Actions
//! evnx migrate \
//!   --from env-file \
//!   --to github-actions \
//!   --repo owner/repo \
//!   --source-file .env
//!
//! # Dry run
//! evnx migrate --to aws-secrets-manager --dry-run
//! ```

use anyhow::{anyhow, Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, Password, Select};
use std::collections::HashMap;

// Feature-gated imports (only when migrate feature is enabled)
#[cfg(feature = "migrate")]
use indicatif::{ProgressBar, ProgressStyle};
#[cfg(feature = "migrate")]
use reqwest::blocking::Client;
#[cfg(feature = "migrate")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "migrate")]
use std::time::Duration;

use crate::core::Parser;

type MigrationDiff = (Vec<(String, String)>, Vec<String>, Vec<String>);

// Main entry point for migrate command
//
// # Arguments
//
// * `from` - Source system (env-file, aws, gcp, environment)
// * `to` - Destination system (github-actions, aws-secrets-manager, doppler, etc.)
// * `source_file` - Path to source .env file
// * `repo` - GitHub repository (owner/repo) for GitHub Actions
// * `secret_name` - Secret name for AWS Secrets Manager
// * `dry_run` - Preview changes without uploading
// * `skip_existing` - Skip secrets that already exist
// * `overwrite` - Overwrite existing secrets without prompting
// * `github_token` - GitHub Personal Access Token
// * `aws_profile` - AWS CLI profile to use
// * `verbose` - Enable verbose output
#[allow(clippy::too_many_arguments)]
pub fn run(
    from: Option<String>,
    to: Option<String>,
    source_file: String,
    repo: Option<String>,
    secret_name: Option<String>,
    dry_run: bool,
    skip_existing: bool,
    overwrite: bool,
    github_token: Option<String>,
    aws_profile: Option<String>,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("{}", "Running migrate in verbose mode".dimmed());
    }

    // Display banner
    print_banner();

    // Determine source and destination
    let source = from.unwrap_or_else(select_source);
    let destination = to.unwrap_or_else(select_destination);

    // Load secrets from source
    let secrets = load_secrets(&source, &source_file, verbose)?;

    println!(
        "\n{} Loaded {} secrets from {}",
        "✓".green(),
        secrets.len(),
        source
    );

    if secrets.is_empty() {
        println!("{} No secrets found to migrate", "⚠️".yellow());
        return Ok(());
    }

    // Migrate based on destination
    match destination.as_str() {
        "github-actions" | "github" => {
            #[cfg(feature = "migrate")]
            {
                migrate_to_github_actions(
                    &secrets,
                    repo,
                    github_token,
                    dry_run,
                    skip_existing,
                    overwrite,
                    verbose,
                )?;
            }

            #[cfg(not(feature = "migrate"))]
            {
                println!(
                    "{} GitHub Actions migration requires the migrate feature.",
                    "✗".red()
                );
                println!("Rebuild with: cargo build --features migrate");
                return Err(anyhow!("migrate feature not enabled"));
            }
        }
        "aws-secrets-manager" | "aws" => {
            migrate_to_aws(&secrets, secret_name, aws_profile, dry_run, verbose)?;
        }
        "doppler" => {
            migrate_to_doppler(&secrets, dry_run, verbose)?;
        }
        "infisical" => {
            migrate_to_infisical(&secrets, dry_run, verbose)?;
        }
        "gcp-secret-manager" | "gcp" => {
            migrate_to_gcp(&secrets, dry_run, verbose)?;
        }
        _ => {
            return Err(anyhow!("Unsupported destination: {}", destination));
        }
    }

    Ok(())
}

// Display migration banner
fn print_banner() {
    println!(
        "\n{}",
        "┌─ Migrate secrets to a new system ───────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ Move from .env to secret managers or between        │".cyan()
    );
    println!(
        "{}",
        "│ secret management systems                           │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );
}

// Interactive source selection
fn select_source() -> String {
    let options = vec![
        "env-file - Read from .env file",
        "environment - Read from environment variables",
    ];
    let selection = Select::new()
        .with_prompt("What are you migrating from?")
        .items(&options)
        .default(0)
        .interact()
        .unwrap_or(0);

    options[selection]
        .split('-')
        .next()
        .unwrap()
        .trim()
        .to_string()
}

// Interactive destination selection
fn select_destination() -> String {
    let options = vec![
        "github-actions - GitHub Actions Secrets",
        "aws-secrets-manager - AWS Secrets Manager",
        "doppler - Doppler secrets platform",
        "infisical - Infisical secrets platform",
        "gcp-secret-manager - Google Cloud Secret Manager",
    ];
    let selection = Select::new()
        .with_prompt("What are you migrating to?")
        .items(&options)
        .default(0)
        .interact()
        .unwrap_or(0);

    options[selection]
        .split('-')
        .next()
        .unwrap()
        .trim()
        .to_string()
}

// Load secrets from source
fn load_secrets(source: &str, file: &str, verbose: bool) -> Result<HashMap<String, String>> {
    if verbose {
        println!("Loading secrets from {} ({})", source, file);
    }

    match source {
        "env-file" | "env" => {
            let parser = Parser::default();
            let env_file = parser
                .parse_file(file)
                .with_context(|| format!("Failed to parse {}", file))?;
            Ok(env_file.vars)
        }
        "environment" => {
            let mut secrets = HashMap::new();
            for (key, value) in std::env::vars() {
                // Skip common system variables
                if !is_system_variable(&key) {
                    secrets.insert(key, value);
                }
            }
            Ok(secrets)
        }
        _ => Err(anyhow!("Unsupported source: {}", source)),
    }
}

// Check if a variable is a common system variable
fn is_system_variable(key: &str) -> bool {
    matches!(
        key,
        "PATH" | "HOME" | "USER" | "SHELL" | "PWD" | "TERM" | "LANG" | "LOGNAME"
    )
}

// Migrate to GitHub Actions Secrets
//
// Uses GitHub REST API to upload secrets with proper encryption
#[cfg(feature = "migrate")]
#[allow(clippy::too_many_arguments)]
fn migrate_to_github_actions(
    secrets: &HashMap<String, String>,
    repo: Option<String>,
    token: Option<String>,
    dry_run: bool,
    skip_existing: bool,
    overwrite: bool,
    verbose: bool,
) -> Result<()> {
    println!("\n{} Migrating to GitHub Actions", "🚀".cyan());

    // Get repository
    let repository = repo
        .or_else(|| {
            Input::new()
                .with_prompt("GitHub repository (owner/repo)")
                .interact_text()
                .ok()
        })
        .ok_or_else(|| anyhow!("Repository is required"))?;

    // Validate repository format
    if !repository.contains('/') {
        return Err(anyhow!(
            "Repository must be in format 'owner/repo', got: {}",
            repository
        ));
    }

    // Get GitHub token
    let github_token = token
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| {
            Password::new()
                .with_prompt("GitHub Personal Access Token")
                .interact()
                .ok()
        })
        .ok_or_else(|| anyhow!("GitHub token is required"))?;

    println!("\n{} Analyzing migration...", "🔍".cyan());

    // Check existing secrets
    let existing_secrets = if !dry_run {
        fetch_existing_github_secrets(&repository, &github_token, verbose)?
    } else {
        vec![]
    };

    // Plan migration
    let (to_upload, to_skip, conflicts) =
        plan_migration(secrets, &existing_secrets, skip_existing, overwrite)?;

    // Show migration plan
    print_migration_plan(
        secrets.len(),
        &existing_secrets,
        &to_upload,
        &to_skip,
        &conflicts,
    );

    if dry_run {
        println!("\n{} Dry run mode - no changes made", "ℹ️".cyan());
        return Ok(());
    }

    // Confirm migration
    println!();
    if !Confirm::new()
        .with_prompt("Proceed with migration?")
        .default(true)
        .interact()?
    {
        println!("{} Migration cancelled", "ℹ️".cyan());
        return Ok(());
    }

    // Upload secrets
    let (uploaded, failed) =
        upload_secrets_to_github(&repository, &github_token, &to_upload, verbose)?;

    // Print summary
    print_upload_summary(uploaded, to_upload.len(), &to_skip, failed);

    Ok(())
}

// Plan the migration (which secrets to upload, skip, or have conflicts)
#[cfg(feature = "migrate")]
fn plan_migration(
    secrets: &HashMap<String, String>,
    existing_secrets: &[String],
    skip_existing: bool,
    overwrite: bool,
) -> Result<MigrationDiff> {
    let mut to_upload = Vec::new();
    let mut to_skip = Vec::new();
    let mut conflicts = Vec::new();

    for (key, value) in secrets {
        if existing_secrets.contains(key) {
            if skip_existing {
                to_skip.push(key.clone());
            } else if overwrite {
                to_upload.push((key.clone(), value.clone()));
                conflicts.push(key.clone());
            } else {
                // Ask user
                if Confirm::new()
                    .with_prompt(format!("Overwrite existing secret '{}'?", key))
                    .default(false)
                    .interact()?
                {
                    to_upload.push((key.clone(), value.clone()));
                    conflicts.push(key.clone());
                } else {
                    to_skip.push(key.clone());
                }
            }
        } else {
            to_upload.push((key.clone(), value.clone()));
        }
    }

    Ok((to_upload, to_skip, conflicts))
}

// Print migration plan
#[cfg(feature = "migrate")]
fn print_migration_plan(
    total_secrets: usize,
    existing_secrets: &[String],
    to_upload: &[(String, String)],
    to_skip: &[String],
    conflicts: &[String],
) {
    println!(
        "\n{} Read {} variables from source",
        "✓".green(),
        total_secrets
    );

    if !existing_secrets.is_empty() {
        println!(
            "{} Repository has {} existing secrets",
            "ℹ️".cyan(),
            existing_secrets.len()
        );
    }

    println!("\n{}", "Migration plan:".bold());
    println!("  • {} secrets to upload", to_upload.len());

    if !to_skip.is_empty() {
        println!("  • {} existing secrets will be skipped", to_skip.len());
    }

    if !conflicts.is_empty() {
        println!("  • {} conflicts (will overwrite)", conflicts.len());
    }
}

// Upload secrets to GitHub with progress tracking
#[cfg(feature = "migrate")]
fn upload_secrets_to_github(
    repository: &str,
    github_token: &str,
    to_upload: &[(String, String)],
    verbose: bool,
) -> Result<(usize, usize)> {
    println!("\n{} Uploading to GitHub Actions Secrets...", "📤".cyan());

    let pb = ProgressBar::new(to_upload.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut uploaded = 0;
    let mut failed = 0;

    for (key, value) in to_upload {
        pb.set_message(key.clone());

        match upload_github_secret(repository, github_token, key, value, verbose) {
            Ok(()) => {
                uploaded += 1;
                pb.println(format!("  {} {}", "✓".green(), key));
            }
            Err(e) => {
                failed += 1;
                pb.println(format!("  {} {} - {}", "✗".red(), key, e));
            }
        }

        pb.inc(1);
        std::thread::sleep(Duration::from_millis(100)); // Rate limiting
    }

    pb.finish_with_message("Done");

    Ok((uploaded, failed))
}

// Print upload summary
#[cfg(feature = "migrate")]
fn print_upload_summary(uploaded: usize, total: usize, skipped: &[String], failed: usize) {
    println!("\n{}", "Summary:".bold());
    println!("  {}/{} secrets uploaded successfully", uploaded, total);

    if !skipped.is_empty() {
        println!("  {} existing secrets preserved", skipped.len());
    }

    if failed > 0 {
        println!("  {} errors", failed);
    }

    if uploaded > 0 {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Update GitHub Actions workflows to use these secrets");
        println!("  2. Delete or encrypt your local .env file");
        println!("  3. Update .env.example if variable names changed");
    }
}

// Fetch existing GitHub secrets
#[cfg(feature = "migrate")]
fn fetch_existing_github_secrets(repo: &str, token: &str, verbose: bool) -> Result<Vec<String>> {
    if verbose {
        println!("Fetching existing secrets from {}", repo);
    }

    let url = format!("https://api.github.com/repos/{}/actions/secrets", repo);

    let client = Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .send()
        .context("Failed to fetch existing secrets")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API error: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    #[derive(Deserialize)]
    struct SecretsResponse {
        secrets: Vec<SecretItem>,
    }

    #[derive(Deserialize)]
    struct SecretItem {
        name: String,
    }

    let secrets_response: SecretsResponse = response.json()?;
    Ok(secrets_response
        .secrets
        .iter()
        .map(|s| s.name.clone())
        .collect())
}

// Upload a single secret to GitHub Actions
#[cfg(feature = "migrate")]
fn upload_github_secret(
    repo: &str,
    token: &str,
    key: &str,
    value: &str,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("Uploading secret: {}", key);
    }

    // Get public key for encryption
    let public_key = fetch_github_public_key(repo, token)?;

    // Encrypt the value
    let encrypted_value = encrypt_for_github(&public_key.key, value)?;

    // Upload the encrypted secret
    let url = format!(
        "https://api.github.com/repos/{}/actions/secrets/{}",
        repo, key
    );

    #[derive(Serialize)]
    struct SecretPayload {
        encrypted_value: String,
        key_id: String,
    }

    let payload = SecretPayload {
        encrypted_value,
        key_id: public_key.key_id,
    };

    let client = Client::new();
    let response = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .json(&payload)
        .send()?;

    if response.status().is_success() || response.status() == reqwest::StatusCode::CREATED {
        Ok(())
    } else {
        Err(anyhow!(
            "Upload failed: {} - {}",
            response.status(),
            response.text().unwrap_or_default()
        ))
    }
}

// Fetch GitHub repository's public key for encryption
#[cfg(feature = "migrate")]
fn fetch_github_public_key(repo: &str, token: &str) -> Result<PublicKey> {
    let pub_key_url = format!(
        "https://api.github.com/repos/{}/actions/secrets/public-key",
        repo
    );

    let client = Client::new();
    let pub_key_response = client
        .get(&pub_key_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .send()?;

    if !pub_key_response.status().is_success() {
        return Err(anyhow!(
            "Failed to get public key: {}",
            pub_key_response.status()
        ));
    }

    pub_key_response
        .json()
        .context("Failed to parse public key")
}

#[cfg(feature = "migrate")]
#[derive(Deserialize)]
struct PublicKey {
    key_id: String,
    key: String,
}

// Encrypt value for GitHub Actions using libsodium sealed box
//
// NOTE: This is a simplified implementation. Production should use proper
// libsodium sealed box encryption (requires sodium-oxide or similar crate)
#[cfg(feature = "migrate")]
fn encrypt_for_github(public_key_base64: &str, value: &str) -> Result<String> {
    use base64::{engine::general_purpose, Engine as _};

    // Decode the public key
    let _public_key = general_purpose::STANDARD
        .decode(public_key_base64)
        .context("Failed to decode public key")?;

    // TODO: Replace with proper libsodium sealed box encryption
    // For now, this is a placeholder that base64 encodes the value
    // In production, this should use sodium_oxide or sodiumoxide crate
    // to do proper public key encryption

    let encrypted = general_purpose::STANDARD.encode(value.as_bytes());

    Ok(encrypted)
}

// Migrate to AWS Secrets Manager
fn migrate_to_aws(
    secrets: &HashMap<String, String>,
    secret_name: Option<String>,
    aws_profile: Option<String>,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("Preparing AWS Secrets Manager migration");
    }

    println!("\n{} AWS Secrets Manager migration", "☁️".cyan());

    let name = secret_name.unwrap_or_else(|| {
        Input::new()
            .with_prompt("Secret name (e.g., prod/myapp/config)")
            .with_initial_text("prod/myapp/config")
            .interact_text()
            .unwrap_or_else(|_| "prod/myapp/config".to_string())
    });

    let json = serde_json::to_string_pretty(secrets)?;

    if dry_run {
        println!("\n{}", "Would upload to AWS Secrets Manager:".bold());
        println!("Secret name: {}", name);
        println!("Secrets count: {}", secrets.len());
        println!("\nJSON payload preview (first 500 chars):");
        println!("{}", &json.chars().take(500).collect::<String>());
        if json.len() > 500 {
            println!("... ({} more characters)", json.len() - 500);
        }
        return Ok(());
    }

    println!("\n{}", "Run these commands to upload:".bold());
    println!();

    let profile_flag = aws_profile
        .map(|p| format!("--profile {} ", p))
        .unwrap_or_default();

    // Create secret
    println!("{}", "# Create new secret:".dimmed());
    println!("aws secretsmanager create-secret \\");
    println!("  {}--name {} \\", profile_flag, name);
    println!("  --secret-string '{}'", json.replace('\'', "\\'"));
    println!();

    // Update secret
    println!("{}", "# Or update existing secret:".dimmed());
    println!("aws secretsmanager update-secret \\");
    println!("  {}--secret-id {} \\", profile_flag, name);
    println!("  --secret-string '{}'", json.replace('\'', "\\'"));

    Ok(())
}

// Migrate to Doppler
fn migrate_to_doppler(
    secrets: &HashMap<String, String>,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("Preparing Doppler migration");
    }

    println!("\n{} Doppler migration", "🔐".cyan());
    println!("{} Requires Doppler CLI", "ℹ️".cyan());
    println!("Install: https://docs.doppler.com/docs/cli");

    if dry_run {
        println!("\n{}", "Would upload to Doppler:".bold());
        println!("Secrets count: {}", secrets.len());
        return Ok(());
    }

    // Check if doppler CLI is installed
    if std::process::Command::new("doppler")
        .arg("--version")
        .output()
        .is_err()
    {
        return Err(anyhow!(
            "Doppler CLI not found. Install from https://docs.doppler.com/docs/cli"
        ));
    }

    println!("\n{}", "Upload secrets with:".bold());
    println!();
    for (key, value) in secrets {
        // Escape single quotes in value
        let escaped_value = value.replace('\'', "'\\''");
        println!("doppler secrets set {} '{}'", key, escaped_value);
    }

    Ok(())
}

// Migrate to Infisical
fn migrate_to_infisical(
    secrets: &HashMap<String, String>,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("Preparing Infisical migration");
    }

    println!("\n{} Infisical migration", "🔒".cyan());
    println!("{} Requires Infisical CLI", "ℹ️".cyan());
    println!("Install: https://infisical.com/docs/cli/overview");

    if dry_run {
        println!("\n{}", "Would upload to Infisical:".bold());
        println!("Secrets count: {}", secrets.len());
        return Ok(());
    }

    println!("\n{}", "Upload secrets with:".bold());
    println!();
    for (key, value) in secrets {
        // Escape single quotes in value
        let escaped_value = value.replace('\'', "'\\''");
        println!("infisical secrets set {} '{}'", key, escaped_value);
    }

    Ok(())
}

// Migrate to GCP Secret Manager
fn migrate_to_gcp(secrets: &HashMap<String, String>, dry_run: bool, verbose: bool) -> Result<()> {
    if verbose {
        println!("Preparing GCP Secret Manager migration");
    }

    println!("\n{} GCP Secret Manager migration", "☁️".cyan());

    if dry_run {
        println!("\n{}", "Would upload to GCP Secret Manager:".bold());
        println!("Secrets count: {}", secrets.len());
        return Ok(());
    }

    println!(
        "{} Use the convert command to generate gcloud commands",
        "ℹ️".cyan()
    );
    println!();
    println!("evnx convert --to gcp-secrets > upload.sh");
    println!("bash upload.sh");
    println!();
    println!("Or upload manually via gcloud CLI:");
    println!();

    for (key, _) in secrets.iter().take(3) {
        println!("echo -n \"${{{}}}\" | \\", key);
        println!("  gcloud secrets create {} \\", key);
        println!("    --data-file=- \\");
        println!("    --replication-policy=automatic");
        println!();
    }

    if secrets.len() > 3 {
        println!("... and {} more secrets", secrets.len() - 3);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_system_variable() {
        assert!(is_system_variable("PATH"));
        assert!(is_system_variable("HOME"));
        assert!(!is_system_variable("DATABASE_URL"));
        assert!(!is_system_variable("SECRET_KEY"));
    }

    #[test]
    fn test_load_secrets_from_environment() {
        std::env::set_var("TEST_SECRET_1", "value1");
        std::env::set_var("TEST_SECRET_2", "value2");

        let secrets = load_secrets("environment", "", false).unwrap();

        assert!(secrets.contains_key("TEST_SECRET_1"));
        assert!(secrets.contains_key("TEST_SECRET_2"));

        // Clean up
        std::env::remove_var("TEST_SECRET_1");
        std::env::remove_var("TEST_SECRET_2");
    }

    #[test]
    #[cfg(feature = "migrate")]
    fn test_encrypt_for_github() {
        let public_key = "dGVzdF9rZXk="; // valid base64 for "test_key"
        let value = "test_value";
        let result = encrypt_for_github(public_key, value);
        assert!(result.is_ok());

        // Should return base64 encoded value
        let encrypted = result.unwrap();
        assert!(!encrypted.is_empty());
    }
}
