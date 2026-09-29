// src/cloud/rotate.rs

//! Changing the master password — and undoing it.
//!
//! ## Why this is not a settings toggle
//!
//! The master key is derived from the password, so changing the password
//! re-derives the key and **every vault key wrapped under it has to be re-wrapped
//! in the same breath**. All of that happens here; the server performs an atomic
//! swap of blobs it cannot read.
//!
//! ## The one job only this side can do
//!
//! ⚠️ **The server cannot tell a correct rotation from random bytes.** That is
//! what zero knowledge means, and it is not a gap to be closed — it is the
//! guarantee. The server can check that a payload is *complete* (it knows how
//! many wraps the account holds) but never that it is *correct*.
//!
//! So the check that a wrap actually opens belongs here, and it is mandatory
//! rather than a flag: after re-wrapping, this unwraps its own output with the
//! new master key and asserts byte-equality with what it started from. A bug
//! that produced a wrong-but-well-formed wrap would otherwise be discovered by a
//! user who can no longer open anything, with no way back.
//!
//! ## And the recovery the server cannot hold
//!
//! Zero knowledge means the server can never be the recovery path. So before
//! anything is sent, the **old** wraps are written to a `0600` file on this
//! machine. They are still ciphertext and still useless without the old
//! password, so this weakens nothing — but it is a complete offline undo that
//! does not depend on the server keeping anything.
//!
//! The server keeps its own 72-hour window as well, restorable by proving the old
//! password. Two independent ways back, for an operation that has no third.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::auth;
use super::client::{ApiError, Client};
use super::config::CloudConfig;

// ─── Wire types ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Me {
    email: String,
    argon2_salt: String,
    encrypted_private_key: String,
    totp_enabled: bool,
}

#[derive(Deserialize)]
struct VaultList {
    vaults: Vec<VaultRow>,
}

#[derive(Deserialize)]
struct VaultRow {
    id: String,
    name: String,
    environment: String,
}

#[derive(Deserialize)]
struct MyWrappedKey {
    encrypted_vault_key: String,
    #[serde(default)]
    eph_pub_key: Option<String>,
    #[serde(default)]
    mlkem_ciphertext: Option<String>,
}

#[derive(Serialize)]
struct ChallengeInit {
    client_public: String,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    session_id: String,
    srp_salt: String,
    #[allow(dead_code)]
    argon2_salt: String,
    server_public: String,
}

#[derive(Serialize)]
struct ReauthVerify {
    session_id: String,
    client_proof: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    totp_code: Option<String>,
}

#[derive(Deserialize)]
struct ProofResponse {
    server_proof: String,
}

#[derive(Serialize)]
struct VaultWrap {
    vault_id: String,
    encrypted_vault_key: String,
}

#[derive(Serialize)]
struct RotateRequest {
    srp_salt: String,
    srp_verifier: String,
    argon2_salt: String,
    encrypted_private_key: String,
    vault_wraps: Vec<VaultWrap>,
    reason: &'static str,
}

#[derive(Deserialize)]
struct RotateResponse {
    status: String,
    vaults_rewrapped: usize,
    sessions_revoked: usize,
    undo_available_until: Option<String>,
}

#[derive(Serialize)]
struct UndoInit {
    email: String,
    client_public: String,
}

#[derive(Serialize)]
struct UndoVerify {
    session_id: String,
    client_proof: String,
}

#[derive(Deserialize)]
struct UndoResponse {
    server_proof: String,
    vaults_restored: usize,
    vaults_not_in_snapshot: Vec<String>,
    sessions_revoked: usize,
}

/// What is written to disk before anything is sent.
///
/// ⚠️ Every field is ciphertext or a salt. Nothing here opens without the **old**
/// master password, so the file is no more sensitive than what the server already
/// stores — which is what makes it safe to write and worth writing.
#[derive(Serialize)]
struct RecoveryBundle {
    evnx_recovery: &'static str,
    written_at: String,
    server: String,
    email: String,
    note: &'static str,
    prev_argon2_salt: String,
    prev_encrypted_private_key: String,
    prev_vault_wraps: Vec<BundledWrap>,
}

#[derive(Serialize)]
struct BundledWrap {
    vault_id: String,
    label: String,
    encrypted_vault_key: String,
}

// ─── Rotate ───────────────────────────────────────────────────────────────────

