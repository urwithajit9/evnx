//! Active sessions — `evnx auth sessions …`.
//!
//! # Why this command exists
//!
//! The login-alert email has always told users to "revoke all sessions from your
//! account settings". Until this module there were no account settings to go to —
//! the server grew the endpoints in B14 and nothing could reach them. That made
//! the advice in a security email unfollowable, which is worse than not giving it.
//!
//! # Revocation is immediate, not eventual
//!
//! Revoking kills the session's refresh tokens **and** writes the session id to a
//! Valkey blocklist, so the outstanding access token stops working at once rather
//! than lingering for up to its fifteen-minute lifetime. That distinction is the
//! difference between "I've locked them out" and "I've locked them out in a
//! quarter of an hour", which matters a great deal when the reason for running
//! the command is a login alert you did not recognise.

use crate::cloud::client::api_path;
use anyhow::{anyhow, Result};
use colored::Colorize;
use serde::Deserialize;

use super::client::Client;
use super::config::CloudConfig;

#[derive(Deserialize)]
struct SessionList {
    sessions: Vec<SessionSummary>,
}

#[derive(Deserialize, Clone, Debug)]
struct SessionSummary {
    session_id: String,
    created_at: String,
    last_used_at: String,
    expires_at: String,
    /// The session running this command.
    current: bool,
}

/// List the account's active sessions.
pub fn list(server_override: Option<&str>, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: SessionList = client
        .get("/api/v1/auth/sessions")
        .map_err(|e| anyhow!("{e}"))?;

    if listed.sessions.is_empty() {
        println!("  No active sessions on {server}.");
        return Ok(());
    }

    println!(
        "  {:<8}  {:<17}  {:<17}  {:<10}  {}",
        "SESSION".bold(),
        "SIGNED IN".bold(),
        "LAST USED".bold(),
        "EXPIRES".bold(),
        "".bold()
    );
    for s in &listed.sessions {
        // The marker goes last, not next to the id. `{:<8}` pads by byte length,
        // and a coloured string carries ANSI escapes that count toward it — so an
        // inline marker silently shifts every column to its right.
        let id: String = s.session_id.chars().take(8).collect();
        println!(
            "  {:<8}  {:<17}  {:<17}  {:<10}  {}",
            id,
            super::when::stamp(&s.created_at),
            super::when::stamp(&s.last_used_at),
            super::when::date(&s.expires_at),
            if s.current {
                "← this device".green().to_string()
            } else {
                String::new()
            },
        );
    }

    if verbose {
        println!();
        for s in &listed.sessions {
            println!("  {}", s.session_id);
        }
    }

    crate::say!();
    crate::say!(
        "  Do not recognise one?  {}",
        "evnx auth sessions revoke <id>".cyan()
    );
    crate::say!(
        "  Sign out everywhere else:  {}",
        "evnx auth sessions revoke-others".cyan()
    );
    Ok(())
}

/// Revoke one session.
pub fn revoke(
    server_override: Option<&str>,
    target: String,
    assume_yes: bool,
    verbose: bool,
) -> Result<()> {
    // First, before signing in or listing anything (I-204, DEC-AU1).
    let target = check_target(&target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: SessionList = client
        .get("/api/v1/auth/sessions")
        .map_err(|e| anyhow!("{e}"))?;
    let session = resolve(&listed.sessions, &target)?;

    if session.current && !assume_yes {
        println!(
            "  {} that is the session you are using right now.",
            "Careful:".yellow().bold()
        );
        println!("  Revoking it signs this machine out immediately.");
    }

    if !assume_yes {
        let ok = crate::utils::prompt::confirm(
            &format!(
                "Revoke session {}?",
                session.session_id.chars().take(8).collect::<String>()
            ),
            false,
            &format!("evnx auth sessions revoke {target} --yes"),
        )?;
        if !ok {
            println!("  Cancelled; nothing was revoked.");
            return Ok(());
        }
    }

    client
        .delete(&api_path(&[
            "auth",
            "sessions",
            &session.session_id.to_string(),
        ])?)
        .map_err(|e| anyhow!("{e}"))?;

    crate::say!(
        "  {} revoked session {}",
        "✓".green(),
        session.session_id.chars().take(8).collect::<String>()
    );
    crate::say!("  Its access token stopped working immediately, not when it expires.");
    if session.current {
        // ⚠️ Remove the stored login too. This printed "signed out" while
        // credentials.json still held the dead session, so the next command
        // failed with an expired-session error instead (I-209, DEC-AU6).
        let mut store = super::creds::Store::load()?;
        store.remove_session(&server);
        store.save()?;
        crate::say!("  You are now signed out on this machine — run `evnx auth login`.");
    }
    if verbose {
        println!("  session id  {}", session.session_id);
    }
    Ok(())
}

/// Revoke every session except this one.
pub fn revoke_others(
    server_override: Option<&str>,
    assume_yes: bool,
    _verbose: bool,
) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    require_login(&client, &server)?;

    let listed: SessionList = client
        .get("/api/v1/auth/sessions")
        .map_err(|e| anyhow!("{e}"))?;
    let others = listed.sessions.iter().filter(|s| !s.current).count();

    if others == 0 {
        crate::say!("  This is your only active session; nothing else to revoke.");
        return Ok(());
    }

    if !assume_yes {
        println!("  About to sign out {others} other session(s). This one stays.");
        let ok = crate::utils::prompt::confirm(
            "Continue?",
            false,
            "evnx auth sessions revoke-others --yes",
        )?;
        if !ok {
            println!("  Cancelled; nothing was revoked.");
            return Ok(());
        }
    }

    client
        .delete("/api/v1/auth/sessions/others")
        .map_err(|e| anyhow!("{e}"))?;

    crate::say!("  {} revoked {others} other session(s)", "✓".green());
    crate::say!("  Their access tokens stopped working immediately.");
    Ok(())
}

