//! `evnx cloud audit` — who did what to a vault.
//!
//! ─── Why this exists in the CLI at all ──────────────────────────────────────
//!
//! The trail is already served and already rendered in the dashboard. But "who
//! pulled this secret, and when" is an incident question, asked from a terminal,
//! often by someone holding a CI token and no browser session. Sending them to a
//! browser at that moment is the wrong answer.
//!
//! ─── Three things the server is emphatic about ──────────────────────────────
//!
//! 1. **Retention is a VISIBILITY limit, not deletion.** `audit_events` is
//!    append-only by database trigger since migration 006 — nothing prunes it
//!    and nothing can. The plan decides how far back the API will *show*.
//!    ⚠️ Describing it as deletion would be a data-handling claim that is not
//!    true, so this file never says "deleted" or "expired".
//!
//! 2. **`hidden_by_retention` must be printed.** The server returns it precisely
//!    because a trail that silently stops is indistinguishable from a vault with
//!    no older activity — and that difference is what an incident needs.
//!
//! 3. **`ip_hash` and `user_agent_hash` are deliberately absent** from the
//!    response. They are stable digests that correlate a person's activity
//!    across events without naming them: worth holding in the database, not
//!    worth handing to every colleague who shares a vault. There is nothing here
//!    to print and nothing to add.
//!
//! Read-only: one request, no blob, no key, no password.

use anyhow::{anyhow, Result};
use colored::Colorize;
use serde::Deserialize;

use super::client::Client;
use super::config::CloudConfig;
use super::sync::resolve_target;
use super::vault;

#[derive(Deserialize)]
struct AuditResponse {
    events: Vec<AuditEvent>,
    /// How many the server would return at most, before retention.
    #[allow(dead_code)]
    limit: i64,
    /// `None` means unlimited visibility on this plan.
    retention_days: Option<i32>,
    /// Events the plan's window withheld. ⚠️ Printed, never swallowed.
    hidden_by_retention: usize,
}

#[derive(Deserialize)]
struct AuditEvent {
    event_type: String,
    /// `None` when that account has since been deleted.
    actor_email: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: String,
}

/// Parse `7d`, `30d` or `all`.
///
/// ⚠️ A bad value is an error naming the accepted ones, not a silent fallback to
/// `all`. Silently widening a window someone narrowed on purpose is the wrong
/// direction to be wrong in.
fn parse_window(raw: &str) -> Result<Option<i64>> {
    let t = raw.trim().to_ascii_lowercase();
    if t == "all" {
        return Ok(None);
    }
    if let Some(days) = t.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        if days > 0 {
            return Ok(Some(days));
        }
    }
    Err(anyhow!(
        "`--last {raw}` is not a window. Use a number of days like `7d` or `30d`, or `all`."
    ))
}

