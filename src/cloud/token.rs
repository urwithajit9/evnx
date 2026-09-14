//! API tokens — `evnx auth token …`, the CI/CD credential.
//!
//! # What a token can and cannot do
//!
//! A token authenticates API requests. It does **not** decrypt anything. The vault
//! key is wrapped under the master key, so a pipeline that pulls needs the token
//! *and* the master password:
//!
//! ```text
//! echo "$EVNX_PASSWORD" | evnx cloud pull --vault app/production --password-stdin
//! # with EVNX_TOKEN=evnx_tok_... in the environment
//! ```
//!
//! That is the honest shape of zero-knowledge sync, and it is worth being blunt
//! about: **the master password in CI is the weak point**, not the token. What the
//! token buys is blast radius. Scoped to one vault and `read`, it can fetch that
//! vault's wrapped key and blobs and nothing else — the server refuses every other
//! vault and every mutating method, centrally, before a handler sees the request.
//! Without that scoping, a leaked runner would expose every vault the account has.
//!
//! Prefer piping the password over exporting it: an environment variable is
//! visible in process listings and tends to end up in logs.
//!
//! # Tokens cannot manage the account
//!
//! Minting, listing and revoking all require a real login, and so does anything
//! under `/auth/totp/*`. A leaked token therefore cannot issue itself a wider,
//! longer-lived replacement that would survive revoking the original, and cannot
//! enrol its own authenticator. The server enforces this with 403; this module
//! says so before making the request, so the reason is legible.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::client::{Client, ENV_TOKEN};
use super::config::CloudConfig;
use super::vault;

/// Scopes the server accepts.
pub const SCOPES: [&str; 2] = ["read", "read_write"];

#[derive(Serialize)]
struct CreateTokenRequest {
    name: String,
    scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vault_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_days: Option<i64>,
}

#[derive(Deserialize)]
struct CreateTokenResponse {
    id: String,
    /// Shown once and never retrievable again.
    raw_token: String,
    expires_at: Option<String>,
}

