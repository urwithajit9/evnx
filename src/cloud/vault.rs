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

/// AES-GCM nonce length — the prefix every stored blob carries.
const NONCE_LEN: usize = 12;

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
///
/// The pair of optional fields distinguishes the two shapes: neither means your
/// own vault (sealed under your master key), both means a share (hybrid X25519 +
/// ML-KEM). One without the other is malformed.
#[derive(Deserialize)]
struct MyWrappedKey {
    encrypted_vault_key: String,
    #[serde(default)]
    eph_pub_key: Option<String>,
    #[serde(default)]
    mlkem_ciphertext: Option<String>,
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

// ─── Member management (Phase 3) ──────────────────────────────────────────────

#[derive(Deserialize)]
struct MemberList {
    members: Vec<MemberSummary>,
}

#[derive(Deserialize)]
struct MemberSummary {
    user_id: String,
    email: String,
    role: String,
    granted_at: String,
    #[serde(default)]
    has_mlkem_key: bool,
    #[serde(default)]
    is_you: bool,
}

/// Show who can reach a vault.
pub fn members(server_override: Option<&str>, target: String, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let vault = resolve(&listed.vaults, &target)?;

    let people: MemberList = client
        .get(&format!("/api/v1/vaults/{}/members", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {}/{} — {} member(s)",
        vault.name.bold(),
        vault.environment,
        people.members.len()
    );
    println!();

    for m in &people.members {
        let you = if m.is_you {
            " (you)".dimmed().to_string()
        } else {
            String::new()
        };
        println!("  {:<10} {}{}", m.role.cyan(), m.email, you);
        if verbose {
            println!("             id      {}", m.user_id);
            println!("             granted {}", m.granted_at);
        }
        // ⚠️ Worth surfacing without being asked: this member cannot be re-wrapped
        // to, so a revoke-and-rekey will refuse until they sign in once.
        if !m.has_mlkem_key {
            println!(
                "             {}",
                "no post-quantum key — they must sign in once before the vault can be re-keyed"
                    .yellow()
            );
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct SetRoleRequest {
    role: String,
}

/// Change a member's role.
pub fn set_role(
    server_override: Option<&str>,
    target: String,
    user_email: String,
    role: String,
    verbose: bool,
) -> Result<()> {
    let valid = ["viewer", "developer", "admin"];
    if !valid.contains(&role.as_str()) {
        return Err(anyhow!(
            "role must be one of: {}. Got `{role}`.\n\
             \x20 `owner` is not assignable — it is set once, by whoever created the vault.",
            valid.join(", ")
        ));
    }
    let user_email = user_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let vault = resolve(&listed.vaults, &target)?;

    let people: MemberList = client
        .get(&format!("/api/v1/vaults/{}/members", vault.id))
        .map_err(|e| anyhow!("{e}"))?;
    let member = people
        .members
        .iter()
        .find(|m| m.email == user_email)
        .ok_or_else(|| {
            anyhow!(
                "{user_email} is not a member of {}/{}.\n\
                 \x20 `evnx vault members {}` shows who is.",
                vault.name,
                vault.environment,
                target
            )
        })?;

    if member.role == role {
        println!(
            "  {user_email} is already {role} on {}/{}.",
            vault.name, vault.environment
        );
        return Ok(());
    }

    client
        .patch::<_, serde::de::IgnoredAny>(
            &format!("/api/v1/vaults/{}/members/{}", vault.id, member.user_id),
            &SetRoleRequest { role: role.clone() },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "  {} {user_email} is now {role} on {}/{}",
        "✓".green(),
        vault.name,
        vault.environment
    );
    if verbose {
        println!("  was {}", member.role);
    }
    Ok(())
}

// ─── Revocation with re-key ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct RevokeVersionList {
    versions: Vec<RevokeVersionSummary>,
}

#[derive(Deserialize)]
struct RevokeVersionSummary {
    version_num: i32,
}

#[derive(Serialize)]
struct StageBlobRequest {
    version_num: i32,
    nonce: String,
    ciphertext: String,
    blob_hash: String,
}

#[derive(Deserialize)]
struct StagedBlob {
    version_num: i32,
    blob_key: String,
    blob_size_bytes: i32,
}

#[derive(Serialize)]
struct RekeyedVersion {
    version_num: i32,
    blob_key: String,
    blob_hash: String,
    blob_size_bytes: i32,
}

#[derive(Serialize)]
struct RekeyedMember {
    user_id: String,
    encrypted_vault_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    eph_pub_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mlkem_ciphertext: Option<String>,
}

#[derive(Serialize)]
struct RekeyRequest {
    versions: Vec<RekeyedVersion>,
    members: Vec<RekeyedMember>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remove_user_id: Option<String>,
}

/// Remove a member and rotate the vault key.
///
/// ─── What this actually does ─────────────────────────────────────────────────
///
/// 1. Opens your copy of the current vault key.
/// 2. Generates a fresh one.
/// 3. Downloads **every** version, decrypts it, re-encrypts it under the new key
///    preserving that version's associated data, and stages it on the server.
/// 4. Wraps the new key for everyone who stays — under your master key for your
///    own copy, hybrid X25519 + ML-KEM for everyone else's.
/// 5. Sends one request that swaps all of it and removes the member, atomically.
///
/// Nothing changes until step 5. Abandoning halfway leaves staged blobs nothing
/// references, and a vault that still opens with the old key.
///
/// ─── ⚠️ The limit, stated plainly ────────────────────────────────────────────
///
/// Rotation stops the removed member reading anything pushed **from now on**. It
/// cannot recall copies of what they could already read. If they had access to a
/// secret, treat that secret as theirs and rotate it at its source — the same
/// advice that applies to a leaked API token, and the step people skip.
#[allow(clippy::too_many_arguments)]
pub fn revoke(
    server_override: Option<&str>,
    target: String,
    user_email: String,
    assume_yes: bool,
    no_rekey: bool,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{
        b64_encode, blob_hash, reencrypt_vault, unwrap_vault_key, unwrap_vault_key_with_master_key,
        vault_aad, wrap_vault_key_for_user, wrap_vault_key_with_master_key, EncryptedBlob,
        UserPublicKeys, VaultKey, WrappedVaultKey,
    };

    let user_email = user_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_session(&client, &server)?;

    let listed: VaultList = client.get("/api/v1/vaults").map_err(|e| anyhow!("{e}"))?;
    let vault = resolve(&listed.vaults, &target)?;
    let label = format!("{}/{}", vault.name, vault.environment);

    let people: MemberList = client
        .get(&format!("/api/v1/vaults/{}/members", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    let leaving = people
        .members
        .iter()
        .find(|m| m.email == user_email)
        .ok_or_else(|| {
            anyhow!(
                "{user_email} is not a member of {label}.\n\
                 \x20 `evnx vault members {target}` shows who is."
            )
        })?;

    if leaving.is_you {
        return Err(anyhow!(
            "to leave {label} yourself, ask an admin to remove you.\n\
             \x20 Revoking your own access would mean re-keying a vault you can no \n\
             \x20 longer open, which cannot work."
        ));
    }

    let remaining: Vec<&MemberSummary> = people
        .members
        .iter()
        .filter(|m| m.email != user_email)
        .collect();

    // ── Refuse early if anyone cannot be re-wrapped ──────────────────────────
    //
    // ⚠️ Checked BEFORE any work, and before the password prompt. A member with
    // no post-quantum key cannot receive the new vault key, and discovering that
    // after re-encrypting fifty versions would be a wasted operation ending in a
    // rejected swap.
    if !no_rekey {
        let stuck: Vec<&str> = remaining
            .iter()
            .filter(|m| !m.has_mlkem_key && !m.is_you)
            .map(|m| m.email.as_str())
            .collect();
        if !stuck.is_empty() {
            return Err(anyhow!(
                "these members have no post-quantum sharing key, so the vault key \n\
                 cannot be re-wrapped for them:\n\
                 \x20   {}\n\
                 \x20\n\
                 \x20 They each need to sign in once with evnx 0.5 or later, or at \n\
                 \x20 app.evnx.dev. Re-keying without them would lock them out of \n\
                 \x20 {label} entirely.",
                stuck.join("\n    ")
            ));
        }
    }

    // ── Confirm ──────────────────────────────────────────────────────────────
    if !assume_yes {
        println!("  About to remove {} from {label}.", user_email.bold());
        if no_rekey {
            println!(
                "  {}",
                "--no-rekey: the vault key is NOT rotated. They keep the ability to".yellow()
            );
            println!(
                "  {}",
                "decrypt every version they had access to, including future ones.".yellow()
            );
        } else {
            println!("  Every version will be re-encrypted under a fresh key and re-wrapped");
            println!("  for the {} remaining member(s).", remaining.len());
            println!();
            println!(
                "  {}",
                "This stops them reading anything pushed from now on. It cannot".yellow()
            );
            println!(
                "  {}",
                "recall copies of what they could already read — rotate those".yellow()
            );
            println!("  {}", "secrets at their source.".yellow());
        }
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!("Remove {user_email} from {label}?"))
            .default(false)
            .interact()
            .context("reading the confirmation")?;
        if !ok {
            println!("  Cancelled; nothing was changed.");
            return Ok(());
        }
    }

    // ── The simple path ──────────────────────────────────────────────────────
    if no_rekey {
        client
            .delete(&format!(
                "/api/v1/vaults/{}/members/{}",
                vault.id, leaving.user_id
            ))
            .map_err(|e| anyhow!("{e}"))?;
        println!("  {} removed {user_email} from {label}", "✓".green());
        println!(
            "  {}",
            "The vault key was NOT rotated. Rotate the secrets themselves.".yellow()
        );
        return Ok(());
    }

    // ── Open the current key ─────────────────────────────────────────────────
    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;

    let my_key: MyWrappedKey = client
        .get(&format!("/api/v1/vaults/{}/my-key", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    let old_key = match (&my_key.eph_pub_key, &my_key.mlkem_ciphertext) {
        (None, None) => {
            let wrapped =
                evnx_crypto::b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
                    .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;
            unwrap_vault_key_with_master_key(&wrapped, &master_key)
                .map_err(|_| anyhow!("could not open your key for {label} — wrong password?"))?
        }
        (Some(eph), Some(ct)) => {
            let sealed =
                evnx_crypto::EncryptedPrivateKey::from_base64(&fetch_sealed_private_key(&client)?)
                    .map_err(|e| {
                        anyhow!("the server sent an unusable encrypted_private_key: {e}")
                    })?;
            let keypair = evnx_crypto::decrypt_private_key(&sealed, &master_key)
                .map_err(|_| anyhow!("could not open your keypair — wrong password?"))?;
            let wrapped = WrappedVaultKey::from_base64(&my_key.encrypted_vault_key, eph, ct)
                .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;
            unwrap_vault_key(&wrapped, &keypair)
                .map_err(|_| anyhow!("could not open your key for {label}."))?
        }
        _ => {
            return Err(anyhow!(
                "the server sent a malformed key for {label} — half a wrap. \n\
                 \x20 Treat this server as untrusted."
            ))
        }
    };

    let new_key = VaultKey::generate();

    // ── Re-encrypt every version ─────────────────────────────────────────────
    let versions: RevokeVersionList = client
        .get(&format!("/api/v1/vaults/{}/versions", vault.id))
        .map_err(|e| anyhow!("{e}"))?;

    let total = versions.versions.len();
    println!("  Re-encrypting {total} version(s)…");

    let mut staged = Vec::with_capacity(total);
    for (i, v) in versions.versions.iter().enumerate() {
        let n = v.version_num;
        let blob_bytes = client
            .get_bytes(&format!("/api/v1/vaults/{}/versions/{n}/blob", vault.id))
            .map_err(|e| anyhow!("fetching version {n}: {e}"))?;

        if blob_bytes.len() <= NONCE_LEN {
            return Err(anyhow!("version {n} is too short to be a blob"));
        }
        let (nonce_bytes, ciphertext) = blob_bytes.split_at(NONCE_LEN);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(nonce_bytes);

        let old_blob = EncryptedBlob {
            nonce,
            ciphertext: ciphertext.to_vec(),
        };

        // ⚠️ The same AAD on both sides. `vault_aad` binds a blob to
        // (vault_id, version), so version `n` must stay version `n`'s.
        let aad = vault_aad(&vault.id, n as u32);
        let fresh = reencrypt_vault(&old_blob, &old_key, &new_key, &aad)
            .map_err(|_| anyhow!("could not re-encrypt version {n} — is your key current?"))?;

        let hash = blob_hash(&fresh.ciphertext);
        let out: StagedBlob = client
            .post(
                &format!("/api/v1/vaults/{}/rekey/blobs", vault.id),
                &StageBlobRequest {
                    version_num: n,
                    nonce: b64_encode(&fresh.nonce),
                    ciphertext: b64_encode(&fresh.ciphertext),
                    blob_hash: hash.clone(),
                },
            )
            .map_err(|e| anyhow!("staging version {n}: {e}"))?;

        staged.push(RekeyedVersion {
            version_num: out.version_num,
            blob_key: out.blob_key,
            blob_hash: hash,
            blob_size_bytes: out.blob_size_bytes,
        });

        if verbose {
            println!("    version {n} re-encrypted ({}/{})", i + 1, total);
        }
    }

    // ── Re-wrap for everyone who stays ───────────────────────────────────────
    println!("  Re-wrapping for {} member(s)…", remaining.len());

    let mut wraps = Vec::with_capacity(remaining.len());
    for m in &remaining {
        if m.is_you {
            // Our own copy stays sealed under the master key: symmetric, no key
            // agreement, and the shape a vault creator's copy has always had.
            let wrapped = wrap_vault_key_with_master_key(&new_key, &master_key)
                .map_err(|e| anyhow!("wrapping the new key for yourself: {e}"))?;
            wraps.push(RekeyedMember {
                user_id: m.user_id.clone(),
                encrypted_vault_key: b64_encode(&wrapped),
                eph_pub_key: None,
                mlkem_ciphertext: None,
            });
            continue;
        }

        let keys: RecipientKeys = client
            .get(&format!("/api/v1/users/{}/public-key", m.email))
            .map_err(|e| anyhow!("fetching {}'s public keys: {e}", m.email))?;
        let mlkem = keys
            .mlkem_public_key
            .ok_or_else(|| anyhow!("{} has no post-quantum sharing key", m.email))?;
        let recipient = UserPublicKeys::from_base64(&keys.x25519_public_key, &mlkem)
            .map_err(|e| anyhow!("unusable public key for {}: {e}", m.email))?;

        let wrapped = wrap_vault_key_for_user(&new_key, &recipient)
            .map_err(|e| anyhow!("wrapping the new key for {}: {e}", m.email))?;

        wraps.push(RekeyedMember {
            user_id: m.user_id.clone(),
            encrypted_vault_key: wrapped.encrypted_vault_key_base64(),
            eph_pub_key: Some(wrapped.eph_pub_key_base64()),
            mlkem_ciphertext: Some(wrapped.mlkem_ciphertext_base64()),
        });
    }

    // ── One atomic swap ──────────────────────────────────────────────────────
    client
        .post::<_, serde::de::IgnoredAny>(
            &format!("/api/v1/vaults/{}/rekey", vault.id),
            &RekeyRequest {
                versions: staged,
                members: wraps,
                remove_user_id: Some(leaving.user_id.clone()),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!(
        "  {} removed {user_email} from {label} and rotated the vault key",
        "✓".green()
    );
    println!(
        "  {total} version(s) re-encrypted, {} member(s) re-wrapped",
        remaining.len()
    );
    println!();
    println!(
        "  {}",
        "They can no longer read anything pushed from now on.".dimmed()
    );
    println!(
        "  {}",
        "They may still hold copies of what they could already read —".yellow()
    );
    println!("  {}", "rotate those secrets at their source.".yellow());

    Ok(())
}

/// The account's sealed Ed25519 seed, for opening a shared vault key.
fn fetch_sealed_private_key(client: &Client) -> Result<String> {
    #[derive(Deserialize)]
    struct MeSeed {
        encrypted_private_key: String,
    }
    let me: MeSeed = client
        .get("/api/v1/auth/me")
        .map_err(|e| anyhow!("fetching your sealed keypair: {e}"))?;
    Ok(me.encrypted_private_key)
}
