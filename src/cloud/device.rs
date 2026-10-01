//! Known devices — `evnx auth devices …`.
//!
//! # A device is not a session, and the wording has to keep them apart
//!
//! [`super::session`] lists **live credentials** you can revoke. This lists
//! **places you have signed in from**, including ones with no session left at
//! all. Revoking a session ends one route in; disavowing a device records a
//! judgement and signs you out everywhere, including here.
//!
//! People reach for whichever they find first after an alarming email, so each
//! command points at the other rather than assuming the right one was chosen.
//!
//! # ⚠️ There is no location in any of this, and there cannot be
//!
//! The server identifies an origin by a **keyed BLAKE3 digest** of the client
//! address and the user agent. It holds neither value and can recover neither,
//! so there is no city, no country, and no "impossible travel" — those need a
//! raw IP, and evnx deliberately never stores one.
//!
//! That is a real cost, honestly paid: a different network simply means a phone
//! that reconnected, and the output must never imply more. Nothing printed here
//! names a place.
//!
//! # Why "this wasn't me" exists at all
//!
//! It is the only thing in the system that produces a **label**. Everything
//! else the server knows about a login is inference; a disavowal is the account
//! holder saying so. The server weighs it above every other signal combined.

use anyhow::{anyhow, Result};
use colored::Colorize;
use serde::Deserialize;

use super::client::Client;
use super::config::CloudConfig;

