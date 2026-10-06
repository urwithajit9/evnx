//! `evnx cloud watch` — tell me when someone else changes this vault.
//!
//! ─── ⚠️ It does NOT pull, and that is the design, not a gap ─────────────────
//!
//! Two reasons, and the second is the one that settles it.
//!
//! **Overwriting `.env` would destroy local edits.** It is a file people change
//! constantly while developing, and a teammate's push is not consent to replace
//! what you have open. There is no undo for that.
//!
//! **Pulling needs the master password.** A watcher that auto-pulled would have
//! to hold the vault key in memory for its entire lifetime — hours, in a
//! terminal pane, on an unlocked laptop. That is a materially different threat
//! model from a one-shot `cloud pull` that derives a key, uses it and exits.
//! For a product whose claim is that the key never lingers anywhere it need not,
//! a long-lived resident key is not a detail.
//!
//! So this polls **metadata only**: no password, no key, no blob, no
//! decryption. The same data `cloud history` and `cloud diff` already read.
//! When something changes it says so and names the keys; taking it is a separate
//! deliberate `evnx cloud pull`.
//!
//! ─── What it is actually for ────────────────────────────────────────────────
//!
//! Finding out a teammate changed a secret **before** your next deploy picks it
//! up silently. That is the failure this prevents — not the inconvenience of
//! typing `pull`.

use anyhow::{anyhow, Result};
use colored::Colorize;
use std::time::Duration;

use super::client::Client;
use super::config::CloudConfig;
use super::sync::{key_delta, key_delta_line, list_versions, resolve_target, VersionSummary};
use super::vault;

/// ⚠️ A floor, not a suggestion. A one-second poll against a shared API is a
/// denial-of-service someone writes by accident, and the value of knowing two
/// seconds sooner is zero.
const MIN_INTERVAL_SECS: u64 = 5;
// ⓘ The default lives in the clap attribute on `CloudCommands::Watch`, so there
// is one place a reader checks and `--help` cannot disagree with the code.

fn newest(versions: &[VersionSummary]) -> Option<&VersionSummary> {
    versions.iter().max_by_key(|v| v.version_num)
}

pub fn run(
    server_override: Option<&str>,
    vault_target: Option<String>,
    interval_secs: u64,
    once: bool,
    verbose: bool,
) -> Result<()> {
    if interval_secs < MIN_INTERVAL_SECS {
        return Err(anyhow!(
            "--interval {interval_secs} is below the {MIN_INTERVAL_SECS}s minimum. \
             Polling faster does not make a teammate push sooner."
        ));
    }

    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let mut seen = list_versions(&client, &vault_ref)?;
    let mut last = newest(&seen).map(|v| v.version_num).unwrap_or(0);

    println!();
    println!("  {}  {}", "evnx cloud watch".bold(), vault_ref.label());
    if last == 0 {
        println!("  No versions yet. Watching for the first push.");
    } else {
        println!("  At v{last}. Watching for newer versions.");
    }
    println!(
        "  {}",
        format!("Polling every {interval_secs}s · metadata only, no password · Ctrl-C to stop")
            .dimmed()
    );
    println!();

    loop {
        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(interval_secs));

        // ⚠️ A failed poll is reported and the loop continues. A watcher that
        // exits on one network blip is a watcher nobody trusts to leave running,
        // and the next poll is seconds away.
        let fresh = match list_versions(&client, &vault_ref) {
            Ok(v) => v,
            Err(e) => {
                if verbose {
                    eprintln!("  {} poll failed: {e}", "!".yellow());
                }
                continue;
            }
        };

        let Some(newest_now) = newest(&fresh) else {
            continue;
        };
        if newest_now.version_num <= last {
            continue;
        }

        // Compare against the version we last reported, not against the
        // immediately previous one — two pushes between polls are one event to
        // the reader, and showing only the last hop would hide a key that
        // appeared and vanished.
        let previous = seen.iter().find(|v| v.version_num == last);
        let line = match previous {
            Some(p) => key_delta_line(&key_delta(p, newest_now)),
            None => format!("{} keys", newest_now.key_count),
        };

        let when: String = newest_now
            .pushed_at
            .replace('T', " ")
            .chars()
            .take(16)
            .collect();
        println!(
            "  {} v{} · {} · {}",
            "▲".green().bold(),
            newest_now.version_num,
            line,
            when.dimmed()
        );
        println!(
            "    {}",
            format!("evnx cloud pull --vault {}", vault_ref.label()).cyan()
        );

        if verbose {
            let d = previous.map(|p| key_delta(p, newest_now));
            if let Some(d) = d {
                if d.changed_contents && d.added.is_empty() && d.removed.is_empty() {
                    println!(
                        "    {}",
                        "the key names are identical, so a value changed — evnx cannot see which"
                            .dimmed()
                    );
                }
            }
        }

        last = newest_now.version_num;
        seen = fresh;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ The floor has to be enforced, not documented. A `--interval 1` in a
    /// CI loop is how a shared API gets hammered by someone being helpful.
    #[test]
    fn an_interval_below_the_floor_is_refused() {
        for bad in [0, 1, 4] {
            let e = run(None, Some("x".into()), bad, true, false).unwrap_err();
            assert!(
                e.to_string().contains("minimum"),
                "interval {bad} should be refused: {e}"
            );
        }
    }
}