/// One vault key, opened, with enough context to talk about it.
struct OpenedVault {
    id: String,
    label: String,
    old_wrap_b64: String,
    key: evnx_crypto::VaultKey,
}

/// Change the master password.
pub fn rotate(
    server_override: Option<&str>,
    password_stdin: bool,
    compromised: bool,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{
        b64_decode, b64_encode, compute_verifier, decrypt_private_key, derive_master_key,
        derive_srp_password, encrypt_private_key, generate_salt, salt_from_base64, salt_to_base64,
        unwrap_vault_key_with_master_key, wrap_vault_key_with_master_key, EncryptedPrivateKey,
    };

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    super::vault::require_session(&client, &server)?;

    let me: Me = client
        .get("/api/v1/auth/me")
        .map_err(|e| anyhow!("could not read the account: {e}"))?;

    println!();
    println!("  {}", "Changing your evnx master password.".bold());
    println!("  account   {}", me.email);
    println!();

    // ── Open everything with the current password ───────────────────────────
    //
    // Done before a single byte is sent. If the password is wrong, this fails
    // here — locally, having changed nothing anywhere.
    let current = auth::read_password(password_stdin, "Current master password")?;

    let old_salt = salt_from_base64(&me.argon2_salt)
        .map_err(|e| anyhow!("the server sent an unusable argon2_salt: {e}"))?;
    println!("  Deriving your current key on this machine…");
    let old_master = derive_master_key(current.as_bytes(), &old_salt)
        .map_err(|e| anyhow!("deriving the master key: {e}"))?;

    let sealed = EncryptedPrivateKey::from_base64(&me.encrypted_private_key)
        .map_err(|e| anyhow!("the server sent an unusable encrypted_private_key: {e}"))?;
    let keypair = decrypt_private_key(&sealed, &old_master).map_err(|_| {
        anyhow!(
            "that master password did not open your keypair, so it is not the one this \
             account was registered with. Nothing has been changed.\n\
             \x20 If you have already changed it on another machine, use the new one."
        )
    })?;

    // ── Every vault key wrapped under the master key ────────────────────────
    //
    // ⚠️ One request per vault. `GET /vaults` does not say which wrap mode a
    // membership uses — only `/my-key` does, by whether the key-agreement fields
    // are present — and the server demands exactly the master-key-wrapped set.
    // Guessing from the role would be wrong: an owner can also hold a vault that
    // was shared to them.
    let list: VaultList = client
        .get("/api/v1/vaults")
        .map_err(|e| anyhow!("could not list your vaults: {e}"))?;

    let mut opened: Vec<OpenedVault> = Vec::new();
    let mut shared = 0usize;

    for v in &list.vaults {
        let label = format!("{}/{}", v.name, v.environment);
        let my_key: MyWrappedKey = client
            .get(&format!("/api/v1/vaults/{}/my-key", v.id))
            .map_err(|e| anyhow!("could not read your key for {label}: {e}"))?;

        match (&my_key.eph_pub_key, &my_key.mlkem_ciphertext) {
            // Wrapped under the master key — this is the set that moves.
            (None, None) => {
                let wrapped = b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
                    .map_err(|e| anyhow!("unusable wrapped key for {label}: {e}"))?;
                let key = unwrap_vault_key_with_master_key(&wrapped, &old_master)
                    .map_err(|_| anyhow!("could not open your key for {label}."))?;
                opened.push(OpenedVault {
                    id: v.id.clone(),
                    label,
                    old_wrap_b64: my_key.encrypted_vault_key,
                    key,
                });
            }
            // Shared to this account, wrapped to the keypair. The keypair is
            // re-sealed rather than replaced, so these are untouched — and must
            // not be sent, or the server refuses the whole rotation.
            (Some(_), Some(_)) => shared += 1,
            _ => {
                return Err(anyhow!(
                    "the server returned half a key wrap for {label}. Nothing has been \
                     changed. This should be impossible — a database constraint refuses \
                     that shape — so please report it."
                ))
            }
        }
    }

    // ── Say what this will do, before it does it ────────────────────────────
    println!();
    println!("  {} vault key(s) will be re-wrapped", opened.len());
    if verbose {
        for v in &opened {
            println!("      {}", v.label.dimmed());
        }
    }
    if shared > 0 {
        println!(
            "  {} vault(s) shared with you are unaffected {}",
            shared,
            "(wrapped to your keypair, which does not change)".dimmed()
        );
    }
    println!("  every other signed-in device will be signed out");

    if let Some(n) = live_token_count(&client) {
        if n > 0 {
            println!();
            println!(
                "  {} {n} API token(s) keep working, but any CI holding your master",
                "!".yellow()
            );
            println!(
                "    password as a secret will fail until that secret is updated. {}",
                "A token authenticates; it does not decrypt.".dimmed()
            );
        }
    }

    if compromised {
        println!();
        println!(
            "  {} no undo will be kept. This takes effect immediately and cannot be \
             reversed.",
            "--compromised:".yellow()
        );
    } else {
        println!();
        println!(
            "  {}",
            "An undo stays available for a limited window, by proving your OLD password.".dimmed()
        );
    }
    println!();

    if !yes
        && !password_stdin
        && !dialoguer::Confirm::new()
            .with_prompt("  Continue")
            .default(false)
            .interact()
            .context("reading the confirmation")?
    {
        println!("  Nothing was changed.");
        return Ok(());
    }

    // ── The new password ────────────────────────────────────────────────────
    let next = if password_stdin {
        // Second line of stdin. Documented on the flag: current, then new.
        auth::read_password(true, "New master password")?
    } else {
        auth::prompt_new_password()?
    };
    auth::check_password_strength(&next)?;
    if next.as_bytes() == current.as_bytes() {
        return Err(anyhow!(
            "the new master password is the same as the current one. Nothing has been changed."
        ));
    }

    // ── Re-derive and re-wrap ───────────────────────────────────────────────
    println!("  Deriving your new key on this machine…");
    let new_srp_salt = generate_salt();
    let new_argon2_salt = generate_salt();

    let srp_password = derive_srp_password(next.as_bytes(), &new_srp_salt)
        .map_err(|e| anyhow!("deriving the SRP password: {e}"))?;
    let verifier = compute_verifier(
        &auth::normalize_email(&me.email),
        srp_password,
        new_srp_salt,
    )
    .map_err(|e| anyhow!("computing the SRP verifier: {e}"))?;

    let new_master = derive_master_key(next.as_bytes(), &new_argon2_salt)
        .map_err(|e| anyhow!("deriving the new master key: {e}"))?;

    let new_sealed = encrypt_private_key(&keypair, &new_master)
        .map_err(|e| anyhow!("re-sealing your keypair: {e}"))?;

    let mut wraps = Vec::with_capacity(opened.len());
    for v in &opened {
        let w = wrap_vault_key_with_master_key(&v.key, &new_master)
            .map_err(|e| anyhow!("re-wrapping the key for {}: {e}", v.label))?;
        wraps.push(VaultWrap {
            vault_id: v.id.clone(),
            encrypted_vault_key: b64_encode(&w),
        });
    }

    // ── Check our own work ──────────────────────────────────────────────────
    //
    // ⚠️ Not optional, and not a debug aid. The server cannot do this — it holds
    // ciphertext — so a wrong-but-well-formed wrap would reach the database and
    // be discovered by a user who can no longer open anything.
    print!("  Verifying the new wraps open… ");
    verify_rewrap(
        &new_master,
        &keypair,
        &new_sealed.to_base64(),
        &wraps,
        &opened,
    )?;
    println!("{}", "ok".green());

    // ── Write the way back, before sending anything ─────────────────────────
    let bundle_path = write_recovery_bundle(&server, &me, &opened)?;
    println!("  Recovery file  {}", bundle_path.display());

    // ── Prove the current password to the server ────────────────────────────
    reauthenticate(&client, &me, &current, &old_salt, verbose)?;

    // ── The swap ────────────────────────────────────────────────────────────
    let body = RotateRequest {
        srp_salt: verifier.srp_salt_base64(),
        srp_verifier: verifier.verifier_hex(),
        argon2_salt: salt_to_base64(&new_argon2_salt),
        encrypted_private_key: new_sealed.to_base64(),
        vault_wraps: wraps,
        reason: if compromised {
            "compromised"
        } else {
            "routine"
        },
    };

    let out: RotateResponse = client
        .post("/api/v1/auth/master-key", &body)
        .map_err(rotate_error)?;

    if out.status == "already_applied" {
        println!();
        println!(
            "  {} this change had already been applied — a previous attempt reached the \
             server after all.",
            "✓".green()
        );
        println!("  Sign in with the new password.");
        return Ok(());
    }

    // ── Confirm it round-tripped ────────────────────────────────────────────
    //
    // Everything above proved the wraps open locally. This proves the ones the
    // server now holds are the ones that were sent.
    print!("  Confirming against the server… ");
    let mut confirmed = 0usize;
    for v in &opened {
        let my_key: MyWrappedKey = client
            .get(&format!("/api/v1/vaults/{}/my-key", v.id))
            .map_err(|e| anyhow!("\n  could not re-read your key for {}: {e}", v.label))?;
        let raw = b64_decode(&my_key.encrypted_vault_key, "encrypted_vault_key")
            .map_err(|e| anyhow!("\n  unusable wrapped key for {}: {e}", v.label))?;
        let back = unwrap_vault_key_with_master_key(&raw, &new_master).map_err(|_| {
            anyhow!(
                "\n  {} the password WAS changed, but {} no longer opens with it.\n\
                 \x20 Undo with `evnx auth undo-password-change`, or restore from {}.",
                "!".red(),
                v.label,
                bundle_path.display()
            )
        })?;
        if back.expose() != v.key.expose() {
            return Err(anyhow!(
                "\n  {} the password WAS changed, but {} came back wrapping a different key.\n\
                 \x20 Undo with `evnx auth undo-password-change`.",
                "!".red(),
                v.label
            ));
        }
        confirmed += 1;
    }
    println!("{}", format!("{confirmed} ok").green());

    println!();
    println!("  {} master password changed", "✓".green());
    println!("  {} vault key(s) re-wrapped", out.vaults_rewrapped);
    if out.sessions_revoked > 0 {
        println!("  {} other session(s) signed out", out.sessions_revoked);
    }
    match out.undo_available_until {
        Some(deadline) => {
            println!(
                "  {}",
                format!("Undo available until {deadline} — `evnx auth undo-password-change`")
                    .dimmed()
            );
            println!(
                "  {}",
                format!("Delete {} once you are sure.", bundle_path.display()).dimmed()
            );
        }
        None => println!("  {}", "No undo was kept.".yellow()),
    }
    Ok(())
}

