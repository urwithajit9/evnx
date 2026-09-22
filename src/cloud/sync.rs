//! `evnx cloud push` and `evnx cloud pull` — the round trip.
//!
//! # What the server learns, and what it cannot
//!
//! It receives the AES-256-GCM nonce, the ciphertext, a BLAKE3 hash of that
//! ciphertext for transport integrity, and **the key names** — `DATABASE_URL`,
//! `STRIPE_KEY` — so the dashboard can show what a vault holds. It never receives
//! a single value, and holds no key that could recover one. Key *names* being
//! visible is a deliberate trade for a usable listing, and is worth knowing:
//! if a name is itself sensitive, it should not be a name.
//!
//! # The file is encrypted verbatim
//!
//! Push encrypts the raw bytes read from disk — not a parsed and re-serialised
//! form. Comments, blank lines, key order and quoting all survive, so a pull into
//! a clean directory reproduces the file byte for byte. The parser is used only to
//! list key names for the server's metadata; if it cannot read the file, the push
//! still carries the exact bytes and simply reports no names.
//!
//! # Version numbers are authenticated, which makes `base_version` mandatory
//!
//! [`vault_aad`] binds a blob to `(vault_id, version)`, and decryption fails if
//! either differs. The server assigns `current + 1`, so the client has to predict
//! that number and seal it in *before* sending. `base_version` is what makes the
//! prediction safe: if someone else pushed in between, the server answers 409
//! instead of assigning a different number, which would otherwise produce a blob
//! nobody could ever open.
//!
//! That is also why a 409 cannot be retried by re-sending the same bytes. The
//! plaintext must be re-encrypted against the new version.
//!
//! [`vault_aad`]: evnx_crypto::vault_aad

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::auth;
use super::binding;
use super::client::{ApiError, Client};
use super::config::CloudConfig;
use super::vault::{self, VaultRef};

/// AES-GCM nonce length. The stored blob is `nonce || ciphertext`.
pub(crate) const NONCE_LEN: usize = 12;

/// Work out which vault a command means.
///
/// `--vault` wins; otherwise the directory's binding from `.evnx.toml`. When the
/// binding is used it is announced, so a push never goes somewhere the person
/// running it cannot see from the output.
pub(crate) fn resolve_target(explicit: Option<String>) -> Result<String> {
    if let Some(v) = explicit {
        return Ok(v);
    }
    let cwd = std::env::current_dir().context("reading the current directory")?;
    match binding::read(&cwd)? {
        Some(bound) => {
            println!(
                "  using {} from {}",
                bound.vault.bold(),
                bound.path.display()
            );
            Ok(bound.vault)
        }
        None => Err(anyhow!(
            "no vault given and this directory has no binding.\n\
             \x20 Pass --vault <name>, or bind the directory once with \
             `evnx cloud link <name>`."
        )),
    }
}

/// How the env file for a push or pull was arrived at, so the command can say so.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FileChoice {
    /// `--file <path>`.
    Explicit,
    /// `--env-name <name>`.
    Named,
    /// `.env.<vault environment>`, because that file exists.
    MatchedEnvironment,
    /// Plain `.env`.
    Default,
}

impl FileChoice {
    fn note(self) -> &'static str {
        match self {
            FileChoice::Explicit => "--file",
            FileChoice::Named => "--env-name",
            FileChoice::MatchedEnvironment => "matched the vault environment",
            FileChoice::Default => "default",
        }
    }
}

/// Decide which local file a push or pull should use.
///
/// ⚠️ Why this exists: `--file` defaulted to `.env` whatever the vault was, so
/// `evnx cloud push --vault app/production` with a forgotten flag pushed local
/// development secrets into the production vault — encrypted, versioned, and
/// shared with everyone holding a key. Nothing said which file it had read.
///
/// `vault_env` is empty when the vault was addressed by id, since a vault-scoped
/// token cannot list vaults; there is nothing to derive from then, so it falls
/// through to `.env`.
///
/// Unlike the local commands, a derived name that does not exist is **not** an
/// error — most projects genuinely have only `.env`, and refusing would break
/// the common case. What makes that safe is that the caller reports the choice
/// every time: a push that names its source cannot silently push the wrong one.
fn resolve_env_file(
    dir: &Path,
    explicit: Option<PathBuf>,
    env_name: Option<&str>,
    vault_env: &str,
) -> Result<(PathBuf, FileChoice)> {
    if let Some(path) = explicit {
        return Ok((path, FileChoice::Explicit));
    }

    if let Some(name) = env_name {
        return Ok((
            crate::core::env_name::resolve(dir, Some(name))?,
            FileChoice::Named,
        ));
    }

    if !vault_env.is_empty() {
        let candidate = dir.join(format!(".env.{vault_env}"));
        if candidate.is_file() {
            return Ok((candidate, FileChoice::MatchedEnvironment));
        }
    }

    Ok((dir.join(".env"), FileChoice::Default))
}

