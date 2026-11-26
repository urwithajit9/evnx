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

use crate::cloud::client::api_path;
use crate::core::verdict::Verdict;
use anyhow::{Context, Result, anyhow};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::auth;
use super::client::{ApiError, Client};
use super::config::CloudConfig;
use super::role::{VAULT_OWNER, VAULT_ROLES, parse_role};

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
        // C10: lower case. The id goes into the AAD verbatim, and the server's
        // is lower case, so an id typed in upper case sealed a version that a
        // pull by name (which takes the server's spelling) could not open.
        return Ok(VaultRef {
            id: target.to_ascii_lowercase(),
            name: String::new(),
            environment: String::new(),
        });
    }

    let v = list_and_pick(client, target)?;
    Ok(VaultRef {
        id: v.id,
        name: v.name,
        environment: v.environment,
    })
}

/// `GET /vaults`: every vault this account can reach.
fn list_vaults(client: &Client) -> std::result::Result<Vec<VaultSummary>, ApiError> {
    client.get::<VaultList>("/api/v1/vaults").map(|l| l.vaults)
}

/// List the vaults and pick the one `target` names: a name, `name/environment`,
/// or an id among them.
///
/// The one place a target is looked up in the list (A-301). `vault members` and
/// six other copies used to list and pick themselves, so a refused list came back
/// bare, and an id was looked up although it needed no lookup (I-309).
fn list_and_pick(client: &Client, target: &str) -> Result<VaultSummary> {
    let vaults = match list_vaults(client) {
        Ok(v) => v,
        Err(ApiError::Forbidden { .. }) if client.is_api_token() => {
            return Err(anyhow!(
                "a vault-scoped API token cannot list vaults, so {target:?} cannot be \
                 resolved by name.\n\
                 \x20 Pass the vault's id in place of its name.\n\
                 \x20 Find it with `evnx vault list --verbose` from a normal login. \
                 The restriction is deliberate — a token issued for one vault should \
                 not be able to enumerate the others."
            ));
        }
        Err(e) => return Err(anyhow::Error::from(e)),
    };
    resolve(&vaults, target).cloned()
}

/// The name and environment of the vault with `id`, from `GET /vaults`.
///
/// For where a person will read the result (`cloud link`, I-221). An error if
/// the id is not among the caller's vaults or the list cannot be fetched — a
/// vault-scoped token cannot list, and then the name is not knowable.
pub(crate) fn resolve_id(client: &Client, id: &str) -> Result<VaultRef> {
    let vaults = list_vaults(client).with_context(|| format!("could not look up vault {id}"))?;
    let v = find_id(&vaults, id)
        .cloned()
        .ok_or_else(|| anyhow!("no vault {id} among yours. Run `evnx vault list` to see them."))?;
    Ok(VaultRef {
        id: v.id,
        name: v.name,
        environment: v.environment,
    })
}

/// The vault with `id`, whatever case it was typed in (C10).
fn find_id<'a>(vaults: &'a [VaultSummary], id: &str) -> Option<&'a VaultSummary> {
    vaults.iter().find(|v| v.id.eq_ignore_ascii_case(id))
}

/// Every member's email by user id, for naming who did what (I-319). Empty when
/// the list cannot be read; a caller then shows the id.
pub(crate) fn member_emails(
    client: &Client,
    vault_id: &str,
) -> std::collections::HashMap<String, String> {
    api_path(&["vaults", vault_id, "members"])
        .ok()
        .and_then(|p| client.get::<MemberList>(&p).ok())
        .map(|l| {
            l.members
                .into_iter()
                .map(|m| (m.user_id, m.email))
                .collect()
        })
        .unwrap_or_default()
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
#[derive(Deserialize, Serialize, Clone, Debug)]
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
    use evnx_crypto::{VaultKey, b64_encode, wrap_vault_key_with_master_key};

    validate_name(&name)?;
    let environment = validate_environment(&environment)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;

    // The master key is not cached anywhere, so every command that touches a
    // vault key asks for the password. Caching it on disk would defeat the point
    // of the server never holding one.
    let password = auth::read_master_password(password_stdin)?;
    let unlocked = auth::unlock(&client, &password)?;

    let vault_key = VaultKey::generate();
    let wrapped = wrap_vault_key_with_master_key(&vault_key, &unlocked.master_key)
        .map_err(|e| anyhow!("wrapping the vault key: {e}"))?;

    let created: CreateVaultResponse = client.post(
        "/api/v1/vaults",
        &CreateVaultRequest {
            name: name.clone(),
            environment: environment.clone(),
            encrypted_vault_key: b64_encode(&wrapped),
        },
    )?;

    crate::say!(
        "  {} created {}/{}",
        "✓".green(),
        created.name,
        created.environment
    );
    if verbose {
        println!("  vault id  {}", created.vault_id);
        println!("  key wrap  master key (no ECDH, no ephemeral)");
    }
    crate::say!();
    // I-320, F7: `create` binds no directory, so a bare `evnx cloud push` fails
    // right after it. Say both ways once, each with its condition, so the line
    // is still true when `cloud link` is run next (which then says push and
    // pull work here without --vault).
    let label = format!("{}/{}", created.name, created.environment);
    crate::say!(
        "  Next:  {}   (or bind this folder once with  {}  and a bare  {}  works)",
        format!("evnx cloud push --vault {label}").cyan(),
        format!("evnx cloud link --vault {label}").cyan(),
        "evnx cloud push".cyan()
    );
    Ok(())
}

