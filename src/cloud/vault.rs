//! Vault commands — `evnx vault …`.
//!
//! # How a vault key is protected
//!
//! Creating a vault generates a fresh random 256-bit [`VaultKey`] on this machine
//! and immediately wraps it. The unwrapped key is never transmitted and never
//! written to disk.
//!
//! The creator's own copy is wrapped **under the master key**, with
//! XChaCha20-Poly1305 keyed by an HKDF subkey of the Argon2id-derived master key.
//! No ECDH, so there is no ephemeral public key and the server stores `NULL` in
//! that column.
//!
//! That is a deliberate cryptographic choice, not a shortcut. A solo vault
//! protected this way is **post-quantum safe**: Argon2id and XChaCha20 are both
//! symmetric constructions, and Grover's algorithm merely halves their effective
//! key length, leaving 128 bits. Wrapping the creator's own key by ECDH instead
//! would put every vault behind X25519, which Shor's algorithm breaks outright —
//! making all of them vulnerable to harvest-now-decrypt-later. Sharing a vault
//! does accept that exposure for the shared copy, which is why
//! `evnx-crypto/docs/security-model.md` targets a hybrid ML-KEM wrap before team
//! sharing ships.
//!
//! [`VaultKey`]: evnx_crypto::VaultKey

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::auth;
use super::client::Client;
use super::config::CloudConfig;

/// Environments the server accepts. Checked here so a typo costs nothing.
pub const ENVIRONMENTS: [&str; 4] = ["production", "staging", "development", "test"];

#[derive(Serialize)]
struct CreateVaultRequest {
    name: String,
    environment: String,
    /// The creator's copy, wrapped under their master key, base64.
    encrypted_vault_key: String,
    // `eph_pub_key` is deliberately absent: the master-key wrap uses no ECDH, so
    // there is no ephemeral to send. The server's field is `Option`.
}

#[derive(Deserialize)]
struct CreateVaultResponse {
    vault_id: String,
    name: String,
    environment: String,
}

#[derive(Deserialize)]
struct VaultList {
    vaults: Vec<VaultSummary>,
}

/// Fetch the vault list and resolve one by name, `name/environment`, or id.
///
/// Shared with `evnx cloud push` / `pull`: both have to turn what a person typed
/// into the id the API uses, and must refuse the same ambiguities.
pub(crate) fn fetch_and_resolve(client: &Client, target: &str) -> Result<VaultRef> {
    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let v = resolve(&listed.vaults, target)?;
    Ok(VaultRef {
        id: v.id.clone(),
        name: v.name.clone(),
        environment: v.environment.clone(),
    })
}

/// The identity of one vault, as the sync commands need it.
#[derive(Debug, Clone)]
pub(crate) struct VaultRef {
    /// Canonical id. **This exact string goes into the AAD**, so push and pull
    /// must both take it from here and never re-format it.
    pub id: String,
    pub name: String,
    pub environment: String,
}

/// `Debug` is safe here: a summary carries an id, a name, an environment, a role
/// and counts — no key material and nothing derived from the master password.
#[derive(Deserialize, Clone, Debug)]
struct VaultSummary {
    id: String,
    name: String,
    environment: String,
    role: String,
    version_count: i64,
    updated_at: String,
}

/// Create a vault.
pub fn create(
    server_override: Option<&str>,
    name: String,
    environment: String,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{b64_encode, wrap_vault_key_with_master_key, VaultKey};

    validate_name(&name)?;
    let environment = validate_environment(&environment)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    // The master key is not cached anywhere, so every command that touches a
    // vault key asks for the password. Caching it on disk would defeat the point
    // of the server never holding one.
    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;

    let vault_key = VaultKey::generate();
    let wrapped = wrap_vault_key_with_master_key(&vault_key, &master_key)
        .map_err(|e| anyhow!("wrapping the vault key: {e}"))?;

    let created: CreateVaultResponse = client
        .post(
            "/api/v1/vaults",
            &CreateVaultRequest {
                name: name.clone(),
                environment: environment.clone(),
                encrypted_vault_key: b64_encode(&wrapped),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {} created {}/{}",
        "✓".green(),
        created.name,
        created.environment
    );
    if verbose {
        println!("  vault id  {}", created.vault_id);
        println!("  key wrap  master key (no ECDH, no ephemeral)");
    }
    println!();
    println!("  Push a .env into it with:  {}", "evnx cloud push".cyan());
    Ok(())
}

/// List the vaults this account can reach.
pub fn list(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;

    if listed.vaults.is_empty() {
        println!("  No vaults yet on {server}.");
        println!("  Create one with:  {}", "evnx vault create".cyan());
        return Ok(());
    }

    let name_w = listed
        .vaults
        .iter()
        .map(|v| v.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let env_w = listed
        .vaults
        .iter()
        .map(|v| v.environment.len())
        .max()
        .unwrap_or(11)
        .max(11);

    println!(
        "  {:<name_w$}  {:<env_w$}  {:<9}  {:>8}  {}",
        "NAME".bold(),
        "ENVIRONMENT".bold(),
        "ROLE".bold(),
        "VERSIONS".bold(),
        "UPDATED".bold()
    );
    for v in &listed.vaults {
        println!(
            "  {:<name_w$}  {:<env_w$}  {:<9}  {:>8}  {}",
            v.name,
            v.environment,
            v.role,
            v.version_count,
            // Trim the RFC 3339 timestamp to the date; the time is rarely what
            // anyone is scanning a list for.
            v.updated_at.split('T').next().unwrap_or(&v.updated_at)
        );
    }

    if verbose {
        println!();
        for v in &listed.vaults {
            println!(
                "  {} {}",
                v.id,
                format!("{}/{}", v.name, v.environment).dimmed()
            );
        }
    }
    Ok(())
}

/// Delete a vault. Owner only, and irreversible from the CLI.
pub fn delete(
    server_override: Option<&str>,
    target: String,
    assume_yes: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let vault = resolve(&listed.vaults, &target)?;

    if !assume_yes {
        println!(
            "  About to delete {}/{} and its {} version(s) from {server}.",
            vault.name.bold(),
            vault.environment,
            vault.version_count
        );
        println!("  {}", "There is no undelete in the CLI.".yellow());
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!("Delete {}/{}?", vault.name, vault.environment))
            .default(false)
            .interact()
            .context("reading the confirmation")?;
        if !ok {
            println!("  Cancelled; nothing was deleted.");
            return Ok(());
        }
    }

    client
        .delete(&format!("/api/v1/vaults/{}", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {} deleted {}/{}",
        "✓".green(),
        vault.name,
        vault.environment
    );
    if verbose {
        println!("  vault id  {}", vault.id);
    }
    Ok(())
}

/// Find the vault a user meant.
///
/// Accepts a UUID, a bare `name`, or `name/environment`. A bare name that exists
/// in more than one environment is an error listing the candidates rather than a
/// guess — picking one and deleting it would be the worst possible outcome.
fn resolve<'a>(vaults: &'a [VaultSummary], target: &str) -> Result<&'a VaultSummary> {
    if let Some(v) = vaults.iter().find(|v| v.id == target) {
        return Ok(v);
    }

    let (name, env) = match target.split_once('/') {
        Some((n, e)) => (n, Some(e)),
        None => (target, None),
    };

    let matches: Vec<&VaultSummary> = vaults
        .iter()
        .filter(|v| v.name == name && env.is_none_or(|e| v.environment == e))
        .collect();

    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(anyhow!(
            "no vault called {target:?}. Run `evnx vault list` to see what you have."
        )),
        many => Err(anyhow!(
            "{target:?} matches {} vaults: {}. Name the environment too, as {}.",
            many.len(),
            many.iter()
                .map(|v| format!("{}/{}", v.name, v.environment))
                .collect::<Vec<_>>()
                .join(", "),
            format!("{name}/<environment>").bold()
        )),
    }
}

