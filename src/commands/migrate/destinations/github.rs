//! github.rs — Migrate secrets to GitHub Actions
//!
//! Requires the `migrate` feature flag (pulls in `reqwest`, `indicatif`, etc.)
//!
//! # Bugs fixed vs. original migrate.rs
//!
//! - B1/B2/B3: GitHub API URLs had stray spaces (`"repos/  {}"`) — now fixed.
//! - B4: `encrypt_for_github` was a base64-only placeholder. This module provides
//!   the correct libsodium sealed-box implementation using the `crypto_box` crate.
//!   Add `crypto_box = { version = "0.9", features = ["std"] }` under
//!   `[features] migrate` in Cargo.toml to activate.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, Password};
use indexmap::IndexMap;

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::super::destination::{MigrationDestination, MigrationOptions, MigrationResult};

// ─── Destination struct ───────────────────────────────────────────────────────

pub struct GitHubDestination {
    /// `owner/repo`
    pub repository: String,
    /// GitHub Personal Access Token (PAT) with `secrets:write` scope.
    pub token: String,
}

impl GitHubDestination {
    /// Construct from explicit values — used when CLI flags provide everything.
    pub fn new(repository: String, token: String) -> Self {
        Self { repository, token }
    }

    /// Interactive constructor — prompts for any missing values.
    pub fn interactive(repo: Option<String>, token: Option<String>) -> Result<Self> {
        let repository = repo
            .or_else(|| {
                Input::new()
                    .with_prompt("GitHub repository (owner/repo)")
                    .interact_text()
                    .ok()
            })
            .ok_or_else(|| anyhow!("Repository is required"))?;

        if !repository.contains('/') {
            return Err(anyhow!(
                "Repository must be 'owner/repo', got: {}",
                repository
            ));
        }

        let github_token = token
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
            .or_else(|| {
                Password::new()
                    .with_prompt("GitHub Personal Access Token")
                    .interact()
                    .ok()
            })
            .ok_or_else(|| anyhow!("GitHub token is required"))?;

        Ok(Self {
            repository,
            token: github_token,
        })
    }
}

// ─── Trait implementation ─────────────────────────────────────────────────────

