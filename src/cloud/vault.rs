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
use super::client::{ApiError, Client};
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
    // An id needs no lookup — and that is not merely an optimisation. A
    // vault-scoped API token is refused `GET /vaults` by design: enumerating
    // vaults is outside what it was issued for. So for CI, passing the id is the
    // only path that works at all.
    if looks_like_vault_id(target) {
        return Ok(VaultRef {
            id: target.to_string(),
            name: String::new(),
            environment: String::new(),
        });
    }

    let listed: VaultList = match client.get("/api/v1/vaults") {
        Ok(l) => l,
        Err(ApiError::Forbidden { .. }) if client.is_api_token() => {
            return Err(anyhow!(
                "a vault-scoped API token cannot list vaults, so {target:?} cannot be \
                 resolved by name.\n\
                 \x20 Pass the vault id instead:  --vault <uuid>\n\
                 \x20 Find it with `evnx vault list --verbose` from a normal login. \
                 The restriction is deliberate — a token issued for one vault should \
                 not be able to enumerate the others."
            ))
        }
        Err(e) => return Err(anyhow!("{e}")),
    };

    let v = resolve(&listed.vaults, target)?;
    Ok(VaultRef {
        id: v.id.clone(),
        name: v.name.clone(),
        environment: v.environment.clone(),
    })
}

/// Whether a string is a hyphenated UUID, and so already a vault id.
///
/// Deliberately strict: a loose check that accepted a vault *name* would send the
/// name straight to the API as an id and produce a 404 instead of the much more
/// useful "no vault called …".
pub(crate) fn looks_like_vault_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The identity of one vault, as the sync commands need it.
#[derive(Debug, Clone)]
pub(crate) struct VaultRef {
    /// Canonical id. **This exact string goes into the AAD**, so push and pull
    /// must both take it from here and never re-format it.
    pub id: String,
    /// Empty when the vault was addressed by id — a vault-scoped token cannot
    /// list vaults, so there is nothing to look the name up in.
    pub name: String,
    /// Empty for the same reason as `name`.
    pub environment: String,
}

impl VaultRef {
    /// How to refer to this vault in output: `name/environment` when known,
    /// otherwise the id.
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.name, self.environment)
        }
    }
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

/// What `GET /api/v1/users/{email}/public-key` answers.
#[derive(Deserialize)]
struct RecipientKeys {
    x25519_public_key: String,
    /// `None` for an account created before post-quantum wrapping existed.
    #[serde(default)]
    mlkem_public_key: Option<String>,
}

#[derive(Serialize)]
struct AddMemberRequest {
    user_email: String,
    role: String,
    encrypted_vault_key: String,
    eph_pub_key: String,
    mlkem_ciphertext: String,
}

