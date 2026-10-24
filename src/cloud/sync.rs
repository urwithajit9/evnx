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

use crate::cloud::client::api_path;
use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
            // stderr and hidden by `-q` (DEC-CL4, I-226): `cloud pull` to stdout
            // and `cloud run` print data there, and this line is not part of it.
            if !crate::utils::ui::is_quiet() {
                eprintln!(
                    "  using {} from {}",
                    bound.vault.bold(),
                    bound.path.display()
                );
            }
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
/// The default `.env` into a production vault: asked on a terminal, `--yes`
/// without one (I-234, DEC-CL9).
///
/// ⚠️ A forgotten `--file` pushed local development secrets into production —
/// encrypted, versioned, shared with everyone holding a key — with only the
/// warning `report_file` prints. CI cannot answer a question, so there it is a
/// refusal naming `--yes`; nothing has been read or sent yet.
///
/// By id the environment is not known: a vault-scoped token cannot list vaults.
/// A signed-in session can, and looks the id up for this question only — the
/// file stays `.env` (owner, 2026-10-25). A token by id is not asked.
fn confirm_production_push(
    client: &Client,
    vault_ref: &VaultRef,
    file: &Path,
    vault_target: &str,
) -> Result<bool> {
    let known = if vault_ref.environment.is_empty() && !client.is_api_token() {
        vault::resolve_id(client, &vault_ref.id).ok()
    } else {
        None
    };
    let (environment, label) = match &known {
        Some(v) => (v.environment.as_str(), v.label()),
        None => (vault_ref.environment.as_str(), vault_ref.label()),
    };
    if environment != "production" {
        return Ok(true);
    }
    crate::utils::prompt::require_terminal_or(&format!(
        "{} is the default file and {label} is a production vault, so this is asked \
         first. Name the file you mean (--file or --env-name), or add --yes:\n  \
         evnx cloud push --vault {vault_target} --yes",
        file.display()
    ))?;
    let ok = crate::utils::prompt::yes_no()
        .with_prompt(format!(
            "Push {} into production vault {label}?",
            file.display()
        ))
        .default(false)
        .interact_opt()?
        .unwrap_or(false);
    Ok(ok)
}

/// Whether `--file` names stdout: exactly `-`, as `template --output -` (DEC-T8).
fn is_stdout(file: Option<&Path>) -> bool {
    file.is_some_and(crate::core::safe_write::is_stdout)
}