#[derive(Deserialize)]
struct DeviceList {
    devices: Vec<DeviceSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct DeviceSummary {
    id: String,
    first_seen: String,
    last_seen: String,
    sign_in_count: i64,
    is_current: bool,
    disavowed: bool,
    /// Sign-ins where neither digest was available.
    unknown_origin: bool,
}

#[derive(Deserialize)]
struct DisavowResponse {
    sessions_revoked: usize,
    change_master_password: bool,
}

/// `evnx auth devices list`
pub fn list(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: DeviceList = client
        .get("/api/v1/auth/devices")
        .map_err(|e| anyhow!("{e}"))?;

    if listed.devices.is_empty() {
        println!("  No sign-ins recorded on {server} yet.");
        return Ok(());
    }

    println!();
    println!(
        "  {:<14}  {:<17}  {:<17}  {:>7}  {}",
        "DEVICE".bold(),
        "FIRST SEEN".bold(),
        "LAST SEEN".bold(),
        "SIGN-INS".bold(),
        "".bold()
    );
    for d in &listed.devices {
        // ⚠️ The marker goes last. `{:<14}` pads by byte length and a coloured
        // string carries ANSI escapes that count toward it, so an inline marker
        // silently shifts every column to its right — the same trap
        // `session::list` documents.
        let marker = if d.disavowed {
            "← you said this was not you".red().to_string()
        } else if d.is_current {
            "← this device".green().to_string()
        } else {
            String::new()
        };
        let id = if d.unknown_origin {
            "(unknown)".dimmed().to_string()
        } else {
            d.id.clone()
        };
        println!(
            "  {:<14}  {:<17}  {:<17}  {:>7}  {}",
            id,
            stamp(&d.first_seen),
            stamp(&d.last_seen),
            d.sign_in_count,
            marker,
        );
    }

    if listed.devices.iter().any(|d| d.unknown_origin) {
        println!();
        println!(
            "  {}",
            "(unknown) groups sign-ins where the server could record no origin —".dimmed()
        );
        println!(
            "  {}",
            "a client behind no proxy, sending no user agent. Not a device.".dimmed()
        );
    }

    println!();
    println!(
        "  {}",
        "A device is a network and browser, not a place. evnx stores a hash of".dimmed()
    );
    println!(
        "  {}",
        "each and cannot tell where you were — a new row can just be a phone".dimmed()
    );
    println!("  {}", "that reconnected.".dimmed());
    println!();
    println!(
        "  Do not recognise one?  {}",
        "evnx auth devices disavow <id>".cyan()
    );
    println!(
        "  Live sessions instead:  {}",
        "evnx auth sessions list".cyan()
    );

    if verbose {
        println!();
        println!("  server: {server}");
    }
    Ok(())
}

/// `evnx auth devices disavow <id>`
pub fn disavow(
    server_override: Option<&str>,
    device_id: String,
    yes: bool,
    verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    // ⚠️ Confirmed by default, because this signs the person running it out
    // too. A command that silently ends your own session while you are trying
    // to investigate something is a command people run once.
    if !yes {
        println!();
        println!(
            "  {} every session on this account, including this one.",
            "This will end".bold()
        );
        println!("  You will need to sign in again.");
        println!();
        print!("  Disavow device {device_id}? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("  Nothing changed.");
            return Ok(());
        }
    }

    let resp: DisavowResponse = client
        .post(
            "/api/v1/auth/devices/disavow",
            &serde_json::json!({ "device_id": device_id }),
        )
        .map_err(|e| anyhow!("{e}"))?;

    println!();
    println!(
        "  {} recorded, and {} session(s) revoked.",
        "✓".green(),
        resp.sessions_revoked
    );

    // ⚠️ This is the part that must not be softened. Revoking a session does
    // not protect a vault: the vault keys are wrapped under the master key, so
    // anyone who learned the master password can sign in again a second later,
    // and anything they already pulled is already plaintext in their hands.
    //
    // The server returns `change_master_password` as a fact rather than advice
    // for exactly this reason — a client that printed "done ✓" and stopped
    // would be telling someone they were safe when they are not.
    if resp.change_master_password {
        println!();
        println!(
            "  {}",
            "⚠ Signing out is not enough. Change your master password now:"
                .yellow()
                .bold()
        );
        println!("     {}", "evnx auth rotate-master-password".cyan());
        println!();
        println!(
            "  {}",
            "Your vault keys are wrapped under that password. Whoever had access".dimmed()
        );
        println!(
            "  {}",
            "can sign in again until it changes — and anything they already".dimmed()
        );
        println!(
            "  {}",
            "pulled is plaintext in their hands already.".dimmed()
        );
    }

    if verbose {
        println!();
        println!("  server: {server}");
    }
    Ok(())
}

fn require_login(client: &Client, server: &str) -> Result<()> {
    if client.is_api_token() {
        return Err(anyhow!(
            "an API token cannot manage devices — that is deliberate.\n\
             \x20 A leaked CI token must not be able to enumerate where you sign in\n\
             \x20 from, nor to sign you out everywhere.\n\
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

/// `2026-10-02T14:23:08.123Z` → `2026-10-02 14:23`.
///
/// Shared shape with `session::stamp`; kept local rather than imported because
/// the two modules format different payloads and a change to one should not
/// silently move the other.
fn stamp(ts: &str) -> String {
    let s = ts.replace('T', " ");
    s.chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_trimmed_to_the_minute() {
        assert_eq!(stamp("2026-10-02T14:23:08.123456Z"), "2026-10-02 14:23");
    }

    #[test]
    fn a_short_timestamp_does_not_panic() {
        assert_eq!(stamp("2026-10-02"), "2026-10-02");
        assert_eq!(stamp(""), "");
    }

    /// ⚠️ Nothing this command prints may claim a location.
    ///
    /// The server identifies an origin by a keyed hash, which cannot be
    /// geolocated. This test exists so a later copy edit cannot quietly
    /// introduce a claim the architecture does not support.
    #[test]
    fn no_output_string_implies_a_place() {
        let source = include_str!("device.rs");
        // ⚠️ Stop at the test module. Scanning the whole file makes this test
        // match its own forbidden-word list, which is a self-reference and not
        // a finding — it failed exactly that way when first written.
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .unwrap_or(source);

        // Only the string literals a user sees, not the module docs that
        // explain why locations are absent.
        for line in production.lines() {
            let l = line.trim();
            if l.starts_with("//") || l.starts_with("///") || l.starts_with("//!") {
                continue;
            }
            let lower = l.to_lowercase();
            for word in ["\"location", "\"country", "\"city", "from location"] {
                assert!(
                    !lower.contains(word),
                    "printed copy implies a place evnx cannot know: {l}"
                );
            }
        }
    }
}