/// Share a vault with another evnx account.
///
/// ─── What the server does and does not learn ─────────────────────────────────
///
/// It hands over the recipient's public keys and stores the result. It never sees
/// the vault key, and it cannot grant access itself — the wrap happens here,
/// under keys only the recipient can reverse.
///
/// ─── Why this needs the master password ──────────────────────────────────────
///
/// Your own copy of the vault key is sealed under your master key. Re-wrapping it
/// for someone else means opening it first, and there is no cached copy anywhere
/// for the same reason the server holds none.
///
/// ─── The trust boundary worth naming ─────────────────────────────────────────
///
/// ⚠️ The public keys come from the server. A malicious server could substitute
/// its own and read what you share. The protocol has no third party to check them
/// against, so this is a genuine limit rather than an oversight: out-of-band
/// fingerprint verification is the answer, and it is not built yet. Sharing is
/// still worth doing — the server would have to actively attack you rather than
/// merely be breached — but "the server cannot read your secrets" becomes "the
/// server cannot read your secrets passively" the moment you share.
pub fn share(
    server_override: Option<&str>,
    target: String,
    recipient_email: String,
    role: String,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{unwrap_vault_key_with_master_key, wrap_vault_key_for_user, UserPublicKeys};

    let valid_roles = ["viewer", "developer", "admin"];
    if !valid_roles.contains(&role.as_str()) {
        return Err(anyhow!(
            "role must be one of: {}. Got `{role}`.",
            valid_roles.join(", ")
        ));
    }

    let recipient_email = recipient_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let vault = resolve(&listed.vaults, &target)?;

    // ── The recipient's keys, before asking for a password ───────────────────
    //
    // Deliberately first. If the recipient cannot be shared with, saying so
    // before prompting for a master password is better than after.
    let keys: RecipientKeys = client
        .get(&format!("/api/v1/users/{recipient_email}/public-key"))
        .map_err(|e| match e {
            ApiError::NotFound { .. } => anyhow!(
                "no evnx account for {recipient_email}.\n\
                 \x20 They need to register first — the vault key is wrapped to their \n\
                 \x20 public key, so there has to be one."
            ),
            other => anyhow!("{other}"),
        })?;

    // ⚠️ No fallback. An X25519-only wrap would be one Shor opens, and it stays
    // that way for as long as the row exists — an adversary recording it does not
    // care that a later version fixed the algorithm.
    let mlkem_public_key = keys.mlkem_public_key.ok_or_else(|| {
        anyhow!(
            "{recipient_email} has no post-quantum sharing key yet.\n\
             \x20 They need to sign in once with evnx 0.5 or later, or at \n\
             \x20 app.evnx.dev, which registers it automatically.\n\
             \x20\n\
             \x20 Sharing without it would wrap the vault key under X25519 alone, \n\
             \x20 which a quantum computer breaks — including from a recording made \n\
             \x20 today."
        )
    })?;

    let recipient = UserPublicKeys::from_base64(&keys.x25519_public_key, &mlkem_public_key)
        .map_err(|e| {
            anyhow!("the server sent an unusable public key for {recipient_email}: {e}")
        })?;

    // ── Open our copy, re-wrap for them ──────────────────────────────────────
    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;

    let my_key: MyWrappedKey = client
        .get(&format!("/api/v1/vaults/{}/my-key", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    if my_key.eph_pub_key.is_some() {
        // Re-sharing someone else's vault would need the account keypair rather
        // than the master key. The server also restricts sharing to owners and
        // admins, so this is mostly belt and braces.
        return Err(anyhow!(
            "{}/{} was shared with you rather than created by you, and re-sharing \n\
             is not supported. Ask the owner to share it directly.",
            vault.name,
            vault.environment
        ));
    }

    let wrapped_for_me =
        evnx_crypto::b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
            .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;
    let vault_key =
        unwrap_vault_key_with_master_key(&wrapped_for_me, &master_key).map_err(|_| {
            anyhow!(
                "could not open the key for {}/{} — the master password is probably wrong.",
                vault.name,
                vault.environment
            )
        })?;

    let wrapped = wrap_vault_key_for_user(&vault_key, &recipient)
        .map_err(|e| anyhow!("wrapping the vault key for {recipient_email}: {e}"))?;

    client
        .post::<_, serde::de::IgnoredAny>(
            &format!("/api/v1/vaults/{}/members", vault.id),
            &AddMemberRequest {
                user_email: recipient_email.clone(),
                role: role.clone(),
                encrypted_vault_key: wrapped.encrypted_vault_key_base64(),
                eph_pub_key: wrapped.eph_pub_key_base64(),
                mlkem_ciphertext: wrapped.mlkem_ciphertext_base64(),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {} shared {}/{} with {recipient_email} as {role}",
        "✓".green(),
        vault.name,
        vault.environment
    );
    if verbose {
        println!("  vault id  {}", vault.id);
        println!("  key wrap  hybrid X25519 + ML-KEM-768");
        println!(
            "  ml-kem ct {} chars",
            wrapped.mlkem_ciphertext_base64().len()
        );
    }
    println!();
    println!(
        "  They can pull it with:  {}",
        format!(
            "evnx cloud pull --vault {}/{}",
            vault.name, vault.environment
        )
        .cyan()
    );
    Ok(())
}

/// This account's wrapped copy of a vault key.
#[derive(Deserialize)]
struct MyWrappedKey {
    encrypted_vault_key: String,
    #[serde(default)]
    eph_pub_key: Option<String>,
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
    fn vault_ids_are_recognised_strictly() {
        // This decides whether the CLI skips `GET /vaults`. A vault-scoped API
        // token is refused that route, so the fast path is the only one that
        // works in CI — but a loose check would send a vault *name* to the API
        // as an id and turn a helpful "no vault called …" into a bare 404.
        assert!(looks_like_vault_id("6c26f157-a81c-414c-b9b2-cdb27d3f7606"));
        for not_an_id in [
            "app/production",
            "app",
            "",
            "6c26f157a81c414cb9b2cdb27d3f7606",    // unhyphenated
            "6c26f157-a81c-414c-b9b2-cdb27d3f760", // too short
            "6c26f157-a81c-414c-b9b2-cdb27d3f76060", // too long
            "6c26f157-a81c-414c-b9b2-cdb27d3f760g", // not hex
            "6c26f157_a81c_414c_b9b2_cdb27d3f7606", // wrong separator
        ] {
            assert!(!looks_like_vault_id(not_an_id), "{not_an_id:?}");
        }
    }

    #[test]
    fn a_vault_addressed_by_id_labels_itself_with_the_id() {
        let by_id = VaultRef {
            id: "6c26f157-a81c-414c-b9b2-cdb27d3f7606".into(),
            name: String::new(),
            environment: String::new(),
        };
        assert_eq!(by_id.label(), by_id.id);

        let named = VaultRef {
            id: "x".into(),
            name: "app".into(),
            environment: "production".into(),
        };
        assert_eq!(named.label(), "app/production");
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