/// Print the file a command settled on, and warn when the pairing looks wrong.
///
/// The warning case is narrow on purpose: a **production** vault paired with a
/// plain `.env` that was not asked for by name. That is the shape of "I forgot
/// the flag", and it is the one that puts development secrets in a production
/// vault.
fn report_file(file: &Path, choice: FileChoice, vault_env: &str, action: &str) {
    println!("  file      {} ({})", file.display(), choice.note());

    if choice == FileChoice::Default && vault_env == "production" {
        println!(
            "  {} {} to a production vault from {} — pass --env-name or --file if that is not what you meant",
            "!".yellow(),
            action,
            file.display()
        );
    }
}

#[derive(Serialize)]
struct PushVersionRequest {
    nonce: String,
    ciphertext: String,
    blob_hash: String,
    key_names: Vec<String>,
    key_count: i32,
    /// Always sent. See the module docs — this is a correctness requirement, not
    /// just conflict detection.
    base_version: Option<i32>,
}

#[derive(Deserialize)]
struct PushVersionResponse {
    version_num: i32,
    #[allow(dead_code)]
    pushed_at: String,
}

/// Only the version number is needed; the rest of `/latest` is metadata for
/// display, which `cloud history` will use.
#[derive(Deserialize)]
struct LatestVersion {
    version_num: i32,
}

#[derive(Deserialize)]
struct VersionList {
    versions: Vec<VersionSummary>,
}

#[derive(Deserialize)]
struct VersionSummary {
    version_num: i32,
    key_count: i32,
    key_names: Vec<String>,
    blob_size_bytes: i64,
    pushed_by: String,
    pushed_at: String,
}

#[derive(Deserialize)]
struct MeId {
    user_id: String,
}

#[derive(Deserialize)]
struct MyKey {
    encrypted_vault_key: String,
    /// Present only when this copy was wrapped for you by someone else — that is,
    /// on a vault shared with you. `None` for a vault you created.
    eph_pub_key: Option<String>,
    /// The ML-KEM-768 half of a shared wrap. Always present alongside
    /// `eph_pub_key` and always absent without it; the server's
    /// `vault_members_wrap_is_whole` constraint makes any other pairing
    /// unstorable.
    #[serde(default)]
    mlkem_ciphertext: Option<String>,
}