/// The shortest session-id prefix `revoke` accepts.
pub(crate) const MIN_TARGET: usize = 4;

/// A revoke target, trimmed, or a refusal naming the rule.
///
/// ⚠️ An empty target matched **every** session by prefix (`starts_with("")`).
/// With one session — the usual single-machine case — `evnx auth sessions revoke
/// "$ID" -y` with `$ID` unset revoked the current session and signed the machine
/// out; with two or more it failed only by luck (I-204). Four characters is far
/// below the 8 the listing shows, so the shown id always works.
fn check_target(target: &str) -> Result<String> {
    let t = target.trim();
    if t.chars().count() < MIN_TARGET {
        return Err(anyhow!(
            "{target:?} is too short to name a session: give at least {MIN_TARGET} characters \
             of its id. `evnx auth sessions list` shows each one."
        ));
    }
    Ok(t.to_string())
}

/// Find the session a person meant, by full id or by the short form shown in the
/// listing.
///
/// A short prefix matching more than one session is refused rather than guessed —
/// revoking the wrong one could sign the user out of the machine they are on.
fn resolve<'a>(sessions: &'a [SessionSummary], target: &str) -> Result<&'a SessionSummary> {
    if let Some(s) = sessions.iter().find(|s| s.session_id == target) {
        return Ok(s);
    }
    let matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|s| s.session_id.starts_with(target))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(anyhow!(
            "no session starting {target:?}. Run `evnx auth sessions list` to see them."
        )),
        many => Err(anyhow!(
            "{target:?} matches {} sessions. Use more characters, or the full id.",
            many.len()
        )),
    }
}

fn require_login(client: &Client, _server: &str) -> Result<()> {
    client.require_user_session("manage sessions", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, current: bool) -> SessionSummary {
        SessionSummary {
            session_id: id.into(),
            created_at: "2026-09-14T15:02:11Z".into(),
            last_used_at: "2026-09-14T15:40:00Z".into(),
            expires_at: "2026-10-14T15:02:11Z".into(),
            current,
        }
    }

    #[test]
    fn a_session_resolves_by_full_id_or_short_prefix() {
        let sessions = [
            s("aaaa1111-2222-3333-4444-555555555555", true),
            s("bbbb9999", false),
        ];
        assert!(
            resolve(&sessions, "aaaa1111-2222-3333-4444-555555555555")
                .unwrap()
                .current
        );
        assert_eq!(resolve(&sessions, "bbbb").unwrap().session_id, "bbbb9999");
    }

    #[test]
    fn an_ambiguous_prefix_refuses_rather_than_guessing() {
        // Guessing could sign the user out of the machine they are sitting at.
        let sessions = [s("abcd1111", false), s("abcd2222", false)];
        let err = resolve(&sessions, "abcd").unwrap_err().to_string();
        assert!(err.contains("matches 2 sessions"), "{err}");
    }

    #[test]
    fn an_unknown_prefix_points_at_the_listing() {
        let err = resolve(&[s("abcd1111", false)], "zzzz")
            .unwrap_err()
            .to_string();
        // ⚠️ `sessions list`, not `sessions`: the bare group prints help, and
        // this test used to assert the wrong hint (I-208).
        assert!(err.contains("`evnx auth sessions list`"), "{err}");
    }

    /// I-204, DEC-AU1: under 4 characters is refused; the 8 shown always works.
    #[test]
    fn t_i204_a_target_needs_four_characters() {
        for bad in ["", "   ", "a", "abc", " ab "] {
            assert!(check_target(bad).is_err(), "{bad:?}");
        }
        assert_eq!(check_target(" abcd ").unwrap(), "abcd");
        assert_eq!(check_target("abcd1111").unwrap(), "abcd1111");
    }

    #[test]
    fn timestamps_are_trimmed_for_display() {
        assert_eq!(
            crate::cloud::when::stamp("2026-09-14T15:02:11Z"),
            "2026-09-14 15:02"
        );
        assert_eq!(
            crate::cloud::when::date("2026-10-14T15:02:11Z"),
            "2026-10-14"
        );
    }
}
