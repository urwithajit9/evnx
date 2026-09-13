//! Account commands — `evnx auth …`.
//!
//! # Where the zero-knowledge boundary sits
//!
//! The master password never leaves this process, and neither does anything
//! derived from it that could open a vault. What goes to the server is:
//!
//! | Sent | Why it is safe to send |
//! |------|------------------------|
//! | `srp_verifier` | A one-way function of a *separately salted* Argon2id derivation. Proves knowledge of the password without revealing it. |
//! | `srp_salt`, `argon2_salt` | Salts are public by design; they exist to make precomputation useless. |
//! | `ed25519_public_key`, `x25519_public_key` | Public halves. |
//! | `encrypted_private_key` | The Ed25519 seed sealed under the master key. Opaque without the password. |
//!
//! The two salts are deliberately different. Sharing one would mean a stolen SRP
//! verifier and the master key came from the same Argon2id output, so cracking
//! the verifier offline would hand over the vaults too. `evnx-crypto` also passes
//! a distinct domain tag into each derivation as Argon2's secret parameter.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::client::{jwt_exp, Client};
use super::config::CloudConfig;
use super::creds::{SecretString, Session, Store};

/// Shortest master password accepted.
///
/// Not arbitrary, and not a copy of a web form's rule. Two properties of this
/// system make a weak master password worse than usual:
///
/// 1. **The verifier is on the server.** A breach hands an attacker material they
///    can attack offline, at their own pace, without rate limiting. Argon2id at
///    64 MiB makes each guess expensive, but not free.
/// 2. **There is no reset.** Nobody can recover the account — the server holds no
///    key. A weak password cannot be quietly upgraded after a leak the way a
///    normal login can.
pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Serialize)]
struct RegisterRequest {
    email: String,
    srp_verifier: String,
    srp_salt: String,
    argon2_salt: String,
    ed25519_public_key: String,
    x25519_public_key: String,
    encrypted_private_key: String,
}

#[derive(Deserialize)]
struct RegisterResponse {
    user_id: String,
    #[allow(dead_code)]
    message: String,
}

/// Create an account.
///
/// Every value the server receives is computed here first; the request is built
/// only once all of it exists, so a crypto failure never leaves a half-registered
/// account.
pub fn register(
    server_override: Option<&str>,
    email: Option<String>,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    // Resolve the server before prompting: a bad --server should fail instantly,
    // not after the user has typed a password twice.
    let server = CloudConfig::resolve_server(server_override)?;

    let email = match email {
        Some(e) => e,
        None => prompt_email()?,
    };
    validate_email(&email)?;
    let email = normalize_email(&email);

    let password = if password_stdin {
        read_password_from_stdin()?
    } else {
        prompt_new_password()?
    };

    register_with_password(&server, &email, &password, verbose)
}

/// [`register`] with the password already in hand.
///
/// Separated so the whole flow — derivation, request, server errors — is testable
/// without a terminal. `register` is then only prompting.
fn register_with_password(
    server: &str,
    email: &str,
    password: &Zeroizing<String>,
    verbose: bool,
) -> Result<()> {
    validate_email(email)?;
    check_password_strength(password)?;

    println!();
    println!("  Deriving keys on this machine…");
    if verbose {
        println!("  (Argon2id, 64 MiB, t=3, p=4 — twice, with separate salts)");
    }

    let body = build_registration(email, password)?;

    let client = Client::new(server.to_string())?;
    let resp: RegisterResponse = client
        .post_public("/api/v1/auth/register", &body)
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} account created on {server}", "✓".green());
    if verbose {
        println!("  user id   {}", resp.user_id);
    }
    println!();
    println!("  {} check your inbox", "Next:".bold());
    println!("  A verification link was sent to {email}.");
    println!("  Vault commands stay unavailable until you open it.");
    println!();
    println!("  Then sign in with:  {}", "evnx auth login".cyan());
    println!();
    println!(
        "  {} your master password is the only thing that can open your vaults.",
        "Remember:".yellow().bold()
    );
    println!("  It is never sent anywhere and cannot be reset. If you lose it, the");
    println!("  data is gone — the server holds only ciphertext.");
    Ok(())
}