/// Encrypt a `.env` and upload it as a new version.
#[allow(clippy::too_many_arguments)]
pub fn push(
    server_override: Option<&str>,
    vault_target: Option<String>,
    file: Option<PathBuf>,
    env_name: Option<String>,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{b64_encode, encrypt_vault, vault_aad};

    let vault_target = resolve_target(vault_target)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    // The vault has to be resolved before the file can be: which environment it
    // holds is what `.env.<environment>` is derived from.
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let (file, choice) = resolve_env_file(
        Path::new("."),
        file,
        env_name.as_deref(),
        &vault_ref.environment,
    )?;
    report_file(&file, choice, &vault_ref.environment, "pushing");

    // Still read before the password prompt: a missing file should fail
    // instantly, not after a second of Argon2id.
    let plaintext = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
    if plaintext.is_empty() {
        return Err(anyhow!("{} is empty; nothing to push", file.display()));
    }

    let current = current_version(&client, &vault_ref)?;
    let target_version = current + 1;

    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;
    let vault_key = unwrap_vault_key(&client, &vault_ref, &master_key)?;

    let (key_names, parse_note) = read_key_names(&plaintext);

    let aad = vault_aad(&vault_ref.id, target_version as u32);
    let blob = encrypt_vault(&plaintext, &vault_key, &aad)
        .map_err(|e| anyhow!("encrypting {}: {e}", file.display()))?;
    let blob_hash = blake3::hash(&blob.ciphertext).to_hex().to_string();

    let pushed: PushVersionResponse = client
        .post(
            &format!("/api/v1/vaults/{}/versions", vault_ref.id),
            &PushVersionRequest {
                nonce: b64_encode(&blob.nonce),
                ciphertext: b64_encode(&blob.ciphertext),
                blob_hash,
                key_count: key_names.len() as i32,
                key_names,
                base_version: Some(current),
            },
        )
        .map_err(|e| push_error(e, current))?;

    println!(
        "  {} pushed {} to {} as version {}",
        "✓".green(),
        file.display(),
        vault_ref.label(),
        pushed.version_num
    );
    if let Some(note) = parse_note {
        println!("  {} {note}", "note:".yellow());
    }
    if verbose {
        println!("  bytes     {} plaintext", plaintext.len());
        println!(
            "  aad       vault {} version {}",
            vault_ref.id, target_version
        );
    }

    // A mismatch here would mean the AAD we sealed does not match the version the
    // server recorded, so the blob could never be decrypted. Cheap to check.
    if pushed.version_num != target_version {
        return Err(anyhow!(
            "the server stored this as version {} but it was encrypted for version {}. \
             That blob cannot be decrypted — push again.",
            pushed.version_num,
            target_version
        ));
    }
    Ok(())
}

/// Download a version and decrypt it to a file.
#[allow(clippy::too_many_arguments)]
pub fn pull(
    server_override: Option<&str>,
    vault_target: Option<String>,
    file: Option<PathBuf>,
    env_name: Option<String>,
    version: Option<i32>,
    force: bool,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{decrypt_vault, vault_aad, EncryptedBlob};

    let vault_target = resolve_target(vault_target)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    // ⚠️ On pull the derivation only fires when the file already exists, so a
    // first pull into a fresh checkout still writes `.env`. That is the right
    // default: pull must not invent `.env.production` in a project that has no
    // such convention. Name it explicitly to create one.
    let (file, choice) = resolve_env_file(
        Path::new("."),
        file,
        env_name.as_deref(),
        &vault_ref.environment,
    )?;
    report_file(&file, choice, &vault_ref.environment, "writing");

    let version = match version {
        Some(v) => v,
        None => {
            let latest = current_version(&client, &vault_ref)?;
            if latest == 0 {
                return Err(anyhow!(
                    "{} has no versions yet. Push one with `evnx cloud push`.",
                    vault_ref.label()
                ));
            }
            latest
        }
    };

    let blob_bytes = client
        .get_bytes(&format!(
            "/api/v1/vaults/{}/versions/{version}/blob",
            vault_ref.id
        ))
        .map_err(|e| anyhow!("{e}"))?;

    if blob_bytes.len() <= NONCE_LEN {
        return Err(anyhow!(
            "the server returned {} bytes, too short to be a blob",
            blob_bytes.len()
        ));
    }
    let (nonce_bytes, ciphertext) = blob_bytes.split_at(NONCE_LEN);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_bytes);

    let password = auth::read_password(password_stdin, "Master password")?;
    let master_key = auth::derive_master_key_for_account(&client, &password)?;
    let vault_key = unwrap_vault_key(&client, &vault_ref, &master_key)?;

    let aad = vault_aad(&vault_ref.id, version as u32);
    let plaintext = decrypt_vault(
        &EncryptedBlob {
            nonce,
            ciphertext: ciphertext.to_vec(),
        },
        &vault_key,
        &aad,
    )
    .map_err(|_| {
        anyhow!(
            "could not decrypt version {version} of {}.\n\
             \x20 Either the master password is wrong, or the server served a different \
             version than it claimed — the version number is authenticated into the \
             ciphertext, so a substituted blob fails here rather than decrypting to \
             stale secrets.",
            vault_ref.label()
        )
    })?;

    if !force && !confirm_overwrite(&file, &plaintext)? {
        return Ok(());
    }

    super::creds::write_atomic_secure(&file, &plaintext)
        .with_context(|| format!("writing {}", file.display()))?;

    println!(
        "  {} pulled version {version} of {} into {}",
        "✓".green(),
        vault_ref.label(),
        file.display()
    );
    if verbose {
        println!("  bytes     {} plaintext", plaintext.len());
        println!("  mode      0600");
    }
    Ok(())
}