/// One line of human detail for an event, from its metadata.
///
/// ⚠️ Metadata is server-written JSON and the shape varies by event type, so
/// every field is read optionally. An event whose metadata is missing or shaped
/// unexpectedly still prints its type and time — a trail that refuses to render
/// an event it does not recognise is worse than one that renders it plainly.
fn detail(e: &AuditEvent) -> String {
    let m = match &e.metadata {
        Some(serde_json::Value::Object(m)) => m,
        _ => return String::new(),
    };
    let num = |k: &str| m.get(k).and_then(|v| v.as_i64());
    let text = |k: &str| m.get(k).and_then(|v| v.as_str());

    match e.event_type.as_str() {
        "push" => num("version")
            .map(|v| format!("version {v}"))
            .unwrap_or_default(),
        "pull" => num("version")
            .map(|v| format!("version {v}"))
            .unwrap_or_default(),
        "version_deleted" => num("version")
            .map(|v| format!("version {v}"))
            .unwrap_or_default(),
        "member_grant" | "member_revoke" | "member_role_change" => {
            let who = text("target_email").or_else(|| text("email")).unwrap_or("");
            let role = text("role").unwrap_or("");
            match (who.is_empty(), role.is_empty()) {
                (false, false) => format!("{who} as {role}"),
                (false, true) => who.to_string(),
                (true, false) => role.to_string(),
                _ => String::new(),
            }
        }
        "vault_rekey" => num("members")
            .map(|n| format!("re-wrapped for {n} member{}", if n == 1 { "" } else { "s" }))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn run(
    server_override: Option<&str>,
    vault_target: Option<String>,
    last: &str,
    event_filter: Option<String>,
    verbose: bool,
) -> Result<()> {
    let window_days = parse_window(last)?;

    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    vault::require_session(&client, &server)?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let res: AuditResponse = client
        .get(&format!("/api/v1/vaults/{}/audit", vault_ref.id))
        .map_err(|e| anyhow!("{e}"))?;

    let cutoff = window_days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    let rows: Vec<&AuditEvent> = res
        .events
        .iter()
        .filter(|e| match (&event_filter, cutoff) {
            (Some(f), _) if !e.event_type.eq_ignore_ascii_case(f) => false,
            (_, Some(c)) => chrono::DateTime::parse_from_rfc3339(&e.created_at)
                .map(|t| t.with_timezone(&chrono::Utc) >= c)
                // ⚠️ An unparseable timestamp is SHOWN, not dropped. Hiding an
                // event because its date could not be read is the one failure a
                // trail must never have.
                .unwrap_or(true),
            _ => true,
        })
        .collect();

    println!();
    println!("  {}  {}", "evnx cloud audit".bold(), vault_ref.label());
    println!();

    if rows.is_empty() {
        if res.events.is_empty() {
            println!("  No activity recorded for this vault.");
        } else {
            println!(
                "  Nothing matches. The vault has {} recorded event{} outside this filter.",
                res.events.len(),
                if res.events.len() == 1 { "" } else { "s" }
            );
        }
    } else {
        println!(
            "  {:<19}  {:<18}  {:<26}  {}",
            "WHEN".bold(),
            "WHAT".bold(),
            "WHO".bold(),
            "DETAIL".bold()
        );
        for e in &rows {
            let when: String = e.created_at.replace('T', " ").chars().take(19).collect();
            // ⚠️ A deleted account is named as such rather than left blank: an
            // empty actor column reads as a bug, and "the account is gone" is a
            // fact worth knowing while reading a trail.
            let who = e.actor_email.as_deref().unwrap_or("(deleted account)");
            println!(
                "  {:<19}  {:<18}  {:<26}  {}",
                when,
                e.event_type,
                who,
                detail(e)
            );
        }
        println!();
        println!(
            "  {} event{} shown",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        );
    }

    // ─── Retention, stated plainly ──────────────────────────────────────────
    //
    // ⚠️ Never "deleted" and never "expired". The rows exist; the plan decides
    // what the API will show.
    if res.hidden_by_retention > 0 {
        println!();
        println!(
            "  {} {} older event{} {} not shown on this plan.",
            "⚠".yellow(),
            res.hidden_by_retention,
            if res.hidden_by_retention == 1 {
                ""
            } else {
                "s"
            },
            if res.hidden_by_retention == 1 {
                "is"
            } else {
                "are"
            }
        );
        if let Some(d) = res.retention_days {
            println!(
                "    This plan shows the last {d} day{}. The events are kept — this is a",
                if d == 1 { "" } else { "s" }
            );
            println!("    limit on what the API will return, not on what is stored.");
        }
    }

    // ⚠️ Said whenever a window was asked for and the plan is narrower, even if
    // nothing was actually withheld — otherwise `--last 30d` on a 7-day plan
    // reads as "the vault was quiet for 23 days".
    if let (Some(asked), Some(plan_days)) = (window_days, res.retention_days) {
        if asked > i64::from(plan_days) {
            println!();
            println!(
                "  {} You asked for {asked} days; this plan shows {plan_days}.",
                "ⓘ".dimmed()
            );
        }
    }

    if verbose {
        println!();
        println!(
            "  retention_days={:?}  hidden_by_retention={}  returned={}",
            res.retention_days,
            res.hidden_by_retention,
            res.events.len()
        );
    }

    println!();
    println!("{}", crate::docs::CLOUD.hint_line());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse() {
        assert_eq!(parse_window("all").unwrap(), None);
        assert_eq!(parse_window("7d").unwrap(), Some(7));
        assert_eq!(parse_window("30d").unwrap(), Some(30));
        assert_eq!(parse_window(" 30D ").unwrap(), Some(30));
    }

    /// ⚠️ A bad window must be an error, not a silent `all`. Widening a window
    /// someone narrowed on purpose is the wrong direction to be wrong in.
    #[test]
    fn a_bad_window_is_an_error_not_a_fallback() {
        for bad in ["7", "week", "0d", "-3d", ""] {
            assert!(parse_window(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