pub(crate) fn require_session(client: &Client, server: &str) -> Result<()> {
    if client.is_signed_in() {
        return Ok(());
    }
    Err(anyhow!(
        "not signed in to {server}. Run `evnx auth login` first."
    ))
}

/// Reject a name the server's regex would reject, before a round trip.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(anyhow!("a vault name must be 1–64 characters"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(anyhow!(
            "a vault name may contain only letters, digits and hyphens (got {name:?})"
        ));
    }
    Ok(())
}

fn validate_environment(env: &str) -> Result<String> {
    let env = env.trim().to_lowercase();
    if ENVIRONMENTS.contains(&env.as_str()) {
        Ok(env)
    } else {
        Err(anyhow!(
            "environment must be one of: {} (got {env:?})",
            ENVIRONMENTS.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(name: &str, env: &str) -> VaultSummary {
        VaultSummary {
            id: format!("id-{name}-{env}"),
            name: name.into(),
            environment: env.into(),
            role: "owner".into(),
            version_count: 2,
            updated_at: "2026-09-14T10:00:00Z".into(),
        }
    }

    #[test]
    fn names_the_server_would_reject_are_caught_first() {
        for bad in [
            "",
            "has space",
            "under_score",
            "dot.dot",
            "UPPER OK but space",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["api-keys", "v1", "A-B-9"] {
            assert!(validate_name(good).is_ok(), "{good:?} should be accepted");
        }
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn environments_are_normalised_and_checked() {
        assert_eq!(validate_environment(" Production ").unwrap(), "production");
        assert!(validate_environment("prod").is_err());
        assert!(validate_environment("").is_err());
    }

    #[test]
    fn a_bare_name_resolves_when_it_is_unambiguous() {
        let vaults = [v("api", "production"), v("web", "staging")];
        assert_eq!(resolve(&vaults, "api").unwrap().environment, "production");
    }

    #[test]
    fn an_ambiguous_name_refuses_rather_than_guessing() {
        // Deleting the wrong environment because the CLI guessed would be the
        // worst outcome this command can produce.
        let vaults = [v("api", "production"), v("api", "staging")];
        let err = resolve(&vaults, "api").unwrap_err().to_string();
        assert!(err.contains("matches 2 vaults"), "{err}");
        assert!(
            err.contains("api/production") && err.contains("api/staging"),
            "{err}"
        );
    }

    #[test]
    fn name_slash_environment_disambiguates() {
        let vaults = [v("api", "production"), v("api", "staging")];
        assert_eq!(
            resolve(&vaults, "api/staging").unwrap().environment,
            "staging"
        );
    }

    #[test]
    fn an_id_resolves_directly() {
        let vaults = [v("api", "production")];
        assert_eq!(resolve(&vaults, "id-api-production").unwrap().name, "api");
    }

    #[test]
    fn an_unknown_name_points_at_the_list_command() {
        let vaults = [v("api", "production")];
        let err = resolve(&vaults, "nope").unwrap_err().to_string();
        assert!(err.contains("evnx vault list"), "{err}");
    }

    #[test]
    fn the_create_request_carries_no_ephemeral_key() {
        // The master-key wrap uses no ECDH. Sending an ephemeral would mean the
        // creator's copy went through X25519, which would cost solo vaults their
        // post-quantum safety.
        let body = CreateVaultRequest {
            name: "api".into(),
            environment: "production".into(),
            encrypted_vault_key: "d3JhcHBlZA==".into(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("eph_pub_key"), "{json}");
    }
}