/// List a vault's versions, newest first.
///
/// Read-only and cheap: no password, no decryption, no blob download. It answers
/// "what is in here and when did it change" without opening anything.
pub fn history(
    server_override: Option<&str>,
    vault_target: Option<String>,
    limit: usize,
    verbose: bool,
) -> Result<()> {
    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let listed: VersionList = client
        .get(&format!("/api/v1/vaults/{}/versions", vault_ref.id))
        .map_err(|e| anyhow!("{e}"))?;

    if listed.versions.is_empty() {
        println!("  {} has no versions yet.", vault_ref.label());
        println!("  Push one with:  {}", "evnx cloud push".cyan());
        return Ok(());
    }

    // `pushed_by` is a user id and nothing can turn it into a name — the API has
    // no id-to-email lookup. For a vault you created it is always you, so say so
    // and fall back to a short id otherwise. Worth revisiting when sharing ships.
    let me: Option<String> = client
        .get::<MeId>("/api/v1/auth/me")
        .ok()
        .map(|m| m.user_id);

    let mut rows: Vec<&VersionSummary> = listed.versions.iter().collect();
    rows.sort_by_key(|v| std::cmp::Reverse(v.version_num));
    let shown = rows.len().min(limit);

    println!(
        "  {:>7}  {:>4}  {:>7}  {:<10}  {}",
        "VERSION".bold(),
        "KEYS".bold(),
        "SIZE".bold(),
        "PUSHED BY".bold(),
        "PUSHED".bold()
    );
    for v in rows.iter().take(shown) {
        let who = match &me {
            Some(id) if *id == v.pushed_by => "you".to_string(),
            _ => v.pushed_by.chars().take(8).collect(),
        };
        println!(
            "  {:>7}  {:>4}  {:>7}  {:<10}  {}",
            v.version_num,
            v.key_count,
            human_size(v.blob_size_bytes),
            who,
            v.pushed_at
                .replace('T', " ")
                .chars()
                .take(16)
                .collect::<String>()
        );
        if verbose {
            println!("           keys: {}", v.key_names.join(", "));
        }
    }

    if rows.len() > shown {
        println!(
            "  … {} older version(s). Use --limit to see more.",
            rows.len() - shown
        );
    }
    println!();
    println!(
        "  Restore one with:  {}",
        format!("evnx cloud pull --version {}", rows[0].version_num).cyan()
    );
    Ok(())
}

/// Bind this directory to a vault.
///
/// The vault is looked up before anything is written, so a typo fails here
/// rather than at the next push.
pub fn link(server_override: Option<&str>, vault_target: String, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let cwd = std::env::current_dir().context("reading the current directory")?;
    let canonical = format!("{}/{}", vault_ref.name, vault_ref.environment);
    let path = binding::write(&cwd, &canonical)?;

    println!("  {} bound this directory to {canonical}", "✓".green());
    println!("  written to {}", path.display());
    println!();
    println!("  `evnx cloud push` and `pull` now work here without --vault.");
    println!("  Safe to commit — a vault name is not a secret, and sharing it");
    println!("  means a teammate's pull lands in the same place.");
    if verbose {
        println!("  vault id  {}", vault_ref.id);
    }
    Ok(())
}

/// Remove this directory's binding.
pub fn unlink(verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    match binding::clear(&cwd)? {
        Some(path) => {
            println!("  {} removed the vault binding", "✓".green());
            if verbose {
                println!("  from {}", path.display());
            }
        }
        None => println!("  This directory has no vault binding."),
    }
    Ok(())
}

/// Bytes in a form a person reads at a glance.
fn human_size(bytes: i64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{:.1} MiB", b / (KIB * KIB))
    }
}

