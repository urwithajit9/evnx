/// Migrate command - migrate secrets to cloud providers
///
/// Currently supports GitHub Actions with plans for AWS, Doppler, etc.
use anyhow::{anyhow, Context, Result};
use colored::*;
use dialoguer::{Confirm, Input, Password, Select};
use std::collections::HashMap;
use std::time::Duration;

#[cfg(feature = "migrate")]
use indicatif::{ProgressBar, ProgressStyle};
#[cfg(feature = "migrate")]
use serde::{Deserialize, Serialize};

use crate::core::Parser;

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

    println!(
        "\n{}",
        "┌─ Migrate secrets to a new system ───────────────────┐".cyan()
    );
    println!(
        "{}",
        "│ This will help you move from .env to a secret       │".cyan()
    );
    println!(
        "{}",
        "│ manager or between secret management systems        │".cyan()
    );
    println!(
        "{}\n",
        "└──────────────────────────────────────────────────────┘".cyan()
    );

    // Determine source and destination
    let source = from.unwrap_or_else(|| {
        let options = vec!["env-file", "aws", "gcp", "environment"];
        let selection = Select::new()
            .with_prompt("What are you migrating from?")
            .items(&options)
            .default(0)
            .interact()
            .unwrap();
        options[selection].to_string()
    });

    let destination = to.unwrap_or_else(|| {
        let options = vec![
            "github-actions",
            "aws-secrets-manager",
            "doppler",
            "infisical",
            "gcp-secret-manager",
        ];
        let selection = Select::new()
            .with_prompt("What are you migrating to?")
            .items(&options)
            .default(0)
            .interact()
            .unwrap();
        options[selection].to_string()
    });

    // Load source secrets
    let secrets = load_secrets(&source, &source_file, verbose)?;

    println!(
        "\n{} Loaded {} secrets from {}",
        "✓".green(),
        secrets.len(),
        source
    );

    // Migrate based on destination
    match destination.as_str() {
        "github-actions" => {
            #[cfg(feature = "migrate")]
            migrate_to_github_actions(
                &secrets,
                repo,
                github_token,
                dry_run,
                skip_existing,
                overwrite,
                verbose,
            )?;

            #[cfg(not(feature = "migrate"))]
            {
                println!(
                    "{} GitHub Actions migration requires the migrate feature.",
                    "✗".red()
                );
                println!("Rebuild with: cargo build --features migrate");
            }
        }
        "aws-secrets-manager" => {
            migrate_to_aws(&secrets, secret_name, aws_profile, dry_run, verbose)?;
        }
        "doppler" => {
            migrate_to_doppler(&secrets, dry_run, verbose)?;
        }
        "infisical" => {
            migrate_to_infisical(&secrets, dry_run, verbose)?;
        }
        "gcp-secret-manager" => {
            println!(
                "{} GCP Secret Manager migration not yet implemented",
                "⚠️".yellow()
            );
            println!("Use: dotenv-space convert --to gcp-secrets > secrets.json");
            println!("Then upload manually via gcloud CLI");
        }
        _ => {
            return Err(anyhow!("Unsupported destination: {}", destination));
        }
    }

    Ok(())
}

/// Load secrets from source
fn load_secrets(source: &str, file: &str, _verbose: bool) -> Result<HashMap<String, String>> {
    match source {
        "env-file" => {
            let parser = Parser::default();
            let env_file = parser
                .parse_file(file)
                .with_context(|| format!("Failed to parse {}", file))?;
            Ok(env_file.vars)
        }
        "environment" => {
            let mut secrets = HashMap::new();
            for (key, value) in std::env::vars() {
                secrets.insert(key, value);
            }
            Ok(secrets)
        }
        _ => Err(anyhow!("Unsupported source: {}", source)),
    }
}