/// List the vaults this account can reach.
pub fn list(server_override: Option<&str>, json: bool, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;

    let listed: VaultList = client.get("/api/v1/vaults")?;
    if json {
        return crate::core::json::emit(&serde_json::json!({ "vaults": listed.vaults }), 1, None);
    }

    if listed.vaults.is_empty() {
        println!("  No vaults yet on {server}.");
        crate::say!("  Create one with:  {}", "evnx vault create".cyan());
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
    client.require_user_session(
        "delete a vault",
        "\x20 Deleting a vault is for a signed-in member, not a pipeline — a leaked\n\x20 CI token must not be able to destroy it.\n",
    )?;

    // With `--yes` there is no question to catch a wrong vault, so the target
    // itself must say which one: the full `name/environment`, or an id.
    if assume_yes && !looks_like_vault_id(&target) && !target.contains('/') {
        return Err(crate::core::verdict::usage(format!(
            "pass the full name/environment with --yes, so a short name cannot delete \
             the wrong vault.\n\n\x20 evnx vault delete {target}/<environment> --yes"
        )));
    }

    let vault = list_and_pick(&client, &target)?;
    let full = format!("{}/{}", vault.name, vault.environment);

    if !assume_yes {
        println!(
            "  About to delete {} and its {} version(s) from {server}.",
            full.bold(),
            vault.version_count
        );
        println!("  {}", "There is no undelete in the CLI.".yellow());
        let ok =
            crate::utils::prompt::confirm_typed(&full, &format!("evnx vault delete {full} --yes"))?;
        if !ok {
            println!("  Cancelled; nothing was deleted.");
            return Err(crate::utils::prompt::Declined.into());
        }
    }

    client.delete(&api_path(&["vaults", &vault.id.to_string()])?)?;
    super::highwater::forget(&vault.id.to_string());

    crate::say!(
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
    use evnx_crypto::{UserPublicKeys, wrap_vault_key_for_user};

    // Case-insensitive, like `--env` (I-223, DEC-CL8): `--role Admin` was refused.
    let role = parse_role(&role, VAULT_ROLES, VAULT_OWNER)?;

    let recipient_email = recipient_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "share a vault",
        "\x20 Sharing decides who can read these secrets; a leaked CI token must\n\x20 not be able to add a reader.\n",
    )?;

    let vault = list_and_pick(&client, &target)?;
    let label = format!("{}/{}", vault.name, vault.environment);

    // ── Already a member? Before the key lookup and the password ─────────────
    //
    // ⚠️ A share to a member rewrote their row, role and key (I-313, I-314):
    // sharing with your own address turned an owner into a developer who could
    // no longer delete the vault, and an admin "sharing" with the owner was the
    // first step to removing them. The server refuses both since SV-3; refusing
    // here spares the password prompt and says what would work instead.
    let people: MemberList = client.get(&api_path(&["vaults", &vault.id, "members"])?)?;
    if let Some(m) = people
        .members
        .iter()
        .find(|m| m.email.to_lowercase() == recipient_email)
    {
        if m.is_you {
            return Err(anyhow!(
                "That is you: you already hold {label} as {}. Sharing cannot change \
                 your own role.",
                m.role
            ));
        }
        if m.role == "owner" {
            return Err(anyhow!(
                "{} owns {label}. Sharing cannot change the owner's role or key.",
                m.email
            ));
        }
        // The role they already hold: nothing to do, as `vault role` says for
        // the same case — a script that shares twice stays green.
        if m.role == role {
            crate::say!(
                "  {} is already {} {role} on {label} — nothing changed.",
                m.email,
                article(&role)
            );
            return Ok(());
        }
        return Err(anyhow!(
            "{} is already {} {} on {label}.\n\
             \x20 To make them {role}:  evnx vault role {target} --user {} --role {role}",
            m.email,
            article(&m.role),
            m.role,
            m.email
        ));
    }

    // ── May you share at that role? ──────────────────────────────────────────
    //
    // The server lets an owner or an admin share, and only a role below the
    // sharer's own (I-311, DEC-VL5). A developer or viewer heard "re-sharing is
    // not supported" after typing the password; say what it is, before.
    if let Some(me) = people.members.iter().find(|m| m.is_you) {
        if me.role != "owner" && me.role != "admin" {
            return Err(anyhow!(
                "sharing {label} needs the admin or owner role; you are {} {} on it.\n\
                 \x20 Ask its owner, or an admin, to share it.",
                article(&me.role),
                me.role
            ));
        }
        if me.role == "admin" && role == "admin" {
            return Err(anyhow!(
                "an admin can share {label} as viewer or developer; only the owner can \
                 make someone an admin."
            ));
        }
    }

    // ── The recipient's keys, before asking for a password ───────────────────
    //
    // Deliberately first. If the recipient cannot be shared with, saying so
    // before prompting for a master password is better than after.
    let keys: RecipientKeys = client
        .get(&api_path(&[
            "users",
            &recipient_email.to_string(),
            "public-key",
        ])?)
        .map_err(|e| match e {
            ApiError::NotFound { .. } => anyhow!(
                "no evnx account for {recipient_email}.\n\
                 \x20 They need to register first — the vault key is wrapped to their \n\
                 \x20 public key, so there has to be one."
            ),
            other => anyhow::Error::from(other),
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
    let password = auth::read_master_password(password_stdin)?;
    let unlocked = auth::unlock(&client, &password)?;

    // Your own copy, or — for an admin — the one shared to you, opened with the
    // keypair `unlock` holds (I-311): one opener for push, pull and share.
    let vault_ref = VaultRef {
        id: vault.id.clone(),
        name: vault.name.clone(),
        environment: vault.environment.clone(),
    };
    let vault_key = super::sync::unwrap_vault_key(&client, &vault_ref, &unlocked)?;

    let wrapped = wrap_vault_key_for_user(&vault_key, &recipient)
        .map_err(|e| anyhow!("wrapping the vault key for {recipient_email}: {e}"))?;

    client.post::<_, serde::de::IgnoredAny>(
        &api_path(&["vaults", &vault.id.to_string(), "members"])?,
        &AddMemberRequest {
            user_email: recipient_email.clone(),
            role: role.clone(),
            encrypted_vault_key: wrapped.encrypted_vault_key_base64(),
            eph_pub_key: wrapped.eph_pub_key_base64(),
            mlkem_ciphertext: wrapped.mlkem_ciphertext_base64(),
        },
    )?;

    crate::say!(
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
    if role != "admin" {
        let next = if role == "viewer" {
            "developer"
        } else {
            "admin"
        };
        crate::say!(
            "  Role: {role}. To raise it:  evnx vault role {target} --user {recipient_email} --role {next}"
        );
    }
    crate::say!();
    crate::say!(
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

/// Leave a vault: delete your own membership (I-312, DEC-VL6).
///
/// The server has always allowed it — "leaving on your own account" — and
/// `vault revoke --user <yourself>` told people to ask an admin instead.
pub fn leave(
    server_override: Option<&str>,
    target: String,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "leave a vault",
        "\x20 Membership is changed by a signed-in person; a leaked CI token must\n\x20 not be able to remove anyone, its owner included.\n",
    )?;

    let vault = list_and_pick(&client, &target)?;
    let label = format!("{}/{}", vault.name, vault.environment);
    let people: MemberList = client.get(&api_path(&["vaults", &vault.id, "members"])?)?;
    let me = people
        .members
        .iter()
        .find(|m| m.is_you)
        .ok_or_else(|| anyhow!("you are not a member of {label}."))?;

    // A vault with no owner has nobody who can delete it; the server refuses.
    if me.role == "owner" {
        return Err(anyhow!(
            "you own {label}, and the owner cannot leave it.\n\
             \x20 To be rid of it, delete it:  evnx vault delete {target}"
        ));
    }

    if !yes {
        println!("  About to leave {} ({}).", label.bold(), me.role);
        println!(
            "  {}",
            "Your copy of its key is deleted from the server. The vault key is not \
             rotated."
                .dimmed()
        );
    }
    if !crate::utils::prompt::confirm(
        &format!("Leave {label}?"),
        yes,
        &format!("evnx vault leave {target} --yes"),
    )? {
        println!("  Cancelled; nothing was changed.");
        return Err(crate::utils::prompt::Declined.into());
    }

    client.delete(&api_path(&["vaults", &vault.id, "members", &me.user_id])?)?;

    crate::say!("  {} left {label}", "✓".green());
    crate::say!(
        "  {}",
        format!(
            "Anything you pulled is still on this machine. The vault key was not rotated: \
             someone who stays can run `evnx vault rekey {label}`."
        )
        .dimmed()
    );
    if verbose {
        println!("  vault id  {}", vault.id);
    }
    Ok(())
}

/// "a" or "an" before a role: an admin, a developer.
fn article(role: &str) -> &'static str {
    if role.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// Find the vault a user meant.
///
/// Accepts a UUID, a bare `name`, or `name/environment`. A bare name that exists
/// in more than one environment is an error listing the candidates rather than a
/// guess — picking one and deleting it would be the worst possible outcome.
fn resolve<'a>(vaults: &'a [VaultSummary], target: &str) -> Result<&'a VaultSummary> {
    if let Some(v) = find_id(vaults, target) {
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

/// Reject a name the server's regex would reject, before a round trip.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(anyhow!("a vault name must be 1–64 characters"));
    }
    // The server's rule, `^[a-z0-9-]+$` (I-315, DEC-VL8): upper case passed here
    // and came back as "name: name: only alphanumeric and hyphens".
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        let suggestion: String = name
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        return Err(anyhow!(
            "a vault name may contain only lower-case letters, digits and hyphens \
             (got {name:?}; {suggestion:?} would do)"
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

    const ID: &str = "0b8a1f8e-3c2d-4e5f-8a9b-1c2d3e4f5a6b";

    /// C10: a vault id typed in upper case is stored lower-cased, so the AAD
    /// (which binds the id verbatim) is the one a pull by name derives.
    #[test]
    fn c10_an_uppercase_id_gives_the_same_aad_as_lowercase() {
        let client = Client::new("http://127.0.0.1:9".to_string()).unwrap();
        let upper = fetch_and_resolve(&client, &ID.to_uppercase()).unwrap();
        let lower = fetch_and_resolve(&client, ID).unwrap();
        assert_eq!(upper.id, ID);
        assert_eq!(
            evnx_crypto::vault_aad(&upper.id, 3),
            evnx_crypto::vault_aad(&lower.id, 3)
        );
        // The premise: the AAD does depend on the spelling.
        assert_ne!(
            evnx_crypto::vault_aad(&ID.to_uppercase(), 3),
            evnx_crypto::vault_aad(ID, 3)
        );
    }

    /// C10: an id is compared in any case, and the server's spelling returned.
    #[test]
    fn c10_ids_are_matched_ignoring_case() {
        let mut v = v("app", "production");
        v.id = ID.to_string();
        let list = [v];
        assert_eq!(
            find_id(&list, &ID.to_uppercase()).map(|v| &v.id[..]),
            Some(ID)
        );
        assert!(find_id(&list, "00000000-0000-0000-0000-000000000000").is_none());
        assert!(resolve(&list, &ID.to_uppercase()).is_ok());
    }

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
            // The server's rule is lower case (I-315); this list accepted
            // "A-B-9", which the server then refused.
            "A-B-9",
            "MyApp",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["api-keys", "v1", "a-b-9"] {
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

    /// ⚠️ The wire property a standalone re-key depends on, and the reason
    /// `rotate_vault_key` takes `remove_user_id` as an `Option` rather than two
    /// code paths.
    ///
    /// `remove_user_id` is what turns a rotation into a revocation: the server
    /// deletes that member inside the same transaction that swaps the key. When
    /// nobody is leaving, the field must be **absent** — the server declares it
    /// `#[serde(default)] Option<Uuid>` and branches on `if let Some(..)`, so an
    /// absent field is a pure rotation.
    #[test]
    fn a_rekey_with_nobody_leaving_omits_remove_user_id() {
        let rotate_only = RekeyRequest {
            versions: vec![],
            members: vec![],
            remove_user_id: None,
        };
        let json = serde_json::to_string(&rotate_only).unwrap();
        assert!(
            !json.contains("remove_user_id"),
            "a rotation with nobody leaving must not name a member to remove: {json}"
        );

        // And a revocation still carries it, or removal would silently stop
        // happening — the failure this would hide is a member who keeps a usable
        // key after the UI says they were removed.
        let revocation = RekeyRequest {
            versions: vec![],
            members: vec![],
            remove_user_id: Some("11111111-2222-3333-4444-555555555555".into()),
        };
        let json = serde_json::to_string(&revocation).unwrap();
        assert!(json.contains("remove_user_id"), "{json}");
        assert!(
            json.contains("11111111-2222-3333-4444-555555555555"),
            "{json}"
        );
    }

    fn member(email: &str, pq: bool, you: bool) -> MemberSummary {
        MemberSummary {
            user_id: format!("uid-{email}"),
            email: email.into(),
            role: "developer".into(),
            granted_at: "2026-10-01T00:00:00Z".into(),
            has_mlkem_key: pq,
            is_you: you,
        }
    }

    /// ⚠️ A re-key must refuse rather than quietly dropping someone.
    ///
    /// The server requires the payload to cover every member exactly once, so a
    /// member left out is a member removed. Silently revoking access while
    /// rotating is the precise failure `vault revoke` exists to make loud — and
    /// whoever runs a rotation under pressure is the least likely to notice.
    #[test]
    fn a_member_without_a_post_quantum_key_is_named_not_skipped() {
        let a = member("has-key@example.com", true, false);
        let b = member("needs-signin@example.com", false, false);
        let all = vec![&a, &b];

        assert_eq!(
            members_without_pq_key(&all),
            vec!["needs-signin@example.com"],
            "the member who cannot be re-wrapped must be reported by name"
        );
    }

    /// ⚠️ Your own copy is sealed under your master key with no key agreement, so
    /// it needs no ML-KEM key. Counting yourself would refuse every re-key run by
    /// an account that predates F1 — including a solo vault with no sharing in it
    /// at all, which is the most common vault there is.
    #[test]
    fn your_own_missing_pq_key_does_not_block_a_rekey() {
        let me = member("me@example.com", false, true);
        let all = vec![&me];
        assert!(
            members_without_pq_key(&all).is_empty(),
            "a solo vault owner with no ML-KEM key must still be able to re-key"
        );
    }

    /// Everyone ready is the common case and must not produce a spurious refusal.
    #[test]
    fn a_fully_equipped_vault_blocks_nobody() {
        let a = member("a@example.com", true, true);
        let b = member("b@example.com", true, false);
        let all = vec![&a, &b];
        assert!(members_without_pq_key(&all).is_empty());
    }
}

// ─── Member management (Phase 3) ──────────────────────────────────────────────

#[derive(Deserialize)]
struct MemberList {
    members: Vec<MemberSummary>,
}

#[derive(Deserialize, Serialize)]
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
pub fn members(
    server_override: Option<&str>,
    target: String,
    json: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;

    // An id needs no lookup (I-309): a vault-scoped token may read this vault's
    // members but is refused the list of vaults, so listing first failed it.
    let vault = fetch_and_resolve(&client, &target)?;

    let people: MemberList = client.get(&api_path(&["vaults", &vault.id, "members"])?)?;
    if json {
        return crate::core::json::emit(&serde_json::json!({ "members": people.members }), 1, None);
    }

    let label = if vault.name.is_empty() {
        vault.id.bold().to_string()
    } else {
        format!("{}/{}", vault.name.bold(), vault.environment)
    };
    println!("  {label} — {} member(s)", people.members.len());
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
    let role = parse_role(&role, VAULT_ROLES, VAULT_OWNER)?; // I-223, A-303
    let user_email = user_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "change a member's role",
        "\x20 Roles decide who can push, share and revoke; a leaked CI token must\n\x20 not be able to promote anyone.\n",
    )?;

    let vault = list_and_pick(&client, &target)?;

    let people: MemberList =
        client.get(&api_path(&["vaults", &vault.id.to_string(), "members"])?)?;
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
        crate::say!(
            "  {user_email} is already {role} on {}/{}.",
            vault.name,
            vault.environment
        );
        return Ok(());
    }

    client.patch::<_, serde::de::IgnoredAny>(
        &api_path(&[
            "vaults",
            &vault.id.to_string(),
            "members",
            &member.user_id.to_string(),
        ])?,
        &SetRoleRequest { role: role.clone() },
    )?;

    crate::say!(
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

/// What `evnx vault revoke` was asked to do.
pub struct RevokeOptions {
    /// The vault named on the command line; `None` with `--all-vaults`.
    pub target: Option<String>,
    /// `--all-vaults`: every vault where the person is a member.
    pub all_vaults: bool,
    /// The member to remove, by email.
    pub user_email: String,
    pub assume_yes: bool,
    pub no_rekey: bool,
    pub password_stdin: bool,
    pub dry_run: bool,
    pub json: bool,
    pub verbose: bool,
}

/// One vault a revoke will act on: who leaves, and who stays.
struct Planned {
    vault: VaultSummary,
    label: String,
    leaving: MemberSummary,
    remaining: Vec<MemberSummary>,
}

/// A vault where the person is a member, but this revoke will not remove them.
struct Skipped {
    vault: VaultSummary,
    reason: String,
}

/// What became of one vault, for the summary and `--json`. Names only.
#[derive(Serialize)]
struct VaultOutcome {
    vault: String,
    environment: String,
    /// `revoked`, `would_revoke` (dry run), `skipped`, `failed` or
    /// `not_attempted` (after a failure).
    result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    versions_reencrypted: Option<usize>,
}

impl VaultOutcome {
    fn new(v: &VaultSummary, result: &'static str) -> Self {
        VaultOutcome {
            vault: v.name.clone(),
            environment: v.environment.clone(),
            result,
            reason: None,
            versions_reencrypted: None,
        }
    }

    fn because(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// `vault revoke --json`: who, what was asked, and each vault's result.
#[derive(Serialize)]
struct RevokeReport<'a> {
    user: &'a str,
    dry_run: bool,
    /// False with `--no-rekey`.
    rekey: bool,
    vaults: Vec<VaultOutcome>,
    /// Rotate at the source: the credentials the member could read, by name,
    /// with who issued each and where to replace it. Empty in a dry run.
    rotate: Vec<super::checklist::RotateEntry>,
    /// The other names they could read, which do not look like credentials.
    other_names: Vec<String>,
    /// What the names cover, one line per vault.
    rotate_notes: Vec<String>,
}

impl<'a> RevokeReport<'a> {
    fn new(
        user: &'a str,
        dry_run: bool,
        rekey: bool,
        vaults: Vec<VaultOutcome>,
        list: &super::checklist::Checklist,
    ) -> Self {
        RevokeReport {
            user,
            dry_run,
            rekey,
            vaults,
            rotate: list.rotate.clone(),
            other_names: list.other.clone(),
            rotate_notes: list.notes.clone(),
        }
    }
}

/// A line of the revoke's own text: on stdout, or on stderr when stdout
/// carries the `--json` document.
fn tell(json: bool, line: impl std::fmt::Display) {
    if json {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// A vault role's rank, for "who may remove whom". `None` for a role this
/// build does not know: then nothing is guessed.
fn vault_rank(role: &str) -> Option<u8> {
    match role {
        "viewer" => Some(0),
        "developer" => Some(1),
        "admin" => Some(2),
        "owner" => Some(3),
        _ => None,
    }
}

/// Why a caller with `my_role` cannot remove `leaving`, by the server's rules
/// (PROTOCOL 7.1): the owner cannot be removed, removing needs admin or owner,
/// and the caller must outrank the member. `None` when they can.
fn cannot_remove(my_role: &str, leaving: &MemberSummary) -> Option<String> {
    if leaving.role == "owner" {
        return Some("they own it, and the owner cannot be removed".into());
    }
    let (Some(me), Some(them)) = (vault_rank(my_role), vault_rank(&leaving.role)) else {
        return Some(format!(
            "a role this evnx does not know (yours: {my_role}, theirs: {}), so it does not guess",
            leaving.role
        ));
    };
    if me < 2 {
        return Some(format!(
            "you are {} {my_role}; removing a member needs admin or owner",
            article(my_role)
        ));
    }
    if me <= them {
        return Some(format!(
            "they are {} {} as you are; only the owner can remove them",
            article(&leaving.role),
            leaving.role
        ));
    }
    None
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
/// With `--all-vaults` it does that for every vault where the caller can see
/// the person as a member: one confirmation, one master password, then each
/// vault in turn, stopping at the first failure.
///
/// ─── ⚠️ The limit, stated plainly ────────────────────────────────────────────
///
/// Rotation stops the removed member reading anything pushed **from now on**. It
/// cannot recall copies of what they could already read. If they had access to a
/// secret, treat that secret as theirs and rotate it at its source — the same
/// advice that applies to a leaked API token, and the step people skip.
pub fn revoke(server_override: Option<&str>, opts: RevokeOptions) -> Result<Verdict> {
    let user_email = opts.user_email.trim().to_lowercase();

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "revoke a member",
        "\x20 Revoking re-keys the vault for everyone left; a leaked CI token must\n\x20 not be able to remove members or lock them out.\n",
    )?;
    if opts.json {
        // The status lines (`say!`) would land in the JSON document.
        crate::utils::ui::set_quiet(true);
    }

    match (&opts.target, opts.all_vaults) {
        (Some(target), false) => revoke_one_vault(&client, target, &user_email, &opts),
        (None, true) => revoke_all_vaults(&client, &user_email, &opts),
        _ => Err(anyhow!(
            "name one vault, or pass --all-vaults, not both. Nothing was changed."
        )),
    }
}

/// The checks `vault revoke <vault>` makes before it asks anything, as errors.
fn plan_one(client: &Client, target: &str, user_email: &str, no_rekey: bool) -> Result<Planned> {
    let vault = list_and_pick(client, target)?;
    let label = format!("{}/{}", vault.name, vault.environment);

    let people: MemberList =
        client.get(&api_path(&["vaults", &vault.id.to_string(), "members"])?)?;
    let mut members = people.members;

    let pos = members
        .iter()
        .position(|m| m.email == user_email)
        .ok_or_else(|| {
            anyhow!(
                "{user_email} is not a member of {label}.\n\
                 \x20 `evnx vault members {target}` shows who is."
            )
        })?;
    let leaving = members.remove(pos);

    // Yourself (I-318): it said "ask an admin to remove you" — to the owner too,
    // whom nobody can remove, and with `--no-rekey`, where re-keying is moot.
    if leaving.is_you {
        if leaving.role == "owner" {
            return Err(anyhow!(
                "you own {label}, and the owner cannot be removed.\n\
                 \x20 To be rid of it, delete it:  evnx vault delete {target}"
            ));
        }
        return Err(anyhow!(
            "to leave {label} yourself:  evnx vault leave {target}"
        ));
    }

    // ⚠️ Before the post-quantum check, the password and the re-encryption
    // (I-317): the server refuses to remove the owner, and finding that out at
    // the swap left every staged blob behind in object storage.
    if leaving.role == "owner" {
        return Err(anyhow!(
            "{user_email} owns {label}, and the owner cannot be removed."
        ));
    }

    // ── Refuse early if anyone cannot be re-wrapped ──────────────────────────
    //
    // ⚠️ Checked BEFORE any work, and before the password prompt. A member with
    // no post-quantum key cannot receive the new vault key, and discovering that
    // after re-encrypting fifty versions would be a wasted operation ending in a
    // rejected swap.
    if !no_rekey {
        let refs: Vec<&MemberSummary> = members.iter().collect();
        let stuck = members_without_pq_key(&refs);
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

    Ok(Planned {
        vault,
        label,
        leaving,
        remaining: members,
    })
}

/// Every vault where the caller can see `user_email` as a member: the ones this
/// revoke will act on, and the ones it must skip, with the reason.
///
/// Reads only (`GET /vaults`, then each vault's members). A vault whose members
/// cannot be read stops the whole command, before anything is changed: a list
/// with a hole in it would be an offboarding that silently missed a vault.
fn plan_all(
    client: &Client,
    user_email: &str,
    no_rekey: bool,
) -> Result<(Vec<Planned>, Vec<Skipped>)> {
    let vaults = list_vaults(client).context("listing your vaults")?;
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    for vault in vaults {
        let label = format!("{}/{}", vault.name, vault.environment);
        let people: MemberList = client
            .get(&api_path(&["vaults", &vault.id, "members"])?)
            .with_context(|| format!("listing the members of {label}; nothing was changed"))?;
        let mut members = people.members;
        let Some(pos) = members.iter().position(|m| m.email == user_email) else {
            continue;
        };
        let leaving = members.remove(pos);

        // Yourself: the same refusal as for one vault, for the whole command.
        if leaving.is_you {
            return Err(anyhow!(
                "{user_email} is your own account, and revoke removes someone else.\n\
                 \x20 To leave a vault yourself:  evnx vault leave <vault>\n\
                 \x20 Nothing was changed."
            ));
        }
        if let Some(reason) = cannot_remove(&vault.role, &leaving) {
            skipped.push(Skipped { vault, reason });
            continue;
        }
        if !no_rekey {
            let refs: Vec<&MemberSummary> = members.iter().collect();
            let stuck = members_without_pq_key(&refs);
            if !stuck.is_empty() {
                skipped.push(Skipped {
                    vault,
                    reason: format!(
                        "the new vault key cannot be wrapped for {}: no post-quantum \
                         sharing key yet (they need to sign in once)",
                        stuck.join(", ")
                    ),
                });
                continue;
            }
        }
        planned.push(Planned {
            vault,
            label,
            leaving,
            remaining: members,
        });
    }
    Ok((planned, skipped))
}

/// What one vault's revoke did: versions re-encrypted (`None` with
/// `--no-rekey`), and the names the member could read there.
struct Done {
    versions: Option<usize>,
    names: Option<super::checklist::VaultNames>,
}

/// Remove `p.leaving` from one vault: a bare removal with `--no-rekey`
/// (`unlocked` is `None`), otherwise the re-key that removes them.
fn revoke_planned(
    client: &Client,
    p: &Planned,
    unlocked: Option<&auth::Unlocked>,
    verbose: bool,
) -> Result<Done> {
    use super::checklist::{NameSource, VaultNames};

    let Some(unlocked) = unlocked else {
        // Nothing is decrypted without a re-key, so the names are the ones the
        // server recorded at each push. Read before the removal, while this is
        // still a vault the caller can list; a failure only loses the checklist.
        let names = api_path(&["vaults", &p.vault.id, "versions"])
            .ok()
            .and_then(|path| client.get::<super::sync::VersionList>(&path).ok())
            .map(|list| {
                let mut v = VaultNames::new(&p.label, NameSource::ServerRecorded);
                v.versions = list.versions.len();
                for version in list.versions {
                    v.names.extend(version.key_names);
                }
                v
            });
        client.delete(&api_path(&[
            "vaults",
            &p.vault.id.to_string(),
            "members",
            &p.leaving.user_id.to_string(),
        ])?)?;
        return Ok(Done {
            versions: None,
            names,
        });
    };
    // The rotation itself lives in `rotate_vault_key` so that `evnx vault rekey`
    // can do the same thing without removing anyone. `remove_user_id` is what
    // makes this a revocation: the server deletes that member inside the same
    // transaction that swaps the key, so there is no window where they are still
    // a member of a vault whose key has already moved, nor one where they are
    // gone but it has not.
    let refs: Vec<&MemberSummary> = p.remaining.iter().collect();
    let mut names = VaultNames::new(&p.label, NameSource::Decrypted);
    let total = rotate_vault_key(
        client,
        &p.vault,
        unlocked,
        &refs,
        Some(&p.leaving.user_id),
        Some(&mut names),
        verbose,
    )?;
    Ok(Done {
        versions: Some(total),
        names: Some(names),
    })
}

/// The master password, read and proven once, unless `--no-rekey` needs none.
fn unlock_once(
    client: &Client,
    no_rekey: bool,
    password_stdin: bool,
) -> Result<Option<auth::Unlocked>> {
    if no_rekey {
        return Ok(None);
    }
    let password = auth::read_master_password(password_stdin)?;
    Ok(Some(auth::unlock(client, &password)?))
}

/// The two warnings a revoke prints before it asks.
fn warn_before(json: bool, no_rekey: bool) {
    if no_rekey {
        tell(
            json,
            format!(
                "  {}",
                "--no-rekey: the vault key is NOT rotated. They keep the ability to".yellow()
            ),
        );
        tell(
            json,
            format!(
                "  {}",
                "decrypt every version they had access to, including future ones.".yellow()
            ),
        );
    } else {
        tell(
            json,
            format!(
                "  {}",
                "This stops them reading anything pushed from now on. It cannot".yellow()
            ),
        );
        tell(
            json,
            format!(
                "  {}",
                "recall copies of what they could already read — rotate those".yellow()
            ),
        );
        tell(json, format!("  {}", "secrets at their source.".yellow()));
    }
}

/// `evnx vault revoke <vault> --user <email>`.
fn revoke_one_vault(
    client: &Client,
    target: &str,
    user_email: &str,
    opts: &RevokeOptions,
) -> Result<Verdict> {
    let json = opts.json;
    let p = plan_one(client, target, user_email, opts.no_rekey)?;
    let label = p.label.clone();

    if opts.dry_run {
        if json {
            crate::core::json::emit(
                &RevokeReport::new(
                    user_email,
                    true,
                    !opts.no_rekey,
                    vec![VaultOutcome::new(&p.vault, "would_revoke")],
                    &Default::default(),
                ),
                1,
                None,
            )?;
        } else if opts.no_rekey {
            println!("  Would remove {user_email} from {label}, without rotating the vault key.");
        } else {
            println!(
                "  Would remove {user_email} from {label}, re-encrypting {} version(s) and \
                 re-wrapping the key for {} member(s).",
                p.vault.version_count,
                p.remaining.len()
            );
        }
        if !json {
            println!("  Dry run: nothing was changed.");
        }
        return Ok(Verdict::Clean);
    }

    // ── Confirm ──────────────────────────────────────────────────────────────
    if !opts.assume_yes {
        tell(
            json,
            format!("  About to remove {} from {label}.", user_email.bold()),
        );
        if !opts.no_rekey {
            tell(
                json,
                "  Every version will be re-encrypted under a fresh key and re-wrapped",
            );
            tell(
                json,
                format!("  for the {} remaining member(s).", p.remaining.len()),
            );
            tell(json, "");
        }
        warn_before(json, opts.no_rekey);
        let ok = crate::utils::prompt::confirm(
            &format!("Remove {user_email} from {label}?"),
            false,
            &format!("evnx vault revoke {target} --user {user_email} --yes"),
        )?;
        if !ok {
            tell(json, "  Cancelled; nothing was changed.");
            return Err(crate::utils::prompt::Declined.into());
        }
    }

    let unlocked = unlock_once(client, opts.no_rekey, opts.password_stdin)?;
    let done = revoke_planned(client, &p, unlocked.as_ref(), opts.verbose)?;
    let list = super::checklist::build(done.names.as_slice());

    if json {
        let mut outcome = VaultOutcome::new(&p.vault, "revoked");
        outcome.versions_reencrypted = done.versions;
        crate::core::json::emit(
            &RevokeReport::new(user_email, false, !opts.no_rekey, vec![outcome], &list),
            1,
            None,
        )?;
        return Ok(Verdict::Clean);
    }

    // ── The simple path ──────────────────────────────────────────────────────
    let Some(total) = done.versions else {
        crate::say!("  {} removed {user_email} from {label}", "✓".green());
        println!(
            "  {}",
            "The vault key was NOT rotated. Rotate the secrets themselves.".yellow()
        );
        print_checklist(&list, done.names.is_some(), false);
        return Ok(Verdict::Clean);
    };

    crate::say!();
    crate::say!(
        "  {} removed {user_email} from {label} and rotated the vault key",
        "✓".green()
    );
    crate::say!(
        "  {total} version(s) re-encrypted, {} member(s) re-wrapped",
        p.remaining.len()
    );
    crate::say!();
    crate::say!(
        "  {}",
        "They can no longer read anything pushed from now on.".dimmed()
    );
    println!(
        "  {}",
        "They may still hold copies of what they could already read —".yellow()
    );
    println!("  {}", "rotate those secrets at their source.".yellow());
    print_checklist(&list, true, false);

    Ok(Verdict::Clean)
}

/// The "rotate at the source" checklist, or why there is none (the names
/// could not be listed). Names only.
fn print_checklist(list: &super::checklist::Checklist, have_names: bool, several: bool) {
    if have_names {
        super::checklist::print(list, several);
    } else {
        println!(
            "  {}",
            "The names they could read could not be listed; check them with `evnx cloud history`."
                .dimmed()
        );
    }
}

/// `evnx vault revoke --all-vaults --user <email>`: offboarding in one command.
///
/// Lists every vault where the caller can see the person, skips (with the
/// reason, before asking) the ones the caller cannot revoke in, asks once,
/// reads the master password once, then revokes in each vault in turn with the
/// one-vault code, and stops at the first failure.
///
/// Exit: 0 when every vault listed was revoked (or the person is in none); 1
/// when a vault was skipped, so the person still has access there; a failure's
/// own code when one vault failed (the summary says which were done).
fn revoke_all_vaults(client: &Client, user_email: &str, opts: &RevokeOptions) -> Result<Verdict> {
    let json = opts.json;
    let (planned, skipped) = plan_all(client, user_email, opts.no_rekey)?;
    let skipped_outcomes = || -> Vec<VaultOutcome> {
        skipped
            .iter()
            .map(|s| VaultOutcome::new(&s.vault, "skipped").because(s.reason.clone()))
            .collect()
    };
    let verdict = if skipped.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Findings
    };
    let none = super::checklist::Checklist::default();
    let report = |vaults: Vec<VaultOutcome>, dry_run: bool, list: &super::checklist::Checklist| {
        RevokeReport::new(user_email, dry_run, !opts.no_rekey, vaults, list)
    };

    if planned.is_empty() && skipped.is_empty() {
        if json {
            crate::core::json::emit(&report(Vec::new(), opts.dry_run, &none), 1, None)?;
        } else {
            println!("  {user_email} is not a member of any vault you can see. Nothing to do.");
        }
        return Ok(Verdict::Clean);
    }

    // ── What will happen, before anything is asked ───────────────────────────
    tell(
        json,
        format!(
            "  {} is a member of {} vault(s) you can see.",
            user_email.bold(),
            planned.len() + skipped.len()
        ),
    );
    if !planned.is_empty() {
        tell(
            json,
            if opts.no_rekey {
                "  Will remove them from these vaults, without rotating the vault keys:"
            } else {
                "  Will remove them from these vaults, re-keying each one:"
            },
        );
        for p in &planned {
            tell(
                json,
                format!(
                    "    {}  ({}, {} version(s), {} member(s) stay)",
                    p.label,
                    p.leaving.role,
                    p.vault.version_count,
                    p.remaining.len()
                ),
            );
        }
    }
    if !skipped.is_empty() {
        tell(
            json,
            format!("  {}", "Skipped, so they keep access there:".yellow()),
        );
        for s in &skipped {
            tell(
                json,
                format!("    {}/{}  {}", s.vault.name, s.vault.environment, s.reason),
            );
        }
    }

    if planned.is_empty() {
        if json {
            crate::core::json::emit(&report(skipped_outcomes(), opts.dry_run, &none), 1, None)?;
        }
        tell(
            json,
            "  Nothing here that you can revoke; nothing was changed.",
        );
        return Ok(verdict);
    }

    if opts.dry_run {
        if json {
            let mut vaults: Vec<VaultOutcome> = planned
                .iter()
                .map(|p| VaultOutcome::new(&p.vault, "would_revoke"))
                .collect();
            vaults.extend(skipped_outcomes());
            crate::core::json::emit(&report(vaults, true, &none), 1, None)?;
        }
        tell(json, "  Dry run: nothing was changed.");
        return Ok(verdict);
    }

    // ── Confirm once, for every vault ────────────────────────────────────────
    if !opts.assume_yes {
        tell(json, "");
        warn_before(json, opts.no_rekey);
    }
    let ok = crate::utils::prompt::confirm(
        &format!("Remove {user_email} from these {} vault(s)?", planned.len()),
        opts.assume_yes,
        &format!("evnx vault revoke --all-vaults --user {user_email} --yes"),
    )?;
    if !ok {
        tell(json, "  Cancelled; nothing was changed.");
        return Err(crate::utils::prompt::Declined.into());
    }

    // ── One password, then each vault in turn ────────────────────────────────
    let unlocked = unlock_once(client, opts.no_rekey, opts.password_stdin)?;

    let mut outcomes = Vec::with_capacity(planned.len() + skipped.len());
    let mut failure: Option<anyhow::Error> = None;
    let mut read = Vec::new();
    let mut unnamed = Vec::new();
    for p in &planned {
        if failure.is_some() {
            outcomes.push(VaultOutcome::new(&p.vault, "not_attempted"));
            continue;
        }
        crate::say!();
        crate::say!("  {}", p.label.bold());
        match revoke_planned(client, p, unlocked.as_ref(), opts.verbose) {
            Ok(done) => {
                let total = done.versions;
                match total {
                    Some(n) => crate::say!(
                        "  {} removed {user_email} from {} and rotated the vault key ({n} version(s) re-encrypted)",
                        "✓".green(),
                        p.label
                    ),
                    None => crate::say!(
                        "  {} removed {user_email} from {} (vault key not rotated)",
                        "✓".green(),
                        p.label
                    ),
                }
                let mut outcome = VaultOutcome::new(&p.vault, "revoked");
                outcome.versions_reencrypted = total;
                outcomes.push(outcome);
                match done.names {
                    Some(n) => read.push(n),
                    None => unnamed.push(p.label.clone()),
                }
            }
            Err(e) => {
                outcomes.push(VaultOutcome::new(&p.vault, "failed").because(e.to_string()));
                failure = Some(e);
            }
        }
    }
    outcomes.extend(skipped_outcomes());

    let mut list = super::checklist::build(&read);
    for label in &unnamed {
        list.notes.push(format!(
            "{label}: the names could not be listed; check them with `evnx cloud history --vault {label}`."
        ));
    }

    // ── Summary ──────────────────────────────────────────────────────────────
    if json {
        crate::core::json::emit(&report(outcomes, false, &list), 1, None)?;
    } else {
        let names = |result: &str| -> Vec<String> {
            outcomes
                .iter()
                .filter(|o| o.result == result)
                .map(|o| format!("{}/{}", o.vault, o.environment))
                .collect()
        };
        println!();
        println!("  Summary for {user_email}:");
        for (result, title) in [
            ("revoked", "revoked      "),
            ("failed", "failed       "),
            ("not_attempted", "not attempted"),
            ("skipped", "skipped      "),
        ] {
            let list = names(result);
            if list.is_empty() {
                continue;
            }
            let tail = match result {
                "failed" => "  (the error is below; check it with `evnx vault members <vault>`)",
                "not_attempted" => "  (stopped at the failure; run the command again)",
                "skipped" => "  (they keep access there)",
                _ => "",
            };
            println!("    {title}  {}{tail}", list.join(", "));
        }
        if outcomes.iter().any(|o| o.result == "revoked") {
            if opts.no_rekey {
                println!(
                    "  {}",
                    "The vault keys were NOT rotated. Rotate the secrets themselves.".yellow()
                );
            } else {
                println!(
                    "  {}",
                    "They may still hold copies of what they could already read —".yellow()
                );
                println!("  {}", "rotate those secrets at their source.".yellow());
            }
            print_checklist(&list, true, read.len() + unnamed.len() > 1);
        }
    }

    match failure {
        Some(e) => Err(e),
        None => Ok(verdict),
    }
}

/// Members who cannot receive a re-wrapped vault key, by email.
///
/// A share is wrapped with hybrid X25519 + ML-KEM-768, so a recipient needs a
/// post-quantum public key on file. Accounts that registered before F1 upload one
/// on their next sign-in, which means this clears itself — but only when that
/// person actually signs in.
///
/// ⚠️ **`is_you` is excluded, and that is not an oversight.** Your own copy is
/// sealed under your master key with no key agreement at all — the creator's wrap
/// has always been symmetric — so it needs no ML-KEM key. Including yourself here
/// would refuse every re-key performed by an account that happens to predate F1,
/// including solo vaults that have no sharing in them whatsoever.
///
/// Both callers check this **before** prompting for a password and before any
/// work: the server rejects a payload that does not cover every member, and
/// discovering that after re-encrypting fifty versions wastes the operation.
fn members_without_pq_key<'a>(members: &[&'a MemberSummary]) -> Vec<&'a str> {
    members
        .iter()
        .filter(|m| !m.has_mlkem_key && !m.is_you)
        .map(|m| m.email.as_str())
        .collect()
}

/// Rotate a vault's key without removing anyone.
///
/// ─── Why this exists separately from `revoke` ────────────────────────────────
///
/// Removing a member has to rotate the key, or they keep a usable copy. Rotating
/// does **not** have to remove anyone — and the situations that call for it most
/// have nobody leaving: a lost laptop with a signed-in CLI, a CI runner that held
/// the master password, an account breach where the person stays on the team.
///
/// Before this command the only way to rotate was to remove a member and add them
/// back, which is both a worse audit trail and a window in which they genuinely
/// had no access.
///
/// ─── What it does not do ─────────────────────────────────────────────────────
///
/// ⚠️ It fixes the future, not the past. Anyone who could already read a version
/// may hold a copy of the plaintext, and no server-side operation recalls that.
/// The output says so, rather than leaving it to the docs — it is the same step
/// people skip with a leaked API token: **rotate the secrets themselves.**
///
/// It also does not sign anyone out. Members re-fetch their wrapped key on the
/// next command, so a rotation is invisible to them apart from the audit trail.
pub fn rekey(
    server_override: Option<&str>,
    target: String,
    assume_yes: bool,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "re-key a vault",
        "\x20 A re-key the server cannot check can lock every member out for good;\n\x20 it takes a signed-in member.\n",
    )?;

    let vault = list_and_pick(&client, &target)?;
    let label = format!("{}/{}", vault.name, vault.environment);

    let people: MemberList =
        client.get(&api_path(&["vaults", &vault.id.to_string(), "members"])?)?;

    // Everyone stays. The server requires the payload cover every member exactly
    // once, so this list IS the contract — a member missed here would be dropped.
    let everyone: Vec<&MemberSummary> = people.members.iter().collect();

    // ── Refuse early if anyone cannot be re-wrapped ──────────────────────────
    //
    // ⚠️ Checked BEFORE the password prompt and before any work, matching
    // `revoke`. A member with no post-quantum key cannot receive the new vault
    // key, the server rejects a payload that does not cover every member, and
    // finding out after re-encrypting fifty versions wastes the whole operation.
    //
    // ⚠️ It refuses rather than dropping them. Silently revoking someone while
    // rotating would be the exact failure `vault revoke` exists to make loud —
    // and the person running this under pressure is the least able to notice.
    let stuck = members_without_pq_key(&everyone);
    if !stuck.is_empty() {
        return Err(anyhow!(
            "these members have no post-quantum sharing key, so the vault key \n\
             cannot be re-wrapped for them:\n\
             \x20   {}\n\
             \x20\n\
             \x20 They each need to sign in once with evnx 0.5 or later, or at \n\
             \x20 app.evnx.dev. Re-keying without them would lock them out of \n\
             \x20 {label} entirely.\n\
             \x20\n\
             \x20 If someone should no longer have access, remove them with \n\
             \x20 `evnx vault revoke {target} --user <email>`, which rotates the \n\
             \x20 key as part of the same operation.",
            stuck.join("\n    ")
        ));
    }

    // ── Confirm ──────────────────────────────────────────────────────────────
    if !assume_yes {
        println!("  About to rotate the key for {}.", label.bold());
        println!(
            "  {} version(s) will be re-encrypted and re-wrapped for {} member(s).",
            vault.version_count,
            everyone.len()
        );
        println!("  Nobody loses access, and nobody is signed out.");
        println!();
        println!(
            "  {}",
            "This stops anyone holding only the OLD key from reading what is".yellow()
        );
        println!(
            "  {}",
            "pushed from now on. It cannot recall copies of what they could".yellow()
        );
        println!(
            "  {}",
            "already read — rotate those secrets at their source.".yellow()
        );
        let ok = crate::utils::prompt::confirm(
            &format!("Rotate the key for {label}?"),
            false,
            &format!("evnx vault rekey {target} --yes"),
        )?;
        if !ok {
            println!("  Cancelled; nothing was changed.");
            return Err(crate::utils::prompt::Declined.into());
        }
    }

    let password = auth::read_master_password(password_stdin)?;
    let unlocked = auth::unlock(&client, &password)?;

    // `None` — nobody is being removed, which is the whole difference from
    // `revoke`. The field is omitted from the request entirely, and the server
    // treats an absent `remove_user_id` as a pure rotation.
    let total = rotate_vault_key(&client, &vault, &unlocked, &everyone, None, None, verbose)?;

    crate::say!();
    crate::say!("  {} rotated the key for {label}", "\u{2713}".green());
    crate::say!(
        "  {total} version(s) re-encrypted, {} member(s) re-wrapped",
        everyone.len()
    );
    crate::say!();
    crate::say!(
        "  {}",
        "Members keep access and stay signed in — they pick up the new key".dimmed()
    );
    crate::say!("  {}", "on their next command.".dimmed());
    crate::say!();
    println!(
        "  {}",
        "Anyone who could already read a version may still hold a copy —".yellow()
    );
    println!("  {}", "rotate those secrets at their source.".yellow());

    Ok(())
}

/// Rotate a vault's key: re-encrypt every version under a fresh one, re-wrap it for
/// everyone named, and swap the whole lot in a single request.
///
/// Returns the number of versions re-encrypted, so the caller can word its own
/// summary — `revoke` and `rekey` report the same work for different reasons.
///
/// ## Why this is a function and not the tail of `revoke`
///
/// Rotating a vault key and removing a member are separate acts that happened to
/// ship together. Removal needs rotation — otherwise a former member keeps a usable
/// key — but rotation does not need removal, and the cases that call for it most
/// have nobody leaving: a lost laptop, a compromised CI runner, an account breach
/// where the person stays on the team.
///
/// `remove_user_id` is the only thing that makes a rotation a revocation. The server
/// deletes that member inside the same transaction that swaps the key, so there is
/// no window in which they are still a member of a vault whose key has moved, nor
/// one in which they are gone but it has not.
///
/// ## What callers must do first
///
/// ⚠️ **Check `has_mlkem_key` on everyone in `wrap_for` before calling.** A member
/// without one cannot receive the new key, the server rejects a payload that does
/// not cover every member, and discovering that *after* re-encrypting fifty versions
/// wastes the whole operation. Both callers check up front, before the password
/// prompt.
///
/// ## What it cannot do
///
/// ⚠️ Rotation fixes the future, not the past. Anyone who could already read a
/// version may hold a copy of the plaintext, and no server-side operation recalls
/// that. Callers must say so rather than implying a rotation reaches backwards.
///
/// ## Why two requests
///
/// Each version becomes one new blob. Sending them inline would be simpler, but
/// `MAX_REQUEST_SIZE_KB` defaults to 64 KB and a single version can approach it, so
/// a vault with any history would be unrotatable. Blobs are staged one at a time,
/// then **one** request swaps all the metadata atomically. Staged blobs that are
/// never committed are orphans in object storage — wasted bytes, not a correctness
/// problem, and unreachable because nothing references them.
fn rotate_vault_key(
    client: &Client,
    vault: &VaultSummary,
    unlocked: &auth::Unlocked,
    wrap_for: &[&MemberSummary],
    remove_user_id: Option<&str>,
    mut names: Option<&mut super::checklist::VaultNames>,
    verbose: bool,
) -> Result<usize> {
    use evnx_crypto::{
        EncryptedBlob, UserPublicKeys, VaultKey, WrappedVaultKey, b64_encode, blob_hash,
        reencrypt_vault, unwrap_vault_key, unwrap_vault_key_with_master_key, vault_aad,
        wrap_vault_key_for_user, wrap_vault_key_with_master_key,
    };

    let label = format!("{}/{}", vault.name, vault.environment);
    let label = label.as_str();
    let my_key: MyWrappedKey =
        client.get(&api_path(&["vaults", &vault.id.to_string(), "my-key"])?)?;

    let old_key = match (&my_key.eph_pub_key, &my_key.mlkem_ciphertext) {
        (None, None) => {
            let wrapped =
                evnx_crypto::b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
                    .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;
            unwrap_vault_key_with_master_key(&wrapped, &unlocked.master_key)
                .map_err(|_| auth::own_copy_will_not_open(label))?
        }
        (Some(eph), Some(ct)) => {
            let wrapped = WrappedVaultKey::from_base64(&my_key.encrypted_vault_key, eph, ct)
                .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;
            unwrap_vault_key(&wrapped, &unlocked.keypair)
                .map_err(|_| anyhow!("could not open your key for {label}."))?
        }
        _ => {
            return Err(anyhow!(
                "the server sent a malformed key for {label} — half a wrap. \n\
                 \x20 Treat this server as untrusted."
            ));
        }
    };

    let new_key = VaultKey::generate();

    // ── Re-encrypt every version ─────────────────────────────────────────────
    let versions: RevokeVersionList =
        client.get(&api_path(&["vaults", &vault.id.to_string(), "versions"])?)?;

    let total = versions.versions.len();
    crate::say!("  Re-encrypting {total} version(s)…");

    let mut staged = Vec::with_capacity(total);
    for (i, v) in versions.versions.iter().enumerate() {
        let n = v.version_num;
        // RM-57: `purpose=rekey` lets the audit trail show these reads as part
        // of the rekey rather than as someone pulling every version. The
        // server records it only for an admin or the owner, who are the only
        // ones who can rekey, and ignores it otherwise.
        let path = api_path(&[
            "vaults",
            &vault.id.to_string(),
            "versions",
            &n.to_string(),
            "blob",
        ])?;
        let blob_bytes = client
            .get_bytes(&format!("{path}?purpose=rekey"))
            .with_context(|| format!("fetching version {n}"))?;

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

        // REVOKE-CHECKLIST: the names this version holds, for "rotate at the
        // source". Only names leave this block; the plaintext is wiped when it
        // drops. A blob that does not open fails just below, with its own
        // message.
        if let Some(names) = names.as_deref_mut() {
            names.versions += 1;
            if let Ok(plain) = evnx_crypto::decrypt_vault(&old_blob, &old_key, &aad) {
                let plain = zeroize::Zeroizing::new(plain);
                let (found, note) = super::sync::read_key_names(&plain);
                if note.is_some() {
                    names.unreadable.push(n);
                } else {
                    names.names.extend(found);
                }
            }
        }

        let fresh = reencrypt_vault(&old_blob, &old_key, &new_key, &aad)
            .map_err(|_| anyhow!("could not re-encrypt version {n} — is your key current?"))?;

        let hash = blob_hash(&fresh.ciphertext);
        let out: StagedBlob = client
            .post(
                &api_path(&["vaults", &vault.id.to_string(), "rekey", "blobs"])?,
                &StageBlobRequest {
                    version_num: n,
                    nonce: b64_encode(&fresh.nonce),
                    ciphertext: b64_encode(&fresh.ciphertext),
                    blob_hash: hash.clone(),
                },
            )
            .with_context(|| format!("staging version {n}"))?;

        staged.push(RekeyedVersion {
            version_num: out.version_num,
            blob_key: out.blob_key,
            blob_hash: hash,
            blob_size_bytes: out.blob_size_bytes,
        });

        if verbose {
            crate::say!("    version {n} re-encrypted ({}/{})", i + 1, total);
        }
    }

    // ── Re-wrap for everyone who stays ───────────────────────────────────────
    crate::say!("  Re-wrapping for {} member(s)…", wrap_for.len());

    let mut wraps = Vec::with_capacity(wrap_for.len());
    for m in wrap_for {
        if m.is_you {
            // Our own copy stays sealed under the master key: symmetric, no key
            // agreement, and the shape a vault creator's copy has always had.
            let wrapped = wrap_vault_key_with_master_key(&new_key, &unlocked.master_key)
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
            .get(&api_path(&["users", &m.email.to_string(), "public-key"])?)
            .with_context(|| format!("fetching {}'s public keys", m.email))?;
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
    client.post::<_, serde::de::IgnoredAny>(
        &api_path(&["vaults", &vault.id.to_string(), "rekey"])?,
        &RekeyRequest {
            versions: staged,
            members: wraps,
            remove_user_id: remove_user_id.map(str::to_owned),
        },
    )?;

    Ok(total)
}