#[derive(Deserialize)]
struct TokenList {
    tokens: Vec<TokenSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct TokenSummary {
    id: String,
    name: String,
    scope: String,
    vault_id: Option<String>,
    expires_at: Option<String>,
    created_at: String,
    last_used_at: Option<String>,
}

/// Mint a token.
pub fn create(
    server_override: Option<&str>,
    name: String,
    scope: String,
    vault_target: Option<String>,
    expires_in_days: Option<i64>,
    verbose: bool,
) -> Result<()> {
    let scope = validate_scope(&scope)?;
    if name.trim().is_empty() {
        return Err(anyhow!(
            "a token needs a name — it is how you recognise it later"
        ));
    }

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    // Resolve the vault before minting, so a typo does not produce a token that
    // silently reaches nothing.
    let vault_ref = match &vault_target {
        Some(t) => Some(vault::fetch_and_resolve(&client, t)?),
        None => None,
    };

    let created: CreateTokenResponse = client
        .post(
            "/api/v1/auth/tokens",
            &CreateTokenRequest {
                name: name.trim().to_string(),
                scope: scope.clone(),
                vault_id: vault_ref.as_ref().map(|v| v.id.clone()),
                expires_in_days,
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} created token {}", "✓".green(), name.trim().bold());
    println!();
    println!("  {}", created.raw_token.bold());
    println!();
    println!(
        "  {} this is the only time it is shown. It is stored hashed, so it",
        "Save it now:".yellow().bold()
    );
    println!("  cannot be recovered — only revoked and replaced.");
    println!();
    println!("  scope     {scope}");
    match &vault_ref {
        Some(v) => println!("  vault     {}/{} only", v.name, v.environment),
        None => println!(
            "  vault     {} — reaches every vault on the account",
            "unscoped".yellow()
        ),
    }
    match &created.expires_at {
        Some(e) => println!("  expires   {}", e.split('T').next().unwrap_or(e)),
        None => println!("  expires   {}", "never".yellow()),
    }
    if verbose {
        println!("  token id  {}", created.id);
    }
    println!();
    println!("  Use it in CI:");
    println!("    export {ENV_TOKEN}={}", "evnx_tok_…".dimmed());
    println!(
        "    echo \"$EVNX_PASSWORD\" | evnx cloud pull --vault {} --password-stdin",
        vault_ref
            .as_ref()
            // The **id**, not the name. A vault-scoped token is refused
            // `GET /vaults`, so it cannot resolve a name — printing the friendly
            // spelling here would hand out a command that always fails.
            .map(|v| v.id.clone())
            .unwrap_or_else(|| "<vault>".into())
    );
    if vault_ref.is_some() {
        println!(
            "    {}",
            "(the id, not the name — a scoped token may not list vaults)".dimmed()
        );
    }
    println!();
    println!("  The token authenticates; it does not decrypt. The master password");
    println!("  is still required, so treat it as the more sensitive of the two.");
    Ok(())
}

/// List the account's live tokens.
pub fn list(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: TokenList = client
        .get("/api/v1/auth/tokens")
        .map_err(|e| anyhow!("{e}"))?;

    if listed.tokens.is_empty() {
        println!("  No API tokens on {server}.");
        println!(
            "  Create one with:  {}",
            "evnx auth token create <name>".cyan()
        );
        return Ok(());
    }

    // Map vault ids back to names. One extra request, and worth it — a bare UUID
    // in this listing tells nobody what the token can reach.
    let vault_names: std::collections::HashMap<String, String> = client
        .get::<serde_json::Value>("/api/v1/vaults")
        .ok()
        .and_then(|v| v.get("vaults").cloned())
        .and_then(|v| serde_json::from_value::<Vec<serde_json::Value>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            Some((
                v.get("id")?.as_str()?.to_string(),
                format!(
                    "{}/{}",
                    v.get("name")?.as_str()?,
                    v.get("environment")?.as_str()?
                ),
            ))
        })
        .collect();

    let name_w = listed
        .tokens
        .iter()
        .map(|t| t.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!(
        "  {:<name_w$}  {:<11}  {:<22}  {:<10}  {}",
        "NAME".bold(),
        "SCOPE".bold(),
        "VAULT".bold(),
        "EXPIRES".bold(),
        "LAST USED".bold()
    );
    for t in &listed.tokens {
        let vault = match &t.vault_id {
            Some(id) => vault_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.chars().take(8).collect()),
            None => "all vaults".to_string(),
        };
        println!(
            "  {:<name_w$}  {:<11}  {:<22}  {:<10}  {}",
            t.name,
            t.scope,
            vault,
            t.expires_at
                .as_deref()
                .map(short_date)
                .unwrap_or_else(|| "never".into()),
            t.last_used_at
                .as_deref()
                .map(short_date)
                .unwrap_or_else(|| "never".into()),
        );
    }

    if verbose {
        println!();
        for t in &listed.tokens {
            println!(
                "  {}  {} (created {})",
                t.id,
                t.name,
                short_date(&t.created_at)
            );
        }
    }
    Ok(())
}

/// Revoke a token by name or id.
pub fn revoke(
    server_override: Option<&str>,
    target: String,
    assume_yes: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: TokenList = client
        .get("/api/v1/auth/tokens")
        .map_err(|e| anyhow!("{e}"))?;
    let token = resolve(&listed.tokens, &target)?;

    if !assume_yes {
        println!(
            "  About to revoke {} ({}, {}).",
            token.name.bold(),
            token.scope,
            token
                .vault_id
                .as_ref()
                .map(|_| "vault-scoped")
                .unwrap_or("all vaults")
        );
        println!(
            "  {}",
            "Anything using it stops working immediately.".yellow()
        );
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!("Revoke {}?", token.name))
            .default(false)
            .interact()
            .context("reading the confirmation")?;
        if !ok {
            println!("  Cancelled; nothing was revoked.");
            return Ok(());
        }
    }

    client
        .delete(&format!("/api/v1/auth/tokens/{}", token.id))
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} revoked {}", "✓".green(), token.name);
    if verbose {
        println!("  token id  {}", token.id);
    }
    Ok(())
}