/// Latest version number, or 0 when the vault has never been pushed to.
///
/// The server answers 404 for a vault with no versions, which is not an error
/// here — it is the starting state.
pub(crate) fn current_version(client: &Client, vault_ref: &VaultRef) -> Result<i32> {
    match client.get::<LatestVersion>(&format!("/api/v1/vaults/{}/versions/latest", vault_ref.id)) {
        Ok(v) => Ok(v.version_num),
        Err(ApiError::NotFound { .. }) => Ok(0),
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// Fetch this account's wrapped copy of the vault key and open it.
///
/// Two shapes arrive here and the pair of optional fields is what distinguishes
/// them:
///
/// * **your own vault** — neither field. Wrapped under an HKDF subkey of the
///   master key, symmetric, no key agreement at all.
/// * **shared with you** — both fields. Wrapped by the hybrid
///   X25519 + ML-KEM-768 path, which needs the account keypair rather than just
///   the master key.
///
/// Anything else is a malformed row and is refused rather than guessed at.
pub(crate) fn unwrap_vault_key(
    client: &Client,
    vault_ref: &VaultRef,
    master_key: &evnx_crypto::MasterKey,
) -> Result<evnx_crypto::VaultKey> {
    use evnx_crypto::{b64_decode, unwrap_vault_key_with_master_key};

    let my_key: MyKey = client
        .get(&format!("/api/v1/vaults/{}/my-key", vault_ref.id))
        .map_err(|e| anyhow!("{e}"))?;

    match (&my_key.eph_pub_key, &my_key.mlkem_ciphertext) {
        // Shared with us: open it with the hybrid path.
        (Some(eph), Some(ct)) => unwrap_shared_vault_key(
            client,
            vault_ref,
            master_key,
            &my_key.encrypted_vault_key,
            eph,
            ct,
        ),

        // Our own copy.
        (None, None) => {
            let wrapped = b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
                .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;

            unwrap_vault_key_with_master_key(&wrapped, master_key).map_err(|_| {
                anyhow!(
                    "could not unwrap the key for {} — the master password is probably wrong.",
                    vault_ref.label()
                )
            })
        }

        // ⚠️ Half a wrap. An ephemeral with no ML-KEM ciphertext is a
        // pre-0.2.0 share: the vault key behind it was wrapped under X25519
        // alone, which Shor breaks. Refusing is the point — opening it would
        // mean carrying that exposure forward silently.
        (Some(_), None) => Err(anyhow!(
            "{} was shared with you before post-quantum wrapping existed, and the \n\
             key is wrapped in a way this version will not open.\n\
             \x20 Ask whoever shared it to run `evnx vault share` again.",
            vault_ref.label()
        )),
        (None, Some(_)) => Err(anyhow!(
            "the server sent a malformed key for {} — an ML-KEM ciphertext with no \n\
             ephemeral public key. Treat this server as untrusted.",
            vault_ref.label()
        )),
    }
}

/// Open a vault key that was shared with us — hybrid X25519 + ML-KEM-768.
///
/// Needs the account keypair, not just the master key, so it fetches and unseals
/// the Ed25519 seed. Both derived keys come from that one seed.
fn unwrap_shared_vault_key(
    client: &Client,
    vault_ref: &VaultRef,
    master_key: &evnx_crypto::MasterKey,
    encrypted_vault_key_b64: &str,
    eph_pub_key_b64: &str,
    mlkem_ciphertext_b64: &str,
) -> Result<evnx_crypto::VaultKey> {
    use evnx_crypto::{
        decrypt_private_key, unwrap_vault_key, EncryptedPrivateKey, WrappedVaultKey,
    };

    #[derive(Deserialize)]
    struct MeSeed {
        encrypted_private_key: String,
    }

    let me: MeSeed = client
        .get("/api/v1/auth/me")
        .map_err(|e| anyhow!("fetching your sealed keypair: {e}"))?;

    let sealed = EncryptedPrivateKey::from_base64(&me.encrypted_private_key)
        .map_err(|e| anyhow!("the server sent an unusable encrypted_private_key: {e}"))?;
    let keypair = decrypt_private_key(&sealed, master_key)
        .map_err(|_| anyhow!("could not open your keypair — the master password is wrong."))?;

    let wrapped = WrappedVaultKey::from_base64(
        encrypted_vault_key_b64,
        eph_pub_key_b64,
        mlkem_ciphertext_b64,
    )
    .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;

    unwrap_vault_key(&wrapped, &keypair).map_err(|_| {
        anyhow!(
            "could not open the key for {}.\n\
             \x20 The wrapped key does not verify. Either it was not wrapped for \n\
             \x20 this account, or the server altered it.",
            vault_ref.label()
        )
    })
}

/// Key names for the server's listing. Never values.
///
/// Returns the names plus an optional note when the file could not be parsed. A
/// parse failure is not fatal: the ciphertext carries the exact bytes either way,
/// and refusing to push a file the user explicitly named would be unhelpful.
fn read_key_names(plaintext: &[u8]) -> (Vec<String>, Option<String>) {
    let Ok(text) = std::str::from_utf8(plaintext) else {
        return (
            Vec::new(),
            Some("the file is not UTF-8, so no key names were recorded".into()),
        );
    };
    let parser = crate::core::parser::Parser::new(Default::default());
    match parser.parse_content(text) {
        Ok(vars) => (vars.keys().cloned().collect(), None),
        Err(e) => (
            Vec::new(),
            Some(format!(
                "could not parse the file for key names ({e}); the contents were pushed unchanged"
            )),
        ),
    }
}

/// Ask before replacing a file whose contents differ.
fn confirm_overwrite(file: &Path, incoming: &[u8]) -> Result<bool> {
    let existing = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };

    if existing == incoming {
        println!("  {} is already up to date.", file.display());
        return Ok(false);
    }

    println!(
        "  {} exists and differs from the version being pulled.",
        file.display()
    );
    let ok = dialoguer::Confirm::new()
        .with_prompt(format!("Overwrite {}?", file.display()))
        .default(false)
        .interact()
        .context("reading the confirmation")?;
    if !ok {
        println!("  Cancelled; {} was not changed.", file.display());
    }
    Ok(ok)
}

/// Turn a push failure into something actionable.
fn push_error(e: ApiError, based_on: i32) -> anyhow::Error {
    match e {
        ApiError::Conflict { message } => anyhow!(
            "{message}\n\
             \x20 You based this push on version {based_on}. Pull first, re-apply your \
             changes, then push again — the version number is authenticated into the \
             ciphertext, so the same bytes cannot simply be re-sent."
        ),
        other => anyhow!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_are_extracted_without_values() {
        let (names, note) = read_key_names(b"DATABASE_URL=postgres://secret\nPORT=8080\n");
        assert_eq!(names, vec!["DATABASE_URL", "PORT"]);
        assert!(note.is_none());
        // The whole point: names travel, values do not.
        assert!(!names.iter().any(|n| n.contains("secret")));
    }

    #[test]
    fn a_file_that_is_not_utf8_still_pushes_with_no_names() {
        let (names, note) = read_key_names(&[0xff, 0xfe, 0x00]);
        assert!(names.is_empty());
        assert!(note.unwrap().contains("not UTF-8"));
    }

    #[test]
    fn the_aad_binds_the_exact_vault_id_string() {
        use evnx_crypto::vault_aad;
        // Push and pull must derive the AAD from the same id spelling. A
        // hyphenated UUID and its simple form are different byte strings, so
        // mixing them would make every blob undecryptable.
        let hyphenated = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let simple = "7c9e667974254 0de944be07fc1f90ae7".replace(' ', "");
        assert_ne!(vault_aad(hyphenated, 1), vault_aad(&simple, 1));
        // And the version is part of it, which is what defeats a replay.
        assert_ne!(vault_aad(hyphenated, 1), vault_aad(hyphenated, 2));
    }

    #[test]
    fn a_round_trip_reproduces_the_file_byte_for_byte() {
        use evnx_crypto::{decrypt_vault, encrypt_vault, vault_aad, VaultKey};

        // Comments, blank lines, ordering, quoting and trailing whitespace all
        // have to survive — which is why push encrypts raw bytes rather than a
        // re-serialised parse.
        let original = b"# db\nDATABASE_URL=\"postgres://u:p@h/db?x=1\"\n\n\
                         # nothing below\nEMPTY=\nSPACED = value with spaces \n";
        let key = VaultKey::generate();
        let aad = vault_aad("vault-1", 7);

        let blob = encrypt_vault(original, &key, &aad).unwrap();
        let out = decrypt_vault(&blob, &key, &aad).unwrap();
        assert_eq!(out, original);
    }

    #[test]
    fn a_blob_sealed_for_one_version_will_not_open_as_another() {
        use evnx_crypto::{decrypt_vault, encrypt_vault, vault_aad, VaultKey};
        let key = VaultKey::generate();
        let blob = encrypt_vault(b"SECRET=1", &key, &vault_aad("v", 3)).unwrap();
        // A malicious server replaying version 3 as the current version 4 is
        // exactly what the AAD defeats.
        assert!(decrypt_vault(&blob, &key, &vault_aad("v", 4)).is_err());
        assert!(decrypt_vault(&blob, &key, &vault_aad("other", 3)).is_err());
        assert!(decrypt_vault(&blob, &key, &vault_aad("v", 3)).is_ok());
    }

    #[test]
    fn sizes_render_at_a_glance() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(2 * 1024 * 1024), "2.0 MiB");
    }

    #[test]
    fn an_explicit_vault_beats_any_binding() {
        // --vault must win even inside a bound directory, or overriding it would
        // be impossible without editing a file first.
        assert_eq!(
            resolve_target(Some("other/staging".into())).unwrap(),
            "other/staging"
        );
    }

    #[test]
    fn a_conflict_explains_that_re_encryption_is_required() {
        let err = push_error(
            ApiError::Conflict {
                message: "Remote is at version 5. Pull latest before pushing (you based on 3)."
                    .into(),
            },
            3,
        )
        .to_string();
        assert!(err.contains("Remote is at version 5"), "{err}");
        assert!(err.contains("cannot simply be re-sent"), "{err}");
    }
}