// ─── Undo ─────────────────────────────────────────────────────────────────────

/// Put back the password a rotation replaced.
///
/// ⚠️ **Deliberately does not require a session.** Whoever needs this is whoever
/// a rotation locked out, and a session is exactly what they do not have. The
/// only credential it asks for is the one an attacker lacks: the *old* password.
pub fn undo(
    server_override: Option<&str>,
    email: Option<String>,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{
        compute_client_proof, derive_srp_password, generate_client_ephemeral, salt_from_base64,
        verify_server_proof,
    };

    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;

    // The stored session survives a rotation performed elsewhere, so its address
    // is usually still the right one — but it is only a default.
    let remembered: Option<String> = super::creds::Store::load()
        .ok()
        .and_then(|s| s.session(&server).map(|x| x.email.clone()));

    let email = match (email, remembered) {
        (Some(e), _) => auth::normalize_email(&e),
        (None, Some(e)) => auth::normalize_email(&e),
        (None, None) => auth::normalize_email(&auth::prompt_email()?),
    };

    println!();
    println!("  {}", "Undoing a master password change.".bold());
    println!("  account   {email}");
    println!(
        "  {}",
        "Enter the password you used BEFORE the change.".dimmed()
    );
    println!();

    let old = auth::read_password(password_stdin, "Previous master password")?;

    let ephemeral =
        generate_client_ephemeral().map_err(|e| anyhow!("generating an SRP ephemeral: {e}"))?;

    let init: ChallengeResponse = client
        .post_public(
            "/api/v1/auth/master-key/undo/init",
            &UndoInit {
                email: email.clone(),
                client_public: hex::encode(&ephemeral.public_a),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!("  Deriving your previous key on this machine…");
    let srp_salt = salt_from_base64(&init.srp_salt)
        .map_err(|e| anyhow!("the server sent an unusable srp_salt: {e}"))?;
    let server_public = hex::decode(&init.server_public)
        .context("the server sent an unusable ephemeral public key")?;

    let srp_password = derive_srp_password(old.as_bytes(), &srp_salt)
        .map_err(|e| anyhow!("deriving the SRP password: {e}"))?;
    let proof = compute_client_proof(&email, srp_password, &srp_salt, &server_public, &ephemeral)
        .map_err(|e| anyhow!("computing the SRP proof: {e}"))?;

    let out: UndoResponse = client
        .post_public(
            "/api/v1/auth/master-key/undo/verify",
            &UndoVerify {
                session_id: init.session_id,
                client_proof: hex::encode(&proof.client_proof),
            },
        )
        .map_err(undo_error)?;

    // The server proves itself here exactly as it does at login. Skipping it
    // would let a hostile endpoint claim a restore that never happened.
    let server_proof =
        hex::decode(&out.server_proof).context("the server sent an unusable proof")?;
    verify_server_proof(&server_proof, &proof).map_err(|_| {
        anyhow!(
            "the server could not prove it holds your previous verifier, so this is not \
             the server you registered with.\n\
             \x20 Check --server, and treat the connection as untrusted."
        )
    })?;
    if verbose {
        println!("  server proof verified (mutual authentication)");
    }

    println!();
    println!("  {} previous master password restored", "✓".green());
    println!("  {} vault key(s) restored", out.vaults_restored);
    if out.sessions_revoked > 0 {
        println!("  {} session(s) signed out", out.sessions_revoked);
    }
    if !out.vaults_not_in_snapshot.is_empty() {
        println!();
        println!(
            "  {} {} vault(s) were created after the change and could not be restored:",
            "!".yellow(),
            out.vaults_not_in_snapshot.len()
        );
        for id in &out.vaults_not_in_snapshot {
            println!("      {}", id.dimmed());
        }
        println!(
            "  {}",
            "Their keys are wrapped under the password that has just been undone, so they \
             will not open. Delete them, or redo the change to reach them again."
                .dimmed()
        );
    }
    println!();
    println!("  Run `evnx auth login` with the restored password.");
    Ok(())
}

// ─── Shared ───────────────────────────────────────────────────────────────────

/// Prove the current password to the server, arming the rotation.
fn reauthenticate(
    client: &Client,
    me: &Me,
    password: &Zeroizing<String>,
    _argon2_salt: &[u8; 32],
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{
        compute_client_proof, derive_srp_password, generate_client_ephemeral, salt_from_base64,
        verify_server_proof,
    };

    let ephemeral =
        generate_client_ephemeral().map_err(|e| anyhow!("generating an SRP ephemeral: {e}"))?;

    let init: ChallengeResponse = client
        .post(
            "/api/v1/auth/reauth/init",
            &ChallengeInit {
                client_public: hex::encode(&ephemeral.public_a),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    // ⚠️ The SRP salt here is the account's, not the one just generated for the
    // new password — this proves the password being *replaced*.
    let srp_salt = salt_from_base64(&init.srp_salt)
        .map_err(|e| anyhow!("the server sent an unusable srp_salt: {e}"))?;
    let server_public = hex::decode(&init.server_public)
        .context("the server sent an unusable ephemeral public key")?;

    let email = auth::normalize_email(&me.email);
    let srp_password = derive_srp_password(password.as_bytes(), &srp_salt)
        .map_err(|e| anyhow!("deriving the SRP password: {e}"))?;
    let proof = compute_client_proof(&email, srp_password, &srp_salt, &server_public, &ephemeral)
        .map_err(|e| anyhow!("computing the SRP proof: {e}"))?;

    let totp_code = if me.totp_enabled {
        Some(auth::prompt_totp_code()?.trim().to_string())
    } else {
        None
    };

    let out: ProofResponse = client
        .post(
            "/api/v1/auth/reauth/verify",
            &ReauthVerify {
                session_id: init.session_id,
                client_proof: hex::encode(&proof.client_proof),
                totp_code,
            },
        )
        .map_err(reauth_error)?;

    let server_proof =
        hex::decode(&out.server_proof).context("the server sent an unusable proof")?;
    verify_server_proof(&server_proof, &proof).map_err(|_| {
        anyhow!(
            "the server could not prove it holds your verifier, so this is not the server \
             you registered with. Nothing has been changed."
        )
    })?;
    if verbose {
        println!("  re-authenticated (server proof verified)");
    }
    Ok(())
}

/// Open everything that was just produced, with the key it was produced for.
///
/// ⚠️ **The one check the server structurally cannot make.** It holds ciphertext,
/// so a wrong-but-well-formed wrap is indistinguishable to it from a correct one
/// and would be stored without complaint — to be discovered by someone who can no
/// longer open anything, with no way back.
///
/// Every failure here happens before a single byte is sent, so the account is
/// untouched and the message can say so.
fn verify_rewrap(
    new_master: &evnx_crypto::MasterKey,
    keypair: &evnx_crypto::UserKeypair,
    new_sealed_b64: &str,
    wraps: &[VaultWrap],
    opened: &[OpenedVault],
) -> Result<()> {
    use evnx_crypto::{
        b64_decode, decrypt_private_key, unwrap_vault_key_with_master_key, EncryptedPrivateKey,
    };

    let check_sealed = EncryptedPrivateKey::from_base64(new_sealed_b64)
        .map_err(|e| anyhow!("re-reading the sealed keypair: {e}"))?;
    let reopened = decrypt_private_key(&check_sealed, new_master).map_err(|_| {
        anyhow!("the re-sealed keypair did not open with the new key. Nothing has been sent.")
    })?;

    // The identity keypair must survive a rotation unchanged: it is re-sealed,
    // not replaced. If the public keys moved, every vault already shared *to*
    // this account would stop opening, and the server would have no way to know.
    if reopened.ed25519_public_base64() != keypair.ed25519_public_base64()
        || reopened.x25519_public_base64() != keypair.x25519_public_base64()
        || reopened.mlkem_public_base64() != keypair.mlkem_public_base64()
    {
        return Err(anyhow!(
            "the re-sealed keypair opened to different public keys. Nothing has been sent. \
             This is a bug in evnx — please report it."
        ));
    }

    if wraps.len() != opened.len() {
        return Err(anyhow!(
            "built {} wrap(s) for {} vault key(s). Nothing has been sent. This is a bug in \
             evnx — please report it.",
            wraps.len(),
            opened.len()
        ));
    }

    for (w, v) in wraps.iter().zip(opened.iter()) {
        if w.vault_id != v.id {
            return Err(anyhow!(
                "the wraps were built in a different order from the keys they came from. \
                 Nothing has been sent. This is a bug in evnx — please report it."
            ));
        }
        let raw = b64_decode(&w.encrypted_vault_key, "encrypted_vault_key")
            .map_err(|e| anyhow!("re-reading the wrap for {}: {e}", v.label))?;
        let back = unwrap_vault_key_with_master_key(&raw, new_master).map_err(|_| {
            anyhow!(
                "the new wrap for {} did not open with the new key. Nothing has been sent.",
                v.label
            )
        })?;
        if back.expose() != v.key.expose() {
            return Err(anyhow!(
                "the new wrap for {} opened to a different key. Nothing has been sent. \
                 This is a bug in evnx — please report it.",
                v.label
            ));
        }
    }
    Ok(())
}

/// Write the old wraps somewhere the server cannot reach.
fn write_recovery_bundle(
    server: &str,
    me: &Me,
    opened: &[OpenedVault],
) -> Result<std::path::PathBuf> {
    let dir = super::config::ensure_config_dir()?;
    let path = dir.join(format!(
        "rotation-{}.json",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));

    let bundle = RecoveryBundle {
        evnx_recovery: "master-key-rotation/1",
        written_at: chrono::Utc::now().to_rfc3339(),
        server: server.to_string(),
        email: me.email.clone(),
        note: "The values below are the wraps as they were BEFORE the password change. \
               They are ciphertext: nothing here opens without the OLD master password, \
               which evnx has never stored and never sends. Keep this file until a pull \
               has succeeded with the new password, then delete it.",
        prev_argon2_salt: me.argon2_salt.clone(),
        prev_encrypted_private_key: me.encrypted_private_key.clone(),
        prev_vault_wraps: opened
            .iter()
            .map(|v| BundledWrap {
                vault_id: v.id.clone(),
                label: v.label.clone(),
                encrypted_vault_key: v.old_wrap_b64.clone(),
            })
            .collect(),
    };

    let json = serde_json::to_vec_pretty(&bundle).context("serializing the recovery file")?;
    super::creds::write_atomic_secure(&path, &json)?;
    Ok(path)
}

/// How many API tokens the account has, for the warning. `None` if it cannot be
/// determined — a rotation must not fail because a courtesy count did not load.
fn live_token_count(client: &Client) -> Option<usize> {
    #[derive(Deserialize)]
    struct Tokens {
        tokens: Vec<serde::de::IgnoredAny>,
    }
    client
        .get::<Tokens>("/api/v1/auth/tokens")
        .ok()
        .map(|t| t.tokens.len())
}

/// A rejected `/reauth/verify` means the password or the code, not a dead session.
fn reauth_error(e: ApiError) -> anyhow::Error {
    match e {
        ApiError::Unauthorized => anyhow!(
            "that master password — or that two-factor code — was not accepted. \
             Nothing has been changed."
        ),
        ApiError::Locked { .. } => anyhow!(
            "too many failed attempts, so this account is locked for a while. \
             Nothing has been changed.\n\
             \x20 The lock lifts on its own; it is shared with sign-in, so wait rather \
             than retrying."
        ),
        other => anyhow!("{other}"),
    }
}

/// `/master-key` refuses in ways worth translating.
fn rotate_error(e: ApiError) -> anyhow::Error {
    match e {
        ApiError::Forbidden { .. } => anyhow!(
            "the server did not accept the proof of your current password, or it expired \
             before the change was sent. Nothing has been changed — run the command again."
        ),
        ApiError::Conflict { message } => anyhow!(
            "{message}\n\
             \x20 This means evnx and the server disagree about which vault keys you hold. \
             Nothing has been changed. Run `evnx vault list` and try again; if it persists, \
             please report it."
        ),
        other => anyhow!("{other}"),
    }
}

/// A rejected undo says as little as `/srp/init` does, and for the same reason.
fn undo_error(e: ApiError) -> anyhow::Error {
    match e {
        ApiError::Unauthorized => anyhow!(
            "that previous master password was not accepted, or there is nothing to undo.\n\
             \x20 evnx cannot tell you which — the server answers an address with no \
             pending change exactly as it answers a wrong password, so that this command \
             cannot be used to discover who has an account.\n\
             \x20 An undo is only available for a limited window after a change, and only \
             when the change kept one."
        ),
        ApiError::Locked { .. } => anyhow!(
            "too many failed attempts, so the undo is locked for a while. It lifts on its \
             own, and it is counted separately from sign-in."
        ),
        ApiError::Conflict { message } => anyhow!("{message}"),
        other => anyhow!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testutil::ConfigDirGuard;
    use evnx_crypto::{
        b64_encode, derive_master_key, encrypt_private_key, generate_keypair, generate_salt,
        wrap_vault_key_with_master_key, VaultKey,
    };
    use serial_test::serial;

    /// A master key that is genuinely derived, since the wrap functions take one.
    fn master(pw: &str) -> evnx_crypto::MasterKey {
        derive_master_key(pw.as_bytes(), &generate_salt()).unwrap()
    }

    /// Ids are opaque strings to this module, so a counter is enough and keeps
    /// the CLI free of a uuid dependency it does not otherwise need.
    fn next_id() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            "00000000-0000-0000-0000-{:012}",
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn opened(label: &str) -> OpenedVault {
        OpenedVault {
            id: next_id(),
            label: label.to_string(),
            old_wrap_b64: "b2xkLXdyYXA=".into(),
            key: VaultKey::generate(),
        }
    }

    fn wrap_for(v: &OpenedVault, mk: &evnx_crypto::MasterKey) -> VaultWrap {
        VaultWrap {
            vault_id: v.id.clone(),
            encrypted_vault_key: b64_encode(&wrap_vault_key_with_master_key(&v.key, mk).unwrap()),
        }
    }

    #[test]
    fn correct_output_verifies() {
        let mk = master("new one");
        let kp = generate_keypair();
        let sealed = encrypt_private_key(&kp, &mk).unwrap().to_base64();

        let vaults = vec![opened("app/production"), opened("app/staging")];
        let wraps: Vec<_> = vaults.iter().map(|v| wrap_for(v, &mk)).collect();

        verify_rewrap(&mk, &kp, &sealed, &wraps, &vaults).expect("honest output must verify");
    }

    /// ⚠️ The bug this whole check exists for: a wrap that is well-formed, decodes,
    /// and is simply under the wrong key. The server cannot tell — it holds
    /// ciphertext — so it would store it, and the vault would never open again.
    #[test]
    fn a_wrap_under_the_wrong_key_is_caught() {
        let mk = master("new one");
        let stale = master("the key we were supposed to stop using");
        let kp = generate_keypair();
        let sealed = encrypt_private_key(&kp, &mk).unwrap().to_base64();

        let vaults = vec![opened("app/production")];
        let wraps = vec![wrap_for(&vaults[0], &stale)];

        let err = verify_rewrap(&mk, &kp, &sealed, &wraps, &vaults)
            .expect_err("a wrap under the wrong key must not pass");
        let msg = err.to_string();
        assert!(msg.contains("app/production"), "must name the vault: {msg}");
        assert!(
            msg.contains("Nothing has been sent"),
            "must say the account is untouched: {msg}"
        );
    }

    /// Wrapping the right way with the wrong *key material* — the shape a
    /// mixed-up loop variable produces.
    #[test]
    fn a_wrap_of_the_wrong_vault_key_is_caught() {
        let mk = master("new one");
        let kp = generate_keypair();
        let sealed = encrypt_private_key(&kp, &mk).unwrap().to_base64();

        let vaults = vec![opened("app/production")];
        let someone_elses = opened("other/vault");
        let wraps = vec![VaultWrap {
            vault_id: vaults[0].id.clone(),
            encrypted_vault_key: b64_encode(
                &wrap_vault_key_with_master_key(&someone_elses.key, &mk).unwrap(),
            ),
        }];

        let err = verify_rewrap(&mk, &kp, &sealed, &wraps, &vaults)
            .expect_err("a wrap of a different key must not pass");
        assert!(err.to_string().contains("different key"), "{err}");
    }

    /// The wraps and the keys are zipped, so an order that drifted would wrap
    /// each vault with its neighbour's key and still verify pairwise.
    #[test]
    fn a_reordered_payload_is_caught() {
        let mk = master("new one");
        let kp = generate_keypair();
        let sealed = encrypt_private_key(&kp, &mk).unwrap().to_base64();

        let vaults = vec![opened("app/production"), opened("app/staging")];
        let mut wraps: Vec<_> = vaults.iter().map(|v| wrap_for(v, &mk)).collect();
        wraps.swap(0, 1);

        let err = verify_rewrap(&mk, &kp, &sealed, &wraps, &vaults)
            .expect_err("a reordered payload must not pass");
        assert!(err.to_string().contains("different order"), "{err}");
    }

    #[test]
    fn a_short_payload_is_caught() {
        let mk = master("new one");
        let kp = generate_keypair();
        let sealed = encrypt_private_key(&kp, &mk).unwrap().to_base64();

        let vaults = vec![opened("app/production"), opened("app/staging")];
        let wraps = vec![wrap_for(&vaults[0], &mk)];

        assert!(verify_rewrap(&mk, &kp, &sealed, &wraps, &vaults).is_err());
    }

    /// ⚠️ The recovery file is the only way back that does not depend on the
    /// server keeping anything, so it must be written before the request and
    /// readable only by its owner.
    #[test]
    #[serial]
    fn the_recovery_bundle_is_private_and_complete() {
        let _g = ConfigDirGuard::new();

        let me = Me {
            email: "someone@example.com".into(),
            argon2_salt: "c2FsdA==".into(),
            encrypted_private_key: "c2VhbGVk".into(),
            totp_enabled: false,
        };
        let vaults = vec![opened("app/production"), opened("app/staging")];

        let path = write_recovery_bundle("https://api.evnx.dev", &me, &vaults).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "got {mode:o}");
        }

        let text = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["prev_argon2_salt"], "c2FsdA==");
        assert_eq!(json["prev_encrypted_private_key"], "c2VhbGVk");
        assert_eq!(json["prev_vault_wraps"].as_array().unwrap().len(), 2);
        for w in json["prev_vault_wraps"].as_array().unwrap() {
            assert_eq!(w["encrypted_vault_key"], "b2xkLXdyYXA=");
        }

        // ⚠️ The bundle holds wraps, never anything unwrapped. An unwrapped vault
        // key on disk would undo the guarantee the whole product rests on.
        for v in &vaults {
            let raw = b64_encode(v.key.expose());
            assert!(
                !text.contains(&raw),
                "an unwrapped vault key reached the recovery file"
            );
        }
    }
}
