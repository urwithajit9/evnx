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

use super::client::Client;
use super::config::CloudConfig;

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
