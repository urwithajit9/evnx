//! Two-factor authentication — `evnx auth totp …`.
//!
//! # Why this command exists at all
//!
//! evnx-server has supported TOTP since B15: enrolment, single-use recovery
//! codes, disable, and regeneration. Until this module there was **no way to
//! reach any of it** — no dashboard exists yet, and the CLI had only the login-
//! side challenge. A zero-knowledge secrets vault where the second factor cannot
//! be switched on is worse than one that never claimed to have it.
//!
//! # What a second factor protects, and what it does not
//!
//! It protects the **account**: the vault list, sharing, token minting, session
//! management. It does **not** protect the ciphertext. Vault contents are sealed
//! under a key derived from the master password, so someone who has that password
//! can decrypt anything they can fetch, and TOTP is what stops them fetching it.
//!
//! # Recovery codes are the whole safety net
//!
//! The server holds only ciphertext, so a lost authenticator with no recovery code
//! is an account nobody can restore — not the user, not an administrator. That is
//! why the codes are printed once, loudly, and why `disable` and
//! `recovery-codes` both demand a valid second factor rather than a live session:
//! a stolen session must not be able to strip 2FA off the account.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::client::Client;
use super::config::CloudConfig;

#[derive(Deserialize)]
struct SetupResponse {
    /// `otpauth://` URI for an authenticator app.
    totp_uri: String,
    /// The same secret, for typing in by hand.
    secret_base32: String,
}

#[derive(Serialize)]
struct CodeRequest {
    totp_code: String,
}

#[derive(Deserialize)]
struct CodesResponse {
    backup_codes: Vec<String>,
}

/// Turn on two-factor authentication.
pub fn enable(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let setup: SetupResponse = client
        .post("/api/v1/auth/totp/setup", &())
        .map_err(|e| anyhow!("{e}"))?;

    println!("{}", "Scan this with your authenticator app".bold());
    println!();
    println!("{}", render_qr(&setup.totp_uri)?);
    println!("  Cannot scan it? Enter the key by hand instead:");
    println!();
    println!("    {}", group(&setup.secret_base32).bold());
    println!();
    if verbose {
        println!("  uri  {}", setup.totp_uri);
        println!();
    }

    let code = dialoguer::Input::<String>::new()
        .with_prompt("Code from your authenticator")
        .interact_text()
        .context("reading the code")?;

    let confirmed: CodesResponse = client
        .post(
            "/api/v1/auth/totp/confirm",
            &CodeRequest {
                totp_code: code.trim().to_string(),
            },
        )
        .map_err(confirm_error)?;

    println!();
    println!("  {} two-factor authentication is on", "✓".green());
    print_codes(&confirmed.backup_codes, false);
    Ok(())
}