/// Migrate to GitHub Actions Secrets
#[cfg(feature = "migrate")]
fn migrate_to_github_actions(
    secrets: &HashMap<String, String>,
    repo: Option<String>,
    token: Option<String>,
    dry_run: bool,
    skip_existing: bool,
    overwrite: bool,
    verbose: bool,
) -> Result<()> {
    // Get repository
    let repository = repo
        .or_else(|| {
            Input::new()
                .with_prompt("GitHub repository (owner/repo)")
                .interact_text()
                .ok()
        })
        .ok_or_else(|| anyhow!("Repository is required"))?;

    // Get GitHub token
    let github_token = token
        .or_else(|| {
            std::env::var("GITHUB_TOKEN").ok().or_else(|| {
                Password::new()
                    .with_prompt("GitHub Personal Access Token")
                    .interact()
                    .ok()
            })
        })
        .ok_or_else(|| anyhow!("GitHub token is required"))?;

    println!("\nAnalyzing migration...");

    // Check existing secrets
    let existing_secrets = if !dry_run {
        fetch_existing_github_secrets(&repository, &github_token, verbose)?
    } else {
        vec![]
    };

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
                if Confirm::new()
                    .with_prompt(format!("Overwrite existing secret {}?", key))
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

    // Show migration plan
    println!(
        "\n{} Read {} variables from source",
        "✓".green(),
        secrets.len()
    );
    if !existing_secrets.is_empty() {
        println!(
            "{} Repository has {} existing secrets",
            "ℹ️".cyan(),
            existing_secrets.len()
        );
    }
    println!("\n{}", "Migration plan:".bold());
    println!("  • {} variables to upload", to_upload.len());
    if !to_skip.is_empty() {
        println!("  • {} existing secrets will be skipped", to_skip.len());
    }
    if !conflicts.is_empty() {
        println!("  • {} conflicts (will overwrite)", conflicts.len());
    }

    if dry_run {
        println!("\n{} Dry run mode - no changes will be made", "ℹ️".cyan());
        return Ok(());
    }

    println!();
    if !Confirm::new()
        .with_prompt("Proceed with migration?")
        .default(true)
        .interact()?
    {
        println!("{} Migration cancelled", "ℹ️".cyan());
        return Ok(());
    }

    println!("\nUploading to GitHub Actions Secrets...");

    let pb = ProgressBar::new(to_upload.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut uploaded = 0;
    let mut failed = 0;

    for (key, value) in &to_upload {
        pb.set_message(key.clone());

        match upload_github_secret(&repository, &github_token, key, value, verbose) {
            Ok(_) => {
                uploaded += 1;
                pb.println(format!("  {} {}", "✓".green(), key));
            }
            Err(e) => {
                failed += 1;
                pb.println(format!("  {} {} - {}", "✗".red(), key, e));
            }
        }

        pb.inc(1);
        std::thread::sleep(Duration::from_millis(100));
    }

    pb.finish_with_message("Done");

    println!("\n{}", "Summary:".bold());
    println!(
        "  {}/{} secrets uploaded successfully",
        uploaded,
        to_upload.len()
    );
    if !to_skip.is_empty() {
        println!("  {} existing secrets preserved", to_skip.len());
    }
    if failed > 0 {
        println!("  {} errors", failed);
    }

    if uploaded > 0 {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Update your GitHub Actions workflow to use these secrets");
        println!("  2. Delete or encrypt your local .env file");
        println!("  3. Update .env.example if variable names changed");
    }

    Ok(())
}

/// Fetch existing GitHub secrets
#[cfg(feature = "migrate")]
fn fetch_existing_github_secrets(repo: &str, token: &str, verbose: bool) -> Result<Vec<String>> {
    if verbose {
        println!("Fetching existing secrets from {}", repo);
    }

    let url = format!("https://api.github.com/repos/{}/actions/secrets", repo);

    let client = reqwest::blocking::Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "dotenv-space")
        .send()
        .context("Failed to fetch existing secrets")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API error: {} - {}",
            response.status(),
            response.text()?
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

/// Upload a single secret to GitHub
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

    let pub_key_url = format!(
        "https://api.github.com/repos/{}/actions/secrets/public-key",
        repo
    );

    let client = reqwest::blocking::Client::new();
    let pub_key_response = client
        .get(&pub_key_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "dotenv-space")
        .send()?;

    if !pub_key_response.status().is_success() {
        return Err(anyhow!(
            "Failed to get public key: {}",
            pub_key_response.status()
        ));
    }

    #[derive(Deserialize)]
    struct PublicKey {
        key_id: String,
        key: String,
    }

    let public_key: PublicKey = pub_key_response.json()?;

    let encrypted_value = encrypt_for_github(&public_key.key, value)?;

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

    let response = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "dotenv-space")
        .json(&payload)
        .send()?;

    if response.status().is_success() || response.status() == reqwest::StatusCode::CREATED {
        Ok(())
    } else {
        Err(anyhow!(
            "Upload failed: {} - {}",
            response.status(),
            response.text()?
        ))
    }
}