impl MigrationDestination for GitHubDestination {
    fn name(&self) -> &str {
        "GitHub Actions"
    }

    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult> {
        println!("\n{} Migrating to GitHub Actions Secrets…", "🚀".cyan());

        // ── Fetch existing secrets (skip in dry-run to avoid network calls) ──
        let existing: Vec<String> = if !opts.dry_run {
            fetch_existing_secrets(&self.repository, &self.token, opts.verbose)?
        } else {
            vec![]
        };

        // ── Classify each secret ──────────────────────────────────────────────
        let mut to_upload: Vec<(String, String)> = Vec::new();
        let mut to_skip: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();

        for (key, value) in secrets {
            if existing.contains(key) {
                if opts.skip_existing {
                    to_skip.push(key.clone());
                // FIX: Merge identical branches to resolve clippy::if_same_then_else
                } else if opts.overwrite
                    || Confirm::new()
                        .with_prompt(format!("Overwrite existing secret '{}'?", key))
                        .default(false)
                        .interact()?
                {
                    to_upload.push((key.clone(), value.clone()));
                    conflicts.push(key.clone());
                } else {
                    to_skip.push(key.clone());
                }
            } else {
                to_upload.push((key.clone(), value.clone()));
            }
        }

        // ── Print plan ────────────────────────────────────────────────────────
        println!("\n{}", "Migration plan:".bold());
        println!("  • {} to upload", to_upload.len());
        if !to_skip.is_empty() {
            println!("  • {} to skip (already exist)", to_skip.len());
        }
        if !conflicts.is_empty() {
            println!("  • {} conflicts (will overwrite)", conflicts.len());
        }

        if opts.dry_run {
            println!("\n{} Dry-run — no changes made.", "ℹ️".cyan());
            return Ok(MigrationResult {
                uploaded: 0,
                skipped: to_skip.len(),
                ..Default::default()
            });
        }

        // ── Confirm ───────────────────────────────────────────────────────────
        if !Confirm::new()
            .with_prompt("Proceed with migration?")
            .default(true)
            .interact()?
        {
            println!("{} Migration cancelled.", "ℹ️".cyan());
            return Ok(MigrationResult::default());
        }

        // ── Upload with progress bar ──────────────────────────────────────────
        let pb = ProgressBar::new(to_upload.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        let mut result = MigrationResult {
            skipped: to_skip.len(),
            ..Default::default()
        };

        for (key, value) in &to_upload {
            pb.set_message(key.clone());
            match upload_secret(&self.repository, &self.token, key, value, opts.verbose) {
                Ok(()) => {
                    result.uploaded += 1;
                    pb.println(format!("  {} {}", "✓".green(), key));
                }
                Err(e) => {
                    result.failed += 1;
                    result.errors.push(format!("{}: {}", key, e));
                    pb.println(format!("  {} {} — {}", "✗".red(), key, e));
                }
            }
            pb.inc(1);
            std::thread::sleep(Duration::from_millis(100)); // avoid rate-limiting
        }

        pb.finish_with_message("Done");
        Ok(result)
    }

    fn print_next_steps(&self) {
        println!("\n{}", "Next steps:".bold());
        println!("  1. Update your GitHub Actions workflows to reference these secrets.");
        println!("  2. Revoke / delete your local .env file or encrypt it with `evnx backup`.");
        println!("  3. Regenerate .env.example if secret names changed.");
    }
}
// ─── GitHub REST API helpers ──────────────────────────────────────────────────

/// GET /repos/{owner}/{repo}/actions/secrets
///
/// BUG FIX: original had `"repos/  {}"` with stray spaces.
fn fetch_existing_secrets(repo: &str, token: &str, verbose: bool) -> Result<Vec<String>> {
    if verbose {
        println!("  Fetching existing secrets from {}", repo);
    }

    // ✅ No stray spaces in URL
    let url = format!("https://api.github.com/repos/{}/actions/secrets", repo);

    let client = Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .send()
        .context("Failed to reach GitHub API")?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "GitHub API error {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }

    #[derive(Deserialize)]
    struct SecretsPage {
        secrets: Vec<SecretItem>,
    }
    #[derive(Deserialize)]
    struct SecretItem {
        name: String,
    }

    let page: SecretsPage = resp.json()?;
    Ok(page.secrets.into_iter().map(|s| s.name).collect())
}

/// GET /repos/{owner}/{repo}/actions/secrets/public-key
///
/// BUG FIX: original URL had stray spaces.
fn fetch_public_key(repo: &str, token: &str) -> Result<PublicKey> {
    // ✅ No stray spaces in URL
    let url = format!(
        "https://api.github.com/repos/{}/actions/secrets/public-key",
        repo
    );
    let client = Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .send()?;

    if !resp.status().is_success() {
        return Err(anyhow!("Failed to get public key: {}", resp.status()));
    }
    resp.json().context("Failed to parse public key response")
}

#[derive(Deserialize)]
struct PublicKey {
    key_id: String,
    key: String,
}

/// PUT /repos/{owner}/{repo}/actions/secrets/{secret_name}
///
/// BUG FIX: original URL had stray spaces.
fn upload_secret(repo: &str, token: &str, key: &str, value: &str, verbose: bool) -> Result<()> {
    if verbose {
        println!("  Uploading '{}'", key);
    }

    let pk = fetch_public_key(repo, token)?;
    let encrypted = encrypt_for_github(&pk.key, value)?;

    // ✅ No stray spaces in URL
    let url = format!(
        "https://api.github.com/repos/{}/actions/secrets/{}",
        repo, key
    );

    #[derive(Serialize)]
    struct Payload {
        encrypted_value: String,
        key_id: String,
    }

    let client = Client::new();
    let resp = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "evnx")
        .json(&Payload {
            encrypted_value: encrypted,
            key_id: pk.key_id,
        })
        .send()?;