/// Turn two-factor authentication off.
pub fn disable(server_override: Option<&str>, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    println!("  Turning off 2FA needs a current code — a live session is not enough,");
    println!("  so that a stolen one cannot strip the second factor off your account.");
    let code = prompt_second_factor()?;

    client
        .post::<_, serde::de::IgnoredAny>(
            "/api/v1/auth/totp/disable",
            &CodeRequest { totp_code: code },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!("  {} two-factor authentication is off", "✓".green());
    println!("  Your recovery codes were deleted along with it.");
    Ok(())
}

/// Issue a fresh set of recovery codes, invalidating the old ones.
pub fn regenerate(server_override: Option<&str>, _verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    println!("  This replaces every existing recovery code. Any you have written");
    println!("  down will stop working.");
    let code = prompt_second_factor()?;

    let fresh: CodesResponse = client
        .post(
            "/api/v1/auth/totp/backup-codes",
            &CodeRequest { totp_code: code },
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!("  {} new recovery codes issued", "✓".green());
    print_codes(&fresh.backup_codes, true);
    Ok(())
}

/// Print recovery codes with the weight they deserve.
fn print_codes(codes: &[String], replaced: bool) {
    println!();
    println!("{}", "  Recovery codes — save these now".bold());
    println!();
    for pair in codes.chunks(2) {
        let line: Vec<String> = pair.iter().map(|c| format!("{c:<16}")).collect();
        println!("    {}", line.join(""));
    }
    println!();
    println!(
        "  {} shown once. They are stored hashed, so they cannot be recovered —",
        "Read this:".yellow().bold()
    );
    println!("  only replaced. Each one works once, in place of an authenticator code.");
    println!();
    println!("  The server holds only ciphertext. If you lose your authenticator and");
    println!(
        "  have no recovery code left, {} — not you, not us.",
        "nobody can restore the account".yellow()
    );
    if replaced {
        println!();
        println!("  Your previous codes no longer work.");
    }
}

/// Render an `otpauth://` URI as a terminal QR code.
///
/// Rendered **inverted** — light blocks for the dark modules. A scanner needs
/// dark-on-light, and a terminal is usually light-on-dark, so drawing the modules
/// with the foreground colour produces a code that will not scan on the majority
/// of terminals. The hand-typed key is always printed too, because terminal
/// rendering depends on the font having half-block glyphs.
fn render_qr(uri: &str) -> Result<String> {
    use qrcode::render::unicode;
    use qrcode::QrCode;

    let code = QrCode::new(uri.as_bytes()).context("building the enrolment QR code")?;
    let rendered = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();

    // Indent so it sits with the rest of the output.
    Ok(rendered
        .lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Break a base32 secret into groups of four, which is far easier to type.
fn group(secret: &str) -> String {
    secret
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn prompt_second_factor() -> Result<String> {
    let code = dialoguer::Input::<String>::new()
        .with_prompt("Authenticator code (or a recovery code)")
        .interact_text()
        .context("reading the second factor")?;
    Ok(code.trim().to_string())
}

/// Explain a failed confirmation in terms of what probably went wrong.
fn confirm_error(e: super::client::ApiError) -> anyhow::Error {
    use super::client::ApiError;
    match e {
        ApiError::Unauthorized | ApiError::Validation { .. } => anyhow!(
            "that code was not accepted, so 2FA was not switched on.\n\
             \x20 Codes last 30 seconds — try the next one. If they keep failing, your \
             device clock may have drifted: authenticator codes depend on it."
        ),
        other => anyhow!("{other}"),
    }
}

/// Account management needs a real login, never an API token.
fn require_login(client: &Client, server: &str) -> Result<()> {
    if client.is_api_token() {
        return Err(anyhow!(
            "an API token cannot manage two-factor authentication — that is deliberate.\n\
             \x20 A leaked CI token must not be able to enrol its own authenticator on \
             your account.\n\
             \x20 Unset EVNX_TOKEN and run `evnx auth login`."
        ));
    }
    if !client.is_signed_in() {
        return Err(anyhow!(
            "not signed in to {server}. Run `evnx auth login` first."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_grouped_for_typing() {
        assert_eq!(group("ABCDEFGHIJKLMNOP"), "ABCD EFGH IJKL MNOP");
        assert_eq!(group("ABCDE"), "ABCD E");
        assert_eq!(group(""), "");
    }

    #[test]
    fn the_qr_renders_and_carries_a_quiet_zone() {
        let out =
            render_qr("otpauth://totp/evnx:a@example.com?secret=JBSWY3DPEHPK3PXP&issuer=evnx")
                .unwrap();
        assert!(out.lines().count() > 10, "suspiciously small QR:\n{out}");
        // A scanner needs a light border. Without the quiet zone the code is
        // unreadable no matter how well the modules render.
        let first = out.lines().next().unwrap();
        assert!(
            first.trim_start().chars().all(|c| c == '█' || c == ' '),
            "the top row should be quiet zone, got: {first:?}"
        );
    }

    #[test]
    fn a_uri_too_long_to_encode_is_an_error_not_a_panic() {
        // QR has a hard capacity limit; exceeding it must not unwrap-panic in a
        // command the user is midway through.
        let huge = "x".repeat(10_000);
        assert!(render_qr(&huge).is_err());
    }

    #[test]
    fn a_rejected_code_explains_clock_drift() {
        use super::super::client::ApiError;
        let err = confirm_error(ApiError::Unauthorized).to_string();
        assert!(err.contains("30 seconds"), "{err}");
        assert!(err.contains("clock"), "{err}");
        assert!(err.contains("not switched on"), "{err}");
    }
}