#[cfg(feature = "migrate")]
fn encrypt_for_github(public_key_base64: &str, value: &str) -> Result<String> {
    use base64::{engine::general_purpose, Engine as _};

    let _public_key = general_purpose::STANDARD
        .decode(public_key_base64)
        .context("Failed to decode public key")?;

    // TODO: Replace with proper libsodium sealed box encryption
    let encrypted = general_purpose::STANDARD.encode(value.as_bytes());
    Ok(encrypted)
}

/// Migrate to AWS Secrets Manager
fn migrate_to_aws(
    secrets: &HashMap<String, String>,
    secret_name: Option<String>,
    aws_profile: Option<String>,
    dry_run: bool,
    _verbose: bool,
) -> Result<()> {
    println!("{} AWS Secrets Manager migration", "ℹ️".cyan());

    let name = secret_name.unwrap_or_else(|| {
        Input::new()
            .with_prompt("Secret name (e.g., prod/myapp/config)")
            .interact_text()
            .unwrap()
    });

    let json = serde_json::to_string_pretty(secrets)?;

    if dry_run {
        println!("\n{}", "Would upload to AWS Secrets Manager:".bold());
        println!("Secret name: {}", name);
        println!("JSON payload:\n{}", json);
        return Ok(());
    }

    println!("\n{}", "Run this command to upload:".bold());

    let profile_flag = aws_profile
        .map(|p| format!("--profile {} ", p))
        .unwrap_or_default();

    println!("aws secretsmanager create-secret \\");
    println!("  {}--name <SECRET_NAME> \\", profile_flag);
    println!("  --secret-string '{}'", json.replace('\'', "\\'"));

    println!("\n{}", "Or update existing:".yellow());
    println!("aws secretsmanager update-secret \\");
    println!("  {}--secret-id <SECRET_ID> \\", profile_flag);
    println!("  --secret-string '{}'", json.replace('\'', "\\'"));

    Ok(())
}

/// Migrate to Doppler
fn migrate_to_doppler(
    secrets: &HashMap<String, String>,
    dry_run: bool,
    _verbose: bool,
) -> Result<()> {
    println!("{} Doppler migration requires Doppler CLI", "ℹ️".cyan());
    println!("Install: https://docs.doppler.com/docs/cli");

    if dry_run {
        println!("\n{}", "Would upload to Doppler".bold());
        return Ok(());
    }

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
    for (key, value) in secrets {
        println!("doppler secrets set {} '{}'", key, value);
    }

    Ok(())
}

/// Migrate to Infisical
fn migrate_to_infisical(
    secrets: &HashMap<String, String>,
    dry_run: bool,
    _verbose: bool,
) -> Result<()> {
    println!("{} Infisical migration requires Infisical CLI", "ℹ️".cyan());
    println!("Install: https://infisical.com/docs/cli/overview");

    if dry_run {
        println!("\n{}", "Would upload to Infisical".bold());
        return Ok(());
    }

    println!("\n{}", "Upload secrets with:".bold());
    for (key, value) in secrets {
        println!("infisical secrets set {} '{}'", key, value);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_secrets() {
        // Test loading from env file - requires test fixtures
    }

    #[test]
    #[cfg(feature = "migrate")]
    fn test_encrypt_for_github() {
        let public_key = "dGVzdF9rZXk="; // valid base64 for "test_key"
        let value = "test_value";
        let result = encrypt_for_github(public_key, value);
        assert!(result.is_ok());
    }
}