/// Find the token a person meant — by name, or by id.
fn resolve<'a>(tokens: &'a [TokenSummary], target: &str) -> Result<&'a TokenSummary> {
    if let Some(t) = tokens.iter().find(|t| t.id == target) {
        return Ok(t);
    }
    let matches: Vec<&TokenSummary> = tokens.iter().filter(|t| t.name == target).collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(anyhow!(
            "no token called {target:?}. Run `evnx auth token list` to see them."
        )),
        many => Err(anyhow!(
            "{} tokens are called {target:?}. Revoke by id instead: {}",
            many.len(),
            many.iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Token management needs a real login, never a token.
fn require_login(client: &Client, server: &str) -> Result<()> {
    if client.is_api_token() {
        return Err(anyhow!(
            "an API token cannot manage API tokens — that is deliberate.\n\
             \x20 A leaked token must not be able to mint a wider, longer-lived \
             replacement that survives revoking the original.\n\
             \x20 Unset {ENV_TOKEN} and run `evnx auth login`."
        ));
    }
    if !client.is_signed_in() {
        return Err(anyhow!(
            "not signed in to {server}. Run `evnx auth login` first."
        ));
    }
    Ok(())
}

fn validate_scope(scope: &str) -> Result<String> {
    let s = scope.trim().to_lowercase();
    if SCOPES.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(anyhow!(
            "scope must be one of: {} (got {scope:?})",
            SCOPES.join(", ")
        ))
    }
}

/// Trim an RFC 3339 timestamp to its date.
fn short_date(ts: &str) -> String {
    ts.split('T').next().unwrap_or(ts).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, id: &str) -> TokenSummary {
        TokenSummary {
            id: id.into(),
            name: name.into(),
            scope: "read".into(),
            vault_id: None,
            expires_at: None,
            created_at: "2026-09-14T10:00:00Z".into(),
            last_used_at: None,
        }
    }

    #[test]
    fn scopes_are_normalised_and_checked() {
        assert_eq!(validate_scope(" Read ").unwrap(), "read");
        assert_eq!(validate_scope("READ_WRITE").unwrap(), "read_write");
        assert!(validate_scope("write").is_err());
        assert!(validate_scope("admin").is_err());
    }

    #[test]
    fn a_token_resolves_by_name_or_id() {
        let tokens = [t("ci", "id-1"), t("deploy", "id-2")];
        assert_eq!(resolve(&tokens, "ci").unwrap().id, "id-1");
        assert_eq!(resolve(&tokens, "id-2").unwrap().name, "deploy");
    }

    #[test]
    fn duplicate_names_refuse_and_list_the_ids() {
        // The server does not enforce unique names, so two tokens can share one.
        // Revoking the wrong one would break a running pipeline.
        let tokens = [t("ci", "id-1"), t("ci", "id-2")];
        let err = resolve(&tokens, "ci").unwrap_err().to_string();
        assert!(err.contains("2 tokens"), "{err}");
        assert!(err.contains("id-1") && err.contains("id-2"), "{err}");
    }

    #[test]
    fn an_unknown_token_points_at_the_list_command() {
        let err = resolve(&[t("ci", "id-1")], "nope").unwrap_err().to_string();
        assert!(err.contains("evnx auth token list"), "{err}");
    }

    #[test]
    fn dates_are_trimmed_for_display() {
        assert_eq!(short_date("2026-09-14T10:00:00Z"), "2026-09-14");
        assert_eq!(short_date("never"), "never");
    }

    #[test]
    fn an_unscoped_request_omits_the_optional_fields() {
        // The server treats a missing vault_id as unscoped and a missing
        // expires_in_days as no expiry; sending explicit nulls would be a
        // different shape to validate.
        let body = CreateTokenRequest {
            name: "ci".into(),
            scope: "read".into(),
            vault_id: None,
            expires_in_days: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("vault_id"), "{json}");
        assert!(!json.contains("expires_in_days"), "{json}");
    }
}