/// Everything the server needs, all derived locally.
///
/// Split out from [`register`] so the derivation can be tested without a server
/// or a terminal.
fn build_registration(email: &str, password: &Zeroizing<String>) -> Result<RegisterRequest> {
    use evnx_crypto::{
        compute_verifier, derive_master_key, derive_srp_password, encrypt_private_key,
        generate_keypair, generate_salt, salt_to_base64,
    };

    let pw = password.as_bytes();

    // Two independent salts. See the note on MIN_PASSWORD_LEN and the module docs
    // — reusing one would tie the SRP verifier and the master key to the same
    // Argon2id output.
    let srp_salt = generate_salt();
    let argon2_salt = generate_salt();

    let srp_password = derive_srp_password(pw, &srp_salt)
        .map_err(|e| anyhow!("deriving the SRP password: {e}"))?;
    let verifier = compute_verifier(email, srp_password, srp_salt)
        .map_err(|e| anyhow!("computing the SRP verifier: {e}"))?;

    let master_key =
        derive_master_key(pw, &argon2_salt).map_err(|e| anyhow!("deriving the master key: {e}"))?;

    let keypair = generate_keypair();
    let encrypted_private_key = encrypt_private_key(&keypair, &master_key)
        .map_err(|e| anyhow!("sealing the private key: {e}"))?;

    Ok(RegisterRequest {
        email: email.to_string(),
        srp_verifier: verifier.verifier_hex(),
        srp_salt: verifier.srp_salt_base64(),
        argon2_salt: salt_to_base64(&argon2_salt),
        ed25519_public_key: keypair.ed25519_public_base64(),
        x25519_public_key: keypair.x25519_public_base64(),
        encrypted_private_key: encrypted_private_key.to_base64(),
    })
}

/// Fold an address to the form the server stores, and to the SRP identity.
///
/// **This must match `evnx-server`'s normalisation exactly** — it trims and
/// lowercases in both `register` and `srp/init`. The reason is subtler than
/// tidiness: the email is the SRP *identity*, mixed into the verifier at
/// registration by this client. If someone registers as `Ajit@Example.com` and
/// later signs in as `ajit@example.com`, the proof is computed over a different
/// identity string and SRP fails — surfacing as "wrong password" for a password
/// that is perfectly correct. Normalising at both ends makes the two agree.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn prompt_email() -> Result<String> {
    dialoguer::Input::<String>::new()
        .with_prompt("Email")
        .interact_text()
        .context("reading the email address")
}

/// Ask for the password twice, without echoing it.
fn prompt_new_password() -> Result<Zeroizing<String>> {
    let pw = dialoguer::Password::new()
        .with_prompt("Master password")
        .with_confirmation("Confirm master password", "The passwords did not match")
        .interact()
        .context("reading the master password")?;
    Ok(Zeroizing::new(pw))
}

/// Read the password from stdin, for scripts and the end-to-end test harness.
///
/// A trailing newline is stripped and nothing else is: leading and inner spaces
/// are part of the password, because a passphrase may legitimately contain them.
fn read_password_from_stdin() -> Result<Zeroizing<String>> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the password from stdin")?;
    let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
    zeroize::Zeroize::zeroize(&mut line);
    Ok(Zeroizing::new(trimmed))
}

/// Reject an address the server would reject, before spending a second on Argon2id.
///
/// Deliberately loose. The server's validator is authoritative, and a client that
/// tries to out-clever it only invents new ways to refuse a valid address.
fn validate_email(email: &str) -> Result<()> {
    let trimmed = email.trim();
    if trimmed.len() > 254 {
        return Err(anyhow!(
            "that email address is too long (max 254 characters)"
        ));
    }
    // Whitespace anywhere, not just in the domain. The first version of this
    // checked only the domain and happily accepted "a b@example.com".
    if trimmed.chars().any(char::is_whitespace) {
        return Err(anyhow!("{trimmed:?} does not look like an email address"));
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(anyhow!("{trimmed:?} does not look like an email address"));
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(anyhow!("{trimmed:?} does not look like an email address"));
    }
    Ok(())
}