fn report_file(file: &Path, choice: FileChoice, vault_env: &str, action: &str) {
    crate::say!("  file      {} ({})", file.display(), choice.note());

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
pub(crate) struct VersionList {
    pub versions: Vec<VersionSummary>,
}

#[derive(Deserialize)]
pub(crate) struct VersionSummary {
    pub version_num: i32,
    pub key_count: i32,
    pub key_names: Vec<String>,
    /// BLAKE3 of the ciphertext.
    ///
    /// ⚠️ This is what lets `cloud diff` say "same keys, contents differ"
    /// without being able to say *how* they differ. The server has never held a
    /// value, so two versions where only a value was rotated have identical
    /// `key_names` — and reporting "no changes" there would be a lie the user
    /// could not catch. The hash makes the difference visible while leaking
    /// nothing: it is a digest of ciphertext, not of plaintext.
    pub blob_hash: String,
    pub blob_size_bytes: i64,
    pub pushed_by: String,
    pub pushed_at: String,
}

#[derive(Deserialize)]
struct MeId {
    user_id: String,
}

/// Your user id, to tell your own pushes from a teammate's (`history`,
/// `watch`). `None` when `/auth/me` cannot be read; callers then treat every
/// push as someone else's.
pub(crate) fn my_user_id(client: &Client) -> Option<String> {
    client
        .get::<MeId>("/api/v1/auth/me")
        .ok()
        .map(|m| m.user_id)
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
    yes: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{b64_encode, encrypt_vault, vault_aad};

    let vault_target = resolve_target(vault_target)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;
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
    if choice == FileChoice::Default
        && !yes
        && !confirm_production_push(&client, &vault_ref, &file, &vault_target)?
    {
        println!("  Nothing was pushed.");
        return Ok(());
    }

    // Still read before the password prompt: a missing file should fail
    // instantly, not after a second of Argon2id.
    let plaintext = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
    if plaintext.is_empty() {
        return Err(anyhow!("{} is empty; nothing to push", file.display()));
    }

    let current = current_version(&client, &vault_ref)?;
    let target_version = current + 1;

    let password = auth::read_password(password_stdin, "Master password")?;
    let unlocked = auth::unlock(&client, &password)?;
    let vault_key = unwrap_vault_key(&client, &vault_ref, &unlocked)?;

    let (key_names, parse_note) = read_key_names(&plaintext);

    let aad = vault_aad(&vault_ref.id, target_version as u32);
    let blob = encrypt_vault(&plaintext, &vault_key, &aad)
        .map_err(|e| anyhow!("encrypting {}: {e}", file.display()))?;
    let blob_hash = blake3::hash(&blob.ciphertext).to_hex().to_string();

    let pushed: PushVersionResponse = client
        .post(
            &api_path(&["vaults", &vault_ref.id.to_string(), "versions"])?,
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

    crate::say!(
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
    gitignore: crate::utils::protect::GitignoreMode,
    verbose: bool,
) -> Result<()> {
    let vault_target = resolve_target(vault_target)?;

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    // ⚠️ On pull the derivation only fires when the file already exists, so a
    // first pull into a fresh checkout still writes `.env`. That is the right
    // default: pull must not invent `.env.production` in a project that has no
    // such convention. Name it explicitly to create one.
    // `--file -` is stdout (I-244, DEC-U8-4): it wrote a file named `-`,
    // holding plaintext, in the working directory.
    let to_stdout = is_stdout(file.as_deref());
    let (file, choice) = resolve_env_file(
        Path::new("."),
        file,
        env_name.as_deref(),
        &vault_ref.environment,
    )?;
    if to_stdout {
        // Status on stderr: stdout carries the file and nothing else.
        if !crate::utils::ui::is_quiet() {
            eprintln!("  file      - (stdout)");
        }
    } else {
        report_file(&file, choice, &vault_ref.environment, "writing");
    }

    let version = match version {
        Some(v) => v,
        None => {
            let latest = current_version(&client, &vault_ref)?;
            if latest == 0 {
                return Err(anyhow!(
                    "{} has no versions yet. Push one with `evnx cloud push --vault {}`.",
                    vault_ref.label(),
                    vault_ref.label()
                ));
            }
            latest
        }
    };

    // ⚠️ The blob is fetched **before** the password is asked for, and that
    // ordering is deliberate: a version that does not exist should answer 404
    // rather than waste the typing. The split into two functions below is what
    // lets the order survive the extraction.
    let blob = fetch_blob(&client, &vault_ref, version)?;

    let password = auth::read_password(password_stdin, "Master password")?;
    let unlocked = auth::unlock(&client, &password)?;
    let vault_key = unwrap_vault_key(&client, &vault_ref, &unlocked)?;

    let plaintext = decrypt_version(&blob, &vault_ref, version, &vault_key)?;

    if to_stdout {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        out.write_all(&plaintext).context("writing to stdout")?;
        out.flush().context("writing to stdout")?;
        if !crate::utils::ui::is_quiet() {
            eprintln!(
                "  {} pulled version {version} of {} to stdout",
                "✓".green(),
                vault_ref.label()
            );
        }
        return Ok(());
    }

    if !force && !confirm_overwrite(&file, &plaintext)? {
        return Ok(());
    }

    super::creds::write_atomic_secure(&file, &plaintext)
        .with_context(|| format!("writing {}", file.display()))?;

    crate::say!(
        "  {} pulled version {version} of {} into {}",
        "✓".green(),
        vault_ref.label(),
        file.display()
    );
    if verbose {
        println!("  bytes     {} plaintext", plaintext.len());
        // ⚠️ Not the literal "0600". Nothing sets a mode on Windows, where
        // `write_secure` is a plain `fs::write`, so printing it there is a
        // reassurance that is simply false.
        println!(
            "  mode      {}",
            crate::utils::file_permissions::describe_permissions()
        );
    }
    // Plaintext secrets in the project: keep them out of git, the way
    // `template`, `convert` and `restore` do (owner, 2026-10-17).
    crate::utils::protect::protect_output(&file.to_string_lossy(), &gitignore)?;
    Ok(())
}

/// Every version of a vault, in whatever order the server returned them.
///
/// Read-only and cheap — metadata only, no blobs and no key. `history` and
/// `export` both go through here so "what versions exist" has one answer.
pub(crate) fn list_versions(client: &Client, vault_ref: &VaultRef) -> Result<Vec<VersionSummary>> {
    let listed: VersionList = client
        .get(&api_path(&[
            "vaults",
            &vault_ref.id.to_string(),
            "versions",
        ])?)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(listed.versions)
}

/// Download one version's blob and split off its nonce.
///
/// No key is needed and none is touched: this is ciphertext in and ciphertext
/// out. Keeping the download separate from [`decrypt_version`] is what lets
/// `pull` fetch before it asks for a password — and what lets `cloud export`
/// open the vault key **once** and then walk every version.
pub(crate) fn fetch_blob(
    client: &Client,
    vault_ref: &VaultRef,
    version: i32,
) -> Result<evnx_crypto::EncryptedBlob> {
    let blob_bytes = client
        .get_bytes(&api_path(&[
            "vaults",
            &vault_ref.id.to_string(),
            "versions",
            &version.to_string(),
            "blob",
        ])?)
        .map_err(|e| match e {
            // I-303 (DEC-CL11): this read "not found on …: /api/v1/vaults/<uuid>/…".
            ApiError::NotFound { .. } => anyhow!(
                "version {version} of {label} does not exist.\n\
                 \x20 `evnx cloud history --vault {label}` lists the versions it has.",
                label = vault_ref.label()
            ),
            other => anyhow!("{other}"),
        })?;

    if blob_bytes.len() <= NONCE_LEN {
        return Err(anyhow!(
            "the server returned {} bytes, too short to be a blob",
            blob_bytes.len()
        ));
    }
    let (nonce_bytes, ciphertext) = blob_bytes.split_at(NONCE_LEN);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_bytes);

    Ok(evnx_crypto::EncryptedBlob {
        nonce,
        ciphertext: ciphertext.to_vec(),
    })
}

/// Decrypt a blob that was fetched as `version` of `vault_ref`.
///
/// ⚠️ **`version` must be the number the blob was fetched under.** It goes into
/// the AAD, so passing the wrong one does not decrypt to the wrong secrets — it
/// fails. That is the property worth preserving in any caller that loops: an
/// off-by-one pairing version *n*'s ciphertext with *n+1*'s AAD is caught here
/// rather than producing a plausible-looking file.
pub(crate) fn decrypt_version(
    blob: &evnx_crypto::EncryptedBlob,
    vault_ref: &VaultRef,
    version: i32,
    vault_key: &evnx_crypto::VaultKey,
) -> Result<Vec<u8>> {
    use evnx_crypto::{decrypt_vault, vault_aad};

    let aad = vault_aad(&vault_ref.id, version as u32);
    decrypt_vault(blob, vault_key, &aad).map_err(|_| {
        anyhow!(
            "could not decrypt version {version} of {}.\n\
             \x20 The server served a different version than it claimed, or altered \
             it — the version number is authenticated into the ciphertext, so a \
             substituted blob fails here rather than decrypting to stale secrets.",
            vault_ref.label()
        )
    })
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
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let versions = list_versions(&client, &vault_ref)?;

    if versions.is_empty() {
        println!("  {} has no versions yet.", vault_ref.label());
        crate::say!(
            "  Push one with:  {}",
            format!("evnx cloud push --vault {}", vault_ref.label()).cyan()
        );
        return Ok(());
    }

    // `pushed_by` is a user id. You are "you"; anyone else is named by email
    // from the member list (I-319) — it showed an 8-character id. Someone who
    // has since left is no longer listed and keeps the short id.
    let me = my_user_id(&client);
    let emails = vault::member_emails(&client, &vault_ref.id);

    let mut rows: Vec<&VersionSummary> = versions.iter().collect();
    rows.sort_by_key(|v| std::cmp::Reverse(v.version_num));
    let shown = rows.len().min(limit);
    let who = |v: &VersionSummary| -> String {
        match (&me, emails.get(&v.pushed_by)) {
            (Some(id), _) if *id == v.pushed_by => "you".to_string(),
            (_, Some(email)) => email.clone(),
            _ => v.pushed_by.chars().take(8).collect(),
        }
    };
    let width = rows
        .iter()
        .take(shown)
        .map(|v| who(v).chars().count())
        .chain(std::iter::once(10))
        .max()
        .unwrap_or(10);

    println!(
        "  {:>7}  {:>4}  {:>7}  {:<width$}  {}",
        "VERSION".bold(),
        "KEYS".bold(),
        "SIZE".bold(),
        "PUSHED BY".bold(),
        "PUSHED".bold()
    );
    for v in rows.iter().take(shown) {
        println!(
            "  {:>7}  {:>4}  {:>7}  {:<width$}  {}",
            v.version_num,
            v.key_count,
            human_size(v.blob_size_bytes),
            who(v),
            super::when::stamp(&v.pushed_at)
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
    crate::say!();
    crate::say!(
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
    client.require_credential()?;
    let mut vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;
    // ⚠️ An id skips the lookup, so name and environment come back empty, and
    // this wrote `vault = "/"` (I-221, DEC-CL2). Look the id up; a binding is
    // read by people, and `app/production` says what `/` did not.
    if vault_ref.name.is_empty() {
        vault_ref = vault::resolve_id(&client, &vault_ref.id)?;
    }

    let cwd = std::env::current_dir().context("reading the current directory")?;
    let canonical = format!("{}/{}", vault_ref.name, vault_ref.environment);
    let path = binding::write(&cwd, &canonical)?;

    // `-q` (DEC-CL3, I-225): silent on success.
    use crate::utils::ui::say;
    say(format!(
        "  {} bound this directory to {canonical}",
        "✓".green()
    ));
    say(format!("  written to {}", path.display()));
    say("");
    say("  `evnx cloud push` and `pull` now work here without --vault.");
    say("  Safe to commit — a vault name is not a secret, and sharing it");
    say("  means a teammate's pull lands in the same place.");
    if verbose {
        println!("  vault id  {}", vault_ref.id);
    }
    Ok(())
}

/// Remove this directory's binding.
pub fn unlink(verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    match binding::clear(&cwd)? {
        // `-q` (DEC-CL3, I-225): silent on success. "No binding" is the
        // answer to what the user asked for, so it stays.
        Some(path) => {
            crate::utils::ui::say(format!("  {} removed the vault binding", "✓".green()));
            if verbose {
                crate::utils::ui::say(format!("  from {}", path.display()));
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
    match client.get::<LatestVersion>(&api_path(&[
        "vaults",
        &vault_ref.id.to_string(),
        "versions",
        "latest",
    ])?) {
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
    unlocked: &auth::Unlocked,
) -> Result<evnx_crypto::VaultKey> {
    use evnx_crypto::{b64_decode, unwrap_vault_key_with_master_key};

    let my_key: MyKey = client
        .get(&api_path(&["vaults", &vault_ref.id.to_string(), "my-key"])?)
        .map_err(|e| anyhow!("{e}"))?;

    match (&my_key.eph_pub_key, &my_key.mlkem_ciphertext) {
        // Shared with us: open it with the hybrid path.
        (Some(eph), Some(ct)) => unwrap_shared_vault_key(
            vault_ref,
            &unlocked.keypair,
            &my_key.encrypted_vault_key,
            eph,
            ct,
        ),

        // Our own copy.
        (None, None) => {
            let wrapped = b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
                .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;

            unwrap_vault_key_with_master_key(&wrapped, &unlocked.master_key)
                .map_err(|_| auth::own_copy_will_not_open(&vault_ref.label()))
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
/// Needs the account keypair, not just the master key: both derived keys come
/// from the one Ed25519 seed, which [`auth::unlock`] has already opened.
fn unwrap_shared_vault_key(
    vault_ref: &VaultRef,
    keypair: &evnx_crypto::UserKeypair,
    encrypted_vault_key_b64: &str,
    eph_pub_key_b64: &str,
    mlkem_ciphertext_b64: &str,
) -> Result<evnx_crypto::VaultKey> {
    use evnx_crypto::{unwrap_vault_key, WrappedVaultKey};

    let wrapped = WrappedVaultKey::from_base64(
        encrypted_vault_key_b64,
        eph_pub_key_b64,
        mlkem_ciphertext_b64,
    )
    .map_err(|e| anyhow!("the server sent an unusable wrapped key: {e}"))?;

    unwrap_vault_key(&wrapped, keypair).map_err(|_| {
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
        crate::say!("  {} is already up to date.", file.display());
        return Ok(false);
    }

    println!(
        "  {} exists and differs from the version being pulled.",
        file.display()
    );
    // No terminal: say how to go on (I-218), as every other cloud prompt does.
    // `pull` says `--force`, not `--yes`.
    crate::utils::prompt::require_terminal_or(&format!(
        "Add --force to replace {} without being asked.",
        file.display()
    ))?;
    let ok = crate::utils::prompt::yes_no()
        .with_prompt(format!("Overwrite {}?", file.display()))
        .default(false)
        .interact_opt()?
        .unwrap_or(false);
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

    /// I-244, DEC-U8-4: exactly `-` is stdout; a path that merely contains a
    /// dash, or `./-`, is a file.
    #[test]
    fn t_i244_only_a_bare_dash_means_stdout() {
        assert!(is_stdout(Some(Path::new("-"))));
        for f in ["./-", "-.env", ".env", "a-b"] {
            assert!(!is_stdout(Some(Path::new(f))), "{f}");
        }
        assert!(!is_stdout(None));
    }

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

// ─── Deleting a version ───────────────────────────────────────────────────────

/// Remove one version of a vault.
///
/// ⚠️ This exists because the server's version quota refuses a push with *"Delete
/// an older version to make room"*, and for a while there was no way to do that —
/// no endpoint, no command. An error that advises an impossible action is worse
/// than one admitting there is nothing to be done, because it sends people looking
/// for a door that is not there.
///
/// The latest version cannot be deleted, by the server and by the check below.
/// `cloud pull` and `cloud run` fetch the latest, so removing it would silently
/// change what every consumer of the vault receives — the next deploy would pick up
/// older secrets with nobody having asked. The check here is a courtesy that saves
/// a round trip; the server is what enforces it.
pub fn delete_version(
    server_override: Option<&str>,
    vault_target: Option<String>,
    version_num: i32,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_user_session(
        "delete a version",
        "\x20 A deleted version and its blob cannot be recovered; a leaked CI token\n\x20 must not be able to erase history.\n",
    )?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let listed: VersionList = client
        .get(&api_path(&[
            "vaults",
            &vault_ref.id.to_string(),
            "versions",
        ])?)
        .map_err(|e| anyhow!("{e}"))?;

    let latest = listed
        .versions
        .iter()
        .map(|v| v.version_num)
        .max()
        .ok_or_else(|| anyhow!("{} has no versions to delete.", vault_ref.label()))?;

    let target = listed
        .versions
        .iter()
        .find(|v| v.version_num == version_num)
        .ok_or_else(|| {
            anyhow!(
                "{} has no version {version_num}. Run `evnx cloud history --vault {}` \
                 to see what it does have.",
                vault_ref.label(),
                vault_ref.label()
            )
        })?;

    if version_num == latest {
        return Err(anyhow!(
            "version {version_num} is the latest version of {}, and deleting it would \
             change what `evnx cloud pull` and `evnx cloud run` return without anyone \
             asking for that.\n\
             \x20 Push a newer version first, or delete the whole vault with `evnx vault \
             delete`. Nothing has been changed.",
            vault_ref.label()
        ));
    }

    crate::say!();
    println!("  {}", "This deletes one version, permanently.".yellow());
    crate::say!("  vault     {}", vault_ref.label());
    crate::say!("  version   {version_num}");
    crate::say!(
        "  contents  {} key(s), {}, pushed {}",
        target.key_count,
        human_size(target.blob_size_bytes),
        target.pushed_at
    );
    crate::say!(
        "  {}",
        "The encrypted blob is removed from storage. The server never held anything \
         that could rebuild it, so there is no undo."
            .dimmed()
    );
    crate::say!();

    if !crate::utils::prompt::confirm(
        &format!("  Delete version {version_num}"),
        yes,
        &format!("evnx cloud delete-version {version_num} --vault {vault_target} --yes"),
    )? {
        println!("  Nothing was deleted.");
        return Ok(());
    }

    client
        .delete(&api_path(&[
            "vaults",
            &vault_ref.id.to_string(),
            "versions",
            &version_num.to_string(),
        ])?)
        .map_err(delete_version_error)?;

    crate::say!("  {} deleted version {version_num}", "✓".green());
    if verbose {
        println!("  latest is still {latest}");
    }
    Ok(())
}

/// Explain a refusal in terms of the thing that was refused.
fn delete_version_error(e: super::client::ApiError) -> anyhow::Error {
    use super::client::ApiError;
    match e {
        // The server checks the role; a developer can push but not remove history.
        ApiError::Forbidden { message } => anyhow!(
            "{message}\n\
             \x20 Deleting a version needs admin on the vault. A developer can push \
             new versions but not remove old ones."
        ),
        ApiError::Conflict { message } => anyhow!("{message}"),
        other => anyhow!("{other}"),
    }
}

// ─── key-name comparison, shared by `diff` and `watch` ───────────────────────

/// What changed between two versions' key sets.
///
/// ⚠️ **Names only.** The server has never held a value, so this cannot report
/// that a secret was rotated — `changed_contents` is the nearest honest answer,
/// and it comes from the blob hash rather than from anything about the content.
pub(crate) struct KeyDelta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
    /// Same key names, different ciphertext: a value moved and we cannot say which.
    pub changed_contents: bool,
}

pub(crate) fn key_delta(before: &VersionSummary, after: &VersionSummary) -> KeyDelta {
    let b: BTreeSet<&str> = before.key_names.iter().map(String::as_str).collect();
    let a: BTreeSet<&str> = after.key_names.iter().map(String::as_str).collect();
    KeyDelta {
        added: a.difference(&b).map(|s| (*s).to_string()).collect(),
        removed: b.difference(&a).map(|s| (*s).to_string()).collect(),
        unchanged: b.intersection(&a).count(),
        changed_contents: before.blob_hash != after.blob_hash,
    }
}

/// `+A, +B, -C` — one line, for a notification rather than a report.
pub(crate) fn key_delta_line(d: &KeyDelta) -> String {
    let mut parts: Vec<String> = Vec::new();
    for k in &d.added {
        parts.push(format!("+{k}"));
    }
    for k in &d.removed {
        parts.push(format!("-{k}"));
    }
    if parts.is_empty() {
        // ⚠️ Never "no changes" — the names matching does not mean nothing moved.
        return if d.changed_contents {
            "same keys, different contents".to_string()
        } else {
            "identical".to_string()
        };
    }
    parts.join(", ")
}

// ─── cloud diff ──────────────────────────────────────────────────────────────

/// What changed between two versions of a vault — **key names only**.
///
/// ─── ⚠️ What this command can and cannot tell you ───────────────────────────
///
/// The server has never held a value. `vault_versions.key_names` is the entire
/// basis for this comparison, so:
///
///   * a key that appeared or disappeared is **visible**;
///   * a key whose *value* changed is **invisible** by name alone.
///
/// Rotating `DATABASE_URL` and changing nothing else produces two versions with
/// identical key sets. Reporting "no changes" there would be the worst kind of
/// wrong — confidently incorrect about the thing the user asked. So when the
/// names match, the **blob hashes** are compared and the answer becomes "the
/// same keys, with different contents", which is true, useful, and still leaks
/// nothing: a BLAKE3 digest of ciphertext says only *that* bytes differ.
///
/// ⚠️ **This is deliberately not `evnx diff`'s vocabulary.** The local command
/// compares a `.env` against a template and uses `+` for *missing from .env*.
/// Here `+` means *added in the newer version* — the opposite direction. Two
/// commands whose `+` disagreed would be a trap, so this one always prints the
/// direction it is reading (`v5 → v7`) above the list, and never borrows the
/// local command's `~` marker, which it has no way to compute.
///
/// Costs one request. No blob is downloaded, nothing is decrypted, and the
/// master password is never needed — the same reason `cloud history` does not
/// ask for one.
pub fn diff(
    server_override: Option<&str>,
    vault_target: Option<String>,
    from: Option<i32>,
    to: Option<i32>,
    verbose: bool,
) -> Result<()> {
    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let mut versions = list_versions(&client, &vault_ref)?;
    versions.sort_by_key(|v| v.version_num);

    if versions.len() < 2 {
        println!(
            "  {} has {} version{} — there is nothing to compare.",
            vault_ref.label(),
            versions.len(),
            if versions.len() == 1 { "" } else { "s" }
        );
        if versions.len() == 1 {
            crate::say!(
                "  Push another with:  {}",
                format!("evnx cloud push --vault {}", vault_ref.label()).cyan()
            );
        }
        return Ok(());
    }

    // Defaults compare the two newest, which is the question people actually
    // have: "what did that last push change?"
    let newest = versions[versions.len() - 1].version_num;
    let previous = versions[versions.len() - 2].version_num;
    let from_num = from.unwrap_or(previous);
    let to_num = to.unwrap_or(newest);

    let pick = |n: i32| -> Result<&VersionSummary> {
        versions.iter().find(|v| v.version_num == n).ok_or_else(|| {
            let have: Vec<String> = versions.iter().map(|v| v.version_num.to_string()).collect();
            anyhow!(
                "{} has no version {n}. It has: {}",
                vault_ref.label(),
                have.join(", ")
            )
        })
    };
    let a = pick(from_num)?;
    let b = pick(to_num)?;

    if from_num == to_num {
        return Err(anyhow!(
            "--from and --to are both v{from_num}; there is nothing to compare"
        ));
    }

    // ⚠️ Through the shared helper, not a second copy. `watch` reports the same
    // comparison in one line, and two implementations of "what changed" would
    // eventually disagree about an edge case in exactly the place that matters.
    let delta = key_delta(a, b);
    let added = &delta.added;
    let removed = &delta.removed;

    // ⚠️ Direction is stated, not implied. `+` means "added in v{to}" here and
    // "missing from .env" in `evnx diff`, and the header is what keeps the two
    // from being confused.
    println!();
    println!(
        "  {}  {}   v{} → v{}",
        "evnx cloud diff".bold(),
        vault_ref.label(),
        from_num,
        to_num
    );
    println!();

    if added.is_empty() && removed.is_empty() {
        if a.blob_hash == b.blob_hash {
            println!("  Identical. Same keys, same contents.");
        } else {
            // The case that makes this command honest rather than reassuring.
            println!("  {}", "Same keys — but the contents differ.".yellow());
            println!();
            println!(
                "  Both versions hold the same {} key{}, so something changed inside a",
                b.key_count,
                if b.key_count == 1 { "" } else { "s" }
            );
            println!("  value. evnx cannot tell you which: the server stores key names and");
            println!("  ciphertext, never a value, so there is nothing here to compare.");
            println!();
            println!("  To see it, pull both and diff them locally — on your machine, where the",);
            println!("  values exist:");
            println!(
                "      {}",
                format!(
                    "evnx cloud export --vault {} --all-versions --output ./versions",
                    vault_ref.label()
                )
                .cyan()
            );
        }
    } else {
        for k in added {
            println!("  {} {}", "+".green().bold(), k.green());
        }
        for k in removed {
            println!("  {} {}", "-".red().bold(), k.red());
        }
        println!();
        println!(
            "  {} added  ·  {} removed  ·  {} unchanged",
            added.len(),
            removed.len(),
            delta.unchanged
        );

        // ⚠️ Said even when names changed. A push that both adds a key and
        // rotates another would otherwise look like it only added one.
        if delta.changed_contents && delta.unchanged > 0 {
            println!();
            println!(
                "  {}",
                "Values may also have changed in the keys both versions share —".dimmed()
            );
            println!("  {}", "evnx cannot see values, so it cannot say.".dimmed());
        }
    }

    if verbose {
        println!();
        println!(
            "  v{}  {} keys  pushed {}",
            from_num, a.key_count, a.pushed_at
        );
        println!(
            "  v{}  {} keys  pushed {}",
            to_num, b.key_count, b.pushed_at
        );
    }

    crate::say!("{}", crate::docs::CLOUD.hint_line());
    Ok(())
}