#[cfg(test)]
mod env_file_tests {
    use super::*;
    use tempfile::TempDir;

    fn project(files: &[&str]) -> TempDir {
        let d = TempDir::new().unwrap();
        for f in files {
            std::fs::write(d.path().join(f), "A=1\n").unwrap();
        }
        d
    }

    /// ⚠️ The bug this exists for: `--vault app/production` with a forgotten
    /// `--file` pushed local development secrets into the production vault.
    #[test]
    fn the_vault_environment_picks_the_file() {
        let d = project(&[".env", ".env.production"]);

        let (path, choice) = resolve_env_file(d.path(), None, None, "production").unwrap();

        assert!(path.ends_with(".env.production"), "{path:?}");
        assert_eq!(choice, FileChoice::MatchedEnvironment);
    }

    /// Most projects genuinely have only `.env`; refusing would break the common
    /// case. What makes it safe is that the caller prints the choice.
    #[test]
    fn a_missing_derived_file_falls_back_rather_than_failing() {
        let d = project(&[".env"]);

        let (path, choice) = resolve_env_file(d.path(), None, None, "production").unwrap();

        assert!(path.ends_with(".env"), "{path:?}");
        assert_eq!(choice, FileChoice::Default);
    }

    /// A vault addressed by id has no environment to derive from — a
    /// vault-scoped token cannot list vaults.
    #[test]
    fn an_unknown_environment_derives_nothing() {
        let d = project(&[".env", ".env.production"]);

        let (path, choice) = resolve_env_file(d.path(), None, None, "").unwrap();

        assert!(path.ends_with(".env"), "{path:?}");
        assert_eq!(choice, FileChoice::Default);
    }

    #[test]
    fn an_explicit_file_always_wins() {
        let d = project(&[".env", ".env.production"]);

        let (path, choice) = resolve_env_file(
            d.path(),
            Some(PathBuf::from("custom.env")),
            None,
            "production",
        )
        .unwrap();

        assert_eq!(path, PathBuf::from("custom.env"));
        assert_eq!(choice, FileChoice::Explicit);
    }

    /// `--env-name` keeps the strict contract of the local commands: a name that
    /// resolves to nothing is an error, not a fallback. The user named it.
    #[test]
    fn an_explicit_name_must_exist() {
        let d = project(&[".env", ".env.production"]);

        let (path, choice) =
            resolve_env_file(d.path(), None, Some("production"), "staging").unwrap();
        assert!(path.ends_with(".env.production"));
        assert_eq!(choice, FileChoice::Named);

        assert!(
            resolve_env_file(d.path(), None, Some("nope"), "production").is_err(),
            "a named environment that does not exist must fail"
        );
    }
}