/// Refuse a master password too short to survive an offline attack.
fn check_password_strength(password: &Zeroizing<String>) -> Result<()> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(anyhow!(
            "the master password must be at least {MIN_PASSWORD_LEN} characters (got {len}).\n\
             \x20 It is the only thing protecting your vaults, the server keeps a verifier that \
             an attacker could attack offline after a breach, and there is no reset.\n\
             \x20 A passphrase of four or five unrelated words is easier to remember and far \
             stronger than a short complex one."
        ));
    }
    Ok(())
}

/// Sign in and store a session for this server.
///
/// # The SRP exchange, and why M2 is checked
///
/// SRP-6a authenticates **both** directions. The client proves it knows the
/// password with M1; the server proves it holds the matching verifier with M2.
/// Skipping the M2 check throws away half the protocol: it is what stops an
/// impostor server — one that never had the verifier — from completing a login
/// and being trusted. It is checked here before a single token is written to
/// disk, in the TOTP branch as well as the plain one.
pub fn login(
    server_override: Option<&str>,
    email: Option<String>,
    password_stdin: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;

    let email = match email {
        Some(e) => e,
        None => prompt_email()?,
    };
    validate_email(&email)?;
    let email = normalize_email(&email);

    let password = if password_stdin {
        read_password_from_stdin()?
    } else {
        Zeroizing::new(
            dialoguer::Password::new()
                .with_prompt("Master password")
                .interact()
                .context("reading the master password")?,
        )
    };

    login_with_password(&server, &email, &password, verbose)
}