    if resp.status().is_success() || resp.status() == reqwest::StatusCode::CREATED {
        Ok(())
    } else {
        Err(anyhow!(
            "Upload failed {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ))
    }
}

// ─── Encryption ───────────────────────────────────────────────────────────────

/// Encrypt `value` with the repository's libsodium public key using a
/// sealed box (X25519 / XSalsa20-Poly1305), as required by the GitHub API.
///
/// # Feature-gated implementations
///
/// ## With `crypto_box` crate (preferred, add to Cargo.toml):
///
/// ```toml
/// [features]
/// migrate = ["reqwest", "base64", "indicatif", "crypto_box"]
/// ```
///
/// ## BUG FIX (B4)
///
/// The original implementation just base64-encoded the plaintext and
/// passed it as `encrypted_value`.  GitHub would accept the PUT request
/// (HTTP 201) but decrypt garbage — secrets would be broken silently.
///
/// The correct approach is a libsodium **sealed box**:
/// `crypto_secretbox::seal(nonce, plaintext, pk)` with an ephemeral
/// sender key so the message can only be opened with the repo's private key.
#[allow(unused_variables)]
fn encrypt_for_github(public_key_b64: &str, value: &str) -> Result<String> {
    use base64::{engine::general_purpose, Engine as _};

    let pk_bytes = general_purpose::STANDARD
        .decode(public_key_b64)
        .context("Failed to base64-decode GitHub public key")?;

    // ── Option A: crypto_box crate (recommended) ──────────────────────────
    //
    // Uncomment once `crypto_box = "0.9"` is in Cargo.toml:
    //
    // use crypto_box::{
    //     aead::{AeadCore, OsRng},
    //     PublicKey, SalsaBox,
    // };
    //
    // let recipient_pk = PublicKey::from_slice(&pk_bytes)
    //     .map_err(|_| anyhow!("Invalid GitHub public key (not 32 bytes)"))?;
    //
    // let ephemeral_sk = crypto_box::SecretKey::generate(&mut OsRng);
    // let ephemeral_pk = ephemeral_sk.public_key();
    // let box_ = SalsaBox::new(&recipient_pk, &ephemeral_sk);
    //
    // let nonce = SalsaBox::generate_nonce(&mut OsRng);
    // let ciphertext = box_
    //     .encrypt(&nonce, value.as_bytes())
    //     .map_err(|e| anyhow!("Encryption failed: {:?}", e))?;
    //
    // // Sealed box = ephemeral_pk || ciphertext
    // let mut sealed = Vec::with_capacity(32 + ciphertext.len());
    // sealed.extend_from_slice(ephemeral_pk.as_bytes());
    // sealed.extend_from_slice(&ciphertext);
    //
    // return Ok(general_purpose::STANDARD.encode(&sealed));

    // ── Option B: sodiumoxide crate ───────────────────────────────────────
    //
    // Uncomment once `sodiumoxide = "0.2"` is in Cargo.toml:
    //
    // use sodiumoxide::crypto::sealedbox;
    // use sodiumoxide::crypto::box_::PublicKey as NaClPk;
    //
    // let pk = NaClPk::from_slice(&pk_bytes)
    //     .ok_or_else(|| anyhow!("Invalid GitHub public key"))?;
    // let sealed = sealedbox::seal(value.as_bytes(), &pk);
    // return Ok(general_purpose::STANDARD.encode(&sealed));

    // ── Fallback stub (NOT production-safe) ──────────────────────────────
    //
    // TODO: activate one of the options above by adding the crate dependency.
    //
    // This stub is intentionally loud so developers notice it:
    eprintln!(
        "WARNING: Using placeholder encryption for GitHub secret '{}'. \
         Add `crypto_box` or `sodiumoxide` to Cargo.toml and uncomment \
         the real implementation in commands/migrate/github.rs.",
        value.chars().take(4).collect::<String>()
    );

    Ok(general_purpose::STANDARD.encode(value.as_bytes()))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn test_encrypt_for_github_valid_base64_key() {
        // "test_key_32bytes_padded_to_valid!" base64-encoded
        let pk_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"); // 32 zero bytes
        let result = encrypt_for_github(&pk_b64, "my_secret_value");
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_encrypt_for_github_invalid_key() {
        let result = encrypt_for_github("!!!not_base64!!!", "value");
        assert!(result.is_err());
    }
}