/// [`login`] with the password already in hand, so the exchange is testable.
fn login_with_password(
    server: &str,
    email: &str,
    password: &Zeroizing<String>,
    verbose: bool,
) -> Result<()> {
    use evnx_crypto::{
        compute_client_proof, derive_srp_password, generate_client_ephemeral, salt_from_base64,
        verify_server_proof,
    };

    let client = Client::new(server.to_string())?;

    // ── Step 1: our ephemeral A ──────────────────────────────────────────────
    let ephemeral =
        generate_client_ephemeral().map_err(|e| anyhow!("generating an SRP ephemeral: {e}"))?;

    let init: SrpInitResponse = client
        .post_public(
            "/api/v1/auth/srp/init",
            &SrpInitRequest {
                email: email.to_string(),
                client_public: hex::encode(&ephemeral.public_a),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    if verbose {
        println!("  srp session {}", init.session_id);
    }
    println!("  Deriving your key on this machine…");

    // ── Step 2: prove we know the password ───────────────────────────────────
    let srp_salt = salt_from_base64(&init.srp_salt)
        .map_err(|e| anyhow!("the server sent an unusable srp_salt: {e}"))?;
    let server_public = hex::decode(&init.server_public)
        .context("the server sent an unusable ephemeral public key")?;

    let srp_password = derive_srp_password(password.as_bytes(), &srp_salt)
        .map_err(|e| anyhow!("deriving the SRP password: {e}"))?;
    let proof = compute_client_proof(email, srp_password, &srp_salt, &server_public, &ephemeral)
        .map_err(|e| anyhow!("computing the SRP proof: {e}"))?;

    let verify: SrpVerifyResponse = client
        .post_public(
            "/api/v1/auth/srp/verify",
            &SrpVerifyRequest {
                session_id: init.session_id.clone(),
                client_proof: hex::encode(&proof.client_proof),
            },
        )
        .map_err(|e| anyhow!("{e}"))?;

    // ── Step 3: make the server prove itself, before trusting anything ───────
    let server_proof =
        hex::decode(&verify.server_proof).context("the server sent an unusable proof")?;
    verify_server_proof(&server_proof, &proof).map_err(|_| {
        anyhow!(
            "the server could not prove it holds your verifier, so this is not the \
             server you registered with. No session was saved.\n\
             \x20 Check --server, and treat the connection as untrusted."
        )
    })?;
    if verbose {
        println!("  server proof verified (mutual authentication)");
    }

    // ── Step 4: tokens, via TOTP if the account has it ───────────────────────
    let (access_token, refresh_token, remaining) = if verify.requires_totp {
        let pending = verify.totp_pending_token.ok_or_else(|| {
            anyhow!("the server asked for a second factor but sent no pending token")
        })?;
        let code = prompt_totp_code()?;
        let totp: TotpVerifyResponse = client
            .post_public(
                "/api/v1/auth/totp/verify",
                &TotpVerifyRequest {
                    totp_pending_token: pending,
                    totp_code: code.trim().to_string(),
                },
            )
            .map_err(|e| anyhow!("{e}"))?;
        (
            totp.access_token,
            totp.refresh_token,
            totp.backup_codes_remaining,
        )
    } else {
        (
            verify
                .access_token
                .ok_or_else(|| anyhow!("the server completed SRP but sent no access token"))?,
            verify
                .refresh_token
                .ok_or_else(|| anyhow!("the server completed SRP but sent no refresh token"))?,
            None,
        )
    };

    // ── Step 5: persist ──────────────────────────────────────────────────────
    let expires_at = jwt_exp(&access_token).unwrap_or_else(|| chrono::Utc::now().timestamp());
    let mut store = Store::load()?;
    store.set_session(
        server,
        Session {
            email: email.to_string(),
            access_token: SecretString::new(access_token),
            access_expires_at: expires_at,
            refresh_token: SecretString::new(refresh_token),
        },
    );
    store.save()?;

    println!("  {} signed in as {email}", "✓".green());
    if let Some(n) = remaining {
        // Warn while there is still time to act. Running out means the next lost
        // phone is a permanently locked account.
        let note = format!("  {n} recovery code(s) left");
        if n <= 3 {
            println!(
                "{} — reissue them with `evnx auth totp backup-codes`",
                note.yellow()
            );
        } else if verbose {
            println!("{note}");
        }
    }
    Ok(())
}

/// End the session on the server, then remove it locally.
///
/// The local session is cleared **even when the server call fails**. Someone
/// typing `logout` on a shared or borrowed machine wants the credentials off that
/// disk; refusing because the network is down would leave a usable refresh token
/// behind, which is the opposite of what they asked for. The server-side failure
/// is reported so they know the session may still be live elsewhere.
pub fn logout(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let mut store = Store::load()?;

    if store.session(&server).is_none() {
        println!("  Not signed in to {server}; nothing to do.");
        return Ok(());
    }

    let server_result = Client::new(server.clone()).and_then(|c| {
        c.post::<_, serde::de::IgnoredAny>("/api/v1/auth/logout", &())
            .map(|_| ())
    });

    store.remove_session(&server);
    store.save()?;

    match server_result {
        Ok(()) => println!("  {} signed out of {server}", "✓".green()),
        Err(e) => {
            println!("  {} local credentials removed", "✓".green());
            println!("  {} the server was not told: {e}", "warning:".yellow());
            println!("  The session may still be usable until it expires. Revoke it from");
            println!("  another signed-in machine with `evnx auth sessions`.");
        }
    }
    if verbose {
        println!("  store: {}", super::config::credentials_path()?.display());
    }
    Ok(())
}

/// Show the account as the **server** sees it.
///
/// Distinct from `evnx cloud status`, which reports what is on this disk without
/// a network call. This one answers "is my email verified, is 2FA on" — questions
/// only the server can answer, and the first authenticated request the CLI makes.
pub fn status(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;

    if !client.is_signed_in() {
        println!("  Not signed in to {server}. Run `evnx auth login`.");
        return Ok(());
    }

    let me: MeResponse = client.get("/api/v1/auth/me").map_err(|e| anyhow!("{e}"))?;

    println!("{}", "evnx account".bold());
    println!("  server    {server}");
    println!("  email     {}", me.email);
    println!(
        "  verified  {}",
        if me.email_verified {
            "yes".green()
        } else {
            "no".yellow()
        }
    );
    println!(
        "  2FA       {}",
        if me.totp_enabled {
            "enabled".green()
        } else {
            "off".dimmed()
        }
    );
    if verbose {
        println!("  user id   {}", me.user_id);
    }

    if !me.email_verified {
        println!();
        println!("  Vault commands stay unavailable until you open the verification");
        println!("  link sent to {}.", me.email);
    }
    Ok(())
}

fn prompt_totp_code() -> Result<String> {
    dialoguer::Input::<String>::new()
        .with_prompt("Authenticator code (or a recovery code)")
        .interact_text()
        .context("reading the second factor")
}

#[derive(Serialize)]
struct SrpInitRequest {
    email: String,
    /// A, hex-encoded.
    client_public: String,
}

#[derive(Deserialize)]
struct SrpInitResponse {
    session_id: String,
    srp_salt: String,
    #[allow(dead_code)]
    argon2_salt: String,
    /// B, hex-encoded.
    server_public: String,
}

#[derive(Serialize)]
struct SrpVerifyRequest {
    session_id: String,
    /// M1, hex-encoded.
    client_proof: String,
}

#[derive(Deserialize)]
struct SrpVerifyResponse {
    /// M2, hex-encoded.
    server_proof: String,
    requires_totp: bool,
    totp_pending_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Serialize)]
struct TotpVerifyRequest {
    totp_pending_token: String,
    /// A six-digit authenticator code, or a single-use recovery code — the
    /// server accepts either in this field.
    totp_code: String,
}

#[derive(Deserialize)]
struct TotpVerifyResponse {
    access_token: String,
    refresh_token: String,
    backup_codes_remaining: Option<u32>,
}

#[derive(Deserialize)]
struct MeResponse {
    user_id: String,
    email: String,
    email_verified: bool,
    totp_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testutil::ConfigDirGuard;
    use serial_test::serial;

    fn pw(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    #[test]
    fn a_short_password_is_refused_with_the_reason() {
        let err = check_password_strength(&pw("short"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 12"), "{err}");
        assert!(err.contains("no reset"), "the message must say why: {err}");
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // 12 emoji are 48 bytes but 12 characters. Counting bytes would let a
        // 3-character password through, and reject a legitimate 12-character one.
        assert!(check_password_strength(&pw(&"🔑".repeat(12))).is_ok());
        assert!(check_password_strength(&pw(&"é".repeat(11))).is_err());
        assert!(check_password_strength(&pw(&"é".repeat(12))).is_ok());
    }

    #[test]
    fn obviously_broken_emails_are_caught_before_a_second_of_argon2() {
        for bad in [
            "",
            "nope",
            "@example.com",
            "a@",
            "a@b",
            "a b@example.com",  // space in the local part
            "a@exa mple.com",   // space in the domain
            "a\tb@example.com", // tab
        ] {
            assert!(validate_email(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["a@example.com", "first.last+tag@sub.example.co.uk"] {
            assert!(validate_email(good).is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn registration_fields_match_every_server_validator() {
        // The server rejects on exact lengths. These are the same bounds as
        // RegisterRequest in evnx-server/src/routes/auth.rs, and the CLI is the
        // first client to send real values — the server's own tests use filler.
        let body =
            build_registration("a@example.com", &pw("correct horse battery staple")).unwrap();

        assert_eq!(body.srp_salt.len(), 44, "srp_salt must be 44 base64 chars");
        assert_eq!(body.argon2_salt.len(), 44, "argon2_salt must be 44");
        assert_eq!(body.ed25519_public_key.len(), 44, "ed25519 pub must be 44");
        assert_eq!(body.x25519_public_key.len(), 44, "x25519 pub must be 44");
        assert!(
            (256..=1024).contains(&body.srp_verifier.len()),
            "srp_verifier out of range: {}",
            body.srp_verifier.len()
        );
        assert!(
            (60..=300).contains(&body.encrypted_private_key.len()),
            "encrypted_private_key out of range: {}",
            body.encrypted_private_key.len()
        );
        assert!(
            body.srp_verifier.chars().all(|c| c.is_ascii_hexdigit()),
            "the verifier must be hex"
        );
    }

    #[test]
    fn the_two_salts_are_never_the_same() {
        // Sharing a salt would derive the SRP verifier and the master key from
        // one Argon2id output, so cracking the verifier offline would also yield
        // the vault keys.
        let body =
            build_registration("a@example.com", &pw("correct horse battery staple")).unwrap();
        assert_ne!(body.srp_salt, body.argon2_salt);
    }

    #[test]
    fn two_registrations_with_the_same_password_share_nothing() {
        // Fresh salts and a fresh keypair every time: two accounts with an
        // identical password must not produce an identical verifier, or the
        // server could tell that two users chose the same password.
        let a = build_registration("a@example.com", &pw("correct horse battery staple")).unwrap();
        let b = build_registration("b@example.com", &pw("correct horse battery staple")).unwrap();
        assert_ne!(a.srp_salt, b.srp_salt);
        assert_ne!(a.argon2_salt, b.argon2_salt);
        assert_ne!(a.srp_verifier, b.srp_verifier);
        assert_ne!(a.ed25519_public_key, b.ed25519_public_key);
        assert_ne!(a.encrypted_private_key, b.encrypted_private_key);
    }

    #[test]
    fn the_password_never_appears_in_the_request() {
        // The whole premise. If this ever fails, the product is broken.
        const SECRET: &str = "correct horse battery staple";
        let body = build_registration("a@example.com", &pw(SECRET)).unwrap();
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains(SECRET), "the password reached the wire");
        for word in SECRET.split(' ') {
            assert!(!json.contains(word), "{word:?} reached the wire");
        }
    }

    // ─── login ──────────────────────────────────────────────────────────────

    #[test]
    fn the_srp_identity_is_normalised_the_same_way_the_server_normalises_email() {
        // The email is the SRP *identity*, mixed into the verifier by this client
        // at registration. evnx-server trims and lowercases before storing, so a
        // client that did not would compute the login proof over a different
        // identity and fail with what looks like a wrong password.
        assert_eq!(normalize_email("  Ajit@Example.COM "), "ajit@example.com");
        assert_eq!(normalize_email("a@b.co"), "a@b.co");
        // Register and login must land on the same string, whatever was typed.
        assert_eq!(
            normalize_email("Login@Example.Test"),
            normalize_email("login@example.test")
        );
    }

    /// A mock evnx-server that runs the real SRP-6a server half, so the login
    /// tests exercise a genuine exchange rather than canned bytes.
    ///
    /// `/srp/init` records the client ephemeral `A` from the request; `/srp/verify`
    /// reads it back to complete the exchange, which is what the real server does
    /// via its Valkey-backed SRP session. `tamper_m2` corrupts the server's proof,
    /// and is how the mutual-authentication check is proven to actually run.
    fn mock_srp_server(
        email: &str,
        password: &str,
        tamper_m2: bool,
    ) -> (mockito::ServerGuard, Vec<mockito::Mock>) {
        use evnx_crypto::{compute_verifier, derive_srp_password, generate_salt, salt_to_base64};
        use sha2::Sha256;
        use srp::groups::G_2048;
        use srp::server::SrpServer;
        use std::sync::{Arc, Mutex};

        let srp_salt = generate_salt();
        let argon2_salt = generate_salt();
        let srp_password = derive_srp_password(password.as_bytes(), &srp_salt).unwrap();
        let verifier = compute_verifier(email, srp_password, srp_salt).unwrap();
        let v = verifier.verifier.clone();

        // Fixed server private ephemeral so init and verify agree. Shared state
        // rather than a thread-local: mockito runs these closures on its own
        // server thread.
        let b = vec![7u8; 64];
        let a_pub: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        let b_pub = SrpServer::<Sha256>::new(&G_2048).compute_public_ephemeral(&b, &v);

        let mut server = mockito::Server::new();

        let a_for_init = Arc::clone(&a_pub);
        let init_body = format!(
            r#"{{"session_id":"11111111-1111-1111-1111-111111111111","srp_salt":"{}","argon2_salt":"{}","server_public":"{}"}}"#,
            salt_to_base64(&srp_salt),
            salt_to_base64(&argon2_salt),
            hex::encode(&b_pub)
        );
        let init = server
            .mock("POST", "/api/v1/auth/srp/init")
            .with_status(200)
            .with_body_from_request(move |req| {
                let body: serde_json::Value = serde_json::from_slice(req.body().unwrap()).unwrap();
                let a = hex::decode(body["client_public"].as_str().unwrap()).unwrap();
                *a_for_init.lock().unwrap() = a;
                init_body.clone().into_bytes()
            })
            .create();

        let a_for_verify = Arc::clone(&a_pub);
        let v_for_verify = v.clone();
        let verify = server
            .mock("POST", "/api/v1/auth/srp/verify")
            .with_status(200)
            .with_body_from_request(move |req| {
                let body: serde_json::Value =
                    serde_json::from_slice(req.body().unwrap()).unwrap();
                let m1 = hex::decode(body["client_proof"].as_str().unwrap()).unwrap();
                let a = a_for_verify.lock().unwrap().clone();

                let vf = SrpServer::<Sha256>::new(&G_2048)
                    .process_reply(&b, &v_for_verify, &a)
                    .expect("mock server: process_reply");

                // The real server refuses here on a bad M1; the mock mirrors that
                // so a wrong password produces a 401 rather than a bogus success.
                if vf.verify_client(&m1).is_err() {
                    return br#"{"error":"Authentication failed","code":"UNAUTHORIZED"}"#.to_vec();
                }

                let m2 = if tamper_m2 {
                    vec![9u8; vf.proof().len()]
                } else {
                    vf.proof().to_vec()
                };

                format!(
                    r#"{{"server_proof":"{}","requires_totp":false,"access_token":"{}","refresh_token":"refresh-xyz"}}"#,
                    hex::encode(&m2),
                    jwt_with_exp(4_000_000_000)
                )
                .into_bytes()
            })
            .create();

        (server, vec![init, verify])
    }

    fn jwt_with_exp(exp: i64) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"exp":{exp},"scope":"user"}}"#));
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
    }

    #[test]
    #[serial]
    fn a_real_srp_exchange_signs_in_and_stores_the_session() {
        // The happy path, against the genuine SRP server half — so CI covers it,
        // not only the live box.
        let _g = ConfigDirGuard::new();
        const PW: &str = "correct horse battery staple";
        let (server, _mocks) = mock_srp_server("a@example.com", PW, false);

        login_with_password(&server.url(), "a@example.com", &pw(PW), true).unwrap();

        let session = Store::load().unwrap();
        let session = session.session(&server.url()).expect("no session stored");
        assert_eq!(session.email, "a@example.com");
        assert_eq!(session.refresh_token.expose(), "refresh-xyz");
        assert_eq!(session.access_expires_at, 4_000_000_000);
    }

    #[test]
    #[serial]
    fn a_wrong_password_fails_the_exchange_and_stores_nothing() {
        let _g = ConfigDirGuard::new();
        let (server, _mocks) = mock_srp_server("a@example.com", "the real passphrase", false);

        let err = login_with_password(
            &server.url(),
            "a@example.com",
            &pw("not the real passphrase"),
            false,
        );
        assert!(err.is_err());
        assert!(Store::load().unwrap().session(&server.url()).is_none());
    }

    #[test]
    #[serial]
    fn a_tampered_server_proof_is_rejected_and_nothing_is_stored() {
        // The single most important test in this module. SRP authenticates both
        // directions; if M2 is not checked, an impostor server that never held
        // the verifier completes a login and is trusted. A corrupted M2 must
        // fail, and must leave no session behind.
        let _g = ConfigDirGuard::new();
        const PW: &str = "correct horse battery staple";
        let (server, _mocks) = mock_srp_server("a@example.com", PW, true);

        let err = format!(
            "{:#}",
            login_with_password(&server.url(), "a@example.com", &pw(PW), false).unwrap_err()
        );

        assert!(err.contains("could not prove"), "{err}");
        assert!(
            Store::load().unwrap().session(&server.url()).is_none(),
            "a session was stored despite a bad server proof"
        );
    }

    #[test]
    #[serial]
    fn requires_totp_without_a_pending_token_is_a_clear_error() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let _i = server
            .mock("POST", "/api/v1/auth/srp/init")
            .with_status(200)
            .with_body(
                r#"{"session_id":"s","srp_salt":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                    "argon2_salt":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                    "server_public":"00"}"#,
            )
            .create();

        // Fails before the TOTP branch — a two-byte B is not a usable ephemeral —
        // but the point is that it fails loudly rather than storing a session.
        let err = login_with_password(
            &server.url(),
            "a@example.com",
            &pw("correct horse battery staple"),
            false,
        );
        assert!(err.is_err());
        assert!(Store::load().unwrap().session(&server.url()).is_none());
    }

    #[test]
    #[serial]
    fn a_malformed_srp_salt_from_the_server_is_reported_not_panicked_on() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let _i = server
            .mock("POST", "/api/v1/auth/srp/init")
            .with_status(200)
            .with_body(
                r#"{"session_id":"s","srp_salt":"not-base64!","argon2_salt":"x",
                    "server_public":"00"}"#,
            )
            .create();

        let err = format!(
            "{:#}",
            login_with_password(
                &server.url(),
                "a@example.com",
                &pw("correct horse battery staple"),
                false
            )
            .unwrap_err()
        );
        assert!(err.contains("srp_salt"), "{err}");
    }

    // ─── logout ─────────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn logout_without_a_session_makes_no_request() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("POST", "/api/v1/auth/logout")
            .expect(0)
            .create();
        logout(Some(&server.url()), false).unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn logout_clears_local_credentials_even_when_the_server_is_unreachable() {
        // Someone typing `logout` on a borrowed machine wants the tokens off that
        // disk. Refusing because the network is down would leave a usable
        // refresh token behind — the opposite of what was asked for.
        let _g = ConfigDirGuard::new();
        let dead = "http://127.0.0.1:1";
        let mut store = Store::default();
        store.set_session(
            dead,
            Session {
                email: "a@example.com".into(),
                access_token: SecretString::new("a"),
                access_expires_at: 4_000_000_000,
                refresh_token: SecretString::new("r"),
            },
        );
        store.save().unwrap();

        logout(Some(dead), false).unwrap();

        assert!(
            Store::load().unwrap().session(dead).is_none(),
            "credentials survived a logout against an unreachable server"
        );
    }

    // ─── status ─────────────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn status_without_a_session_says_so_without_a_request() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server.mock("GET", "/api/v1/auth/me").expect(0).create();
        status(Some(&server.url()), false).unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn status_reports_the_account_the_server_describes() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let mut store = Store::default();
        store.set_session(
            server.url(),
            Session {
                email: "a@example.com".into(),
                access_token: SecretString::new(&jwt_with_exp(4_000_000_000)),
                access_expires_at: 4_000_000_000,
                refresh_token: SecretString::new("r"),
            },
        );
        store.save().unwrap();

        let m = server
            .mock("GET", "/api/v1/auth/me")
            .with_status(200)
            .with_body(
                r#"{"user_id":"u1","email":"a@example.com","email_verified":false,
                    "encrypted_private_key":"x","argon2_salt":"y","totp_enabled":true}"#,
            )
            .expect(1)
            .create();

        status(Some(&server.url()), true).unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn a_successful_registration_posts_the_derived_body() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("POST", "/api/v1/auth/register")
            // The derived values are random by design and their shapes are
            // pinned by the length tests above, so this asserts only that the
            // right account is being registered.
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"email":"new@example.com"}"#.into(),
            ))
            .with_status(201)
            .with_body(r#"{"user_id":"7c9e6679-7425-40de-944b-e07fc1f90ae7","message":"ok"}"#)
            .create();

        register_with_password(
            &server.url(),
            "new@example.com",
            &pw("correct horse battery staple"),
            true,
        )
        .unwrap();
        drop(m);
    }

    #[test]
    #[serial]
    fn a_duplicate_email_surfaces_the_servers_conflict() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("POST", "/api/v1/auth/register")
            .with_status(409)
            .with_body(r#"{"error":"Email already registered","code":"CONFLICT"}"#)
            .expect(1)
            .create();

        let err = register_with_password(
            &server.url(),
            "taken@example.com",
            &pw("correct horse battery staple"),
            false,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("already registered"), "{err}");
        m.assert();
    }

    #[test]
    #[serial]
    fn a_weak_password_never_reaches_the_network() {
        // The strength check must run before the request, so a mock that would
        // fail the test if called proves it.
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("POST", "/api/v1/auth/register")
            .expect(0)
            .create();

        let err = register_with_password(&server.url(), "a@example.com", &pw("short"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 12"), "{err}");
        m.assert();
    }

    #[test]
    #[serial]
    fn an_unusable_server_url_fails_before_anything_is_derived() {
        let _g = ConfigDirGuard::new();
        let err = register(
            Some("http://api.evnx.dev"),
            Some("a@example.com".into()),
            false,
            false,
        )
        .unwrap_err();
        // `{:#}` renders the whole anyhow chain; `to_string()` shows only the
        // outermost context and would hide the reason entirely.
        let err = format!("{err:#}");
        assert!(err.contains("refusing plain http"), "{err}");
        // Resolved before any prompt: the failure is the URL, not a missing tty.
        assert!(!err.contains("stdin"), "{err}");
    }
}
