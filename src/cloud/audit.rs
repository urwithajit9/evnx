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

use crate::cloud::client::api_path;
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
    if let Some(days) = t
        .strip_suffix('d')
        .and_then(|n| crate::core::args::parse_days(n).ok())
    {
        return Ok(Some(days));
    }
    Err(anyhow!(
        "`--last {raw}` is not a window. Use a number of days from 1d to {}d, like `7d` \
         or `30d`, or `all`.",
        crate::core::args::MAX_DAYS
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
        "pull" => {
            let version = num("version").map(|v| format!("version {v}"));
            // RM-57: rekey and revoke read every version to re-encrypt it. The
            // CLI sends `purpose=rekey` on those reads and the server records
            // it for an admin or the owner, so they do not read as that person
            // pulling each version by hand. WHO still names who did it.
            let rekey = text("purpose") == Some("rekey");
            match (version, rekey) {
                (Some(v), true) => format!("{v} · read for rekey"),
                (None, true) => "read for rekey".to_string(),
                (Some(v), false) => v,
                (None, false) => String::new(),
            }
        }
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
        "vault_rekey" => {
            let rewrapped = num("members")
                .map(|n| format!("re-wrapped for {n} member{}", if n == 1 { "" } else { "s" }));
            // RM-57: a revoke is logged as a rekey holding `removed_user_id`. The
            // server resolves it to `removed_email` (V0); older servers send
            // the id only, which is still worth showing over nothing.
            let removed = text("removed_email")
                .or_else(|| text("target_email"))
                .map(|e| format!("removed {e}"))
                .or_else(|| {
                    text("removed_user_id").map(|id| {
                        let short: String = id.chars().take(8).collect();
                        format!("removed a member (id {short}…)")
                    })
                });
            match (removed, rewrapped) {
                (Some(r), Some(w)) => format!("{r}; {w}"),
                (Some(r), None) => r,
                (None, Some(w)) => w,
                (None, None) => String::new(),
            }
        }
        _ => String::new(),
    }
}

/// The actor, as the WHO column shows it.
///
/// ⚠️ A deleted account is named as such rather than left blank: an empty
/// actor column reads as a bug, and "the account is gone" is a fact worth
/// knowing while reading a trail.
fn who(e: &AuditEvent) -> &str {
    e.actor_email.as_deref().unwrap_or("(deleted account)")
}

/// Widest WHO entry, at least the heading, at most [`WHO_MAX`].
fn who_width<'a>(entries: impl Iterator<Item = &'a str>) -> usize {
    entries
        .map(|w| w.chars().count())
        .max()
        .unwrap_or(0)
        .clamp("WHO".len(), WHO_MAX)
}

/// The widest the WHO column grows before an entry is shortened with `…`.
const WHO_MAX: usize = 40;

/// `s` shortened to `width` characters, ending in `…` when it was cut.
fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
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
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let res: AuditResponse =
        client.get(&api_path(&["vaults", &vault_ref.id.to_string(), "audit"])?)?;

    // `days_before`, not `Utc::now() - Duration::days(d)`: that panics out of
    // range (I-231). `parse_window` already capped `d`, so this is `Some`.
    let cutoff = window_days.and_then(|d| crate::core::args::days_before(chrono::Utc::now(), d));
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

    crate::say!();
    crate::say!("  {}  {}", "evnx cloud audit".bold(), vault_ref.label());
    crate::say!();

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
        // RM-57 (I-308): the WHO column is as wide as its longest entry, up to
        // a cap, so a long address neither pushes DETAIL off nor gets cut at 26.
        let width = who_width(rows.iter().map(|e| who(e)));
        println!(
            "  {:<19}  {:<18}  {:<width$}  {}",
            "WHEN".bold(),
            "WHAT".bold(),
            "WHO".bold(),
            "DETAIL".bold()
        );
        for e in &rows {
            let when: String = e.created_at.replace('T', " ").chars().take(19).collect();
            println!(
                "  {:<19}  {:<18}  {:<width$}  {}",
                when,
                e.event_type,
                fit(who(e), width),
                detail(e)
            );
        }
        crate::say!();
        crate::say!(
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

    crate::say!();
    crate::say!("{}", crate::docs::CLOUD.hint_line());
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

    fn event(kind: &str, actor: Option<&str>, meta: serde_json::Value) -> AuditEvent {
        AuditEvent {
            event_type: kind.into(),
            actor_email: actor.map(Into::into),
            metadata: Some(meta),
            created_at: "2026-11-08T00:00:00Z".into(),
        }
    }

    /// RM-57: a revoke names the removed member when the server resolves it,
    /// and falls back to the id when it does not.
    #[test]
    fn rm57_a_revoke_names_the_removed_member() {
        let e = event(
            "vault_rekey",
            Some("owner@acme.dev"),
            serde_json::json!({"members": 2, "removed_user_id": "0f1e2d3c-aaaa", "removed_email": "ex@acme.dev"}),
        );
        assert_eq!(detail(&e), "removed ex@acme.dev; re-wrapped for 2 members");
        let e = event(
            "vault_rekey",
            Some("owner@acme.dev"),
            serde_json::json!({"members": 1, "removed_user_id": "0f1e2d3c-aaaa"}),
        );
        assert_eq!(
            detail(&e),
            "removed a member (id 0f1e2d3c…); re-wrapped for 1 member"
        );
        let e = event("vault_rekey", None, serde_json::json!({"members": 3}));
        assert_eq!(detail(&e), "re-wrapped for 3 members");
    }

    /// RM-57: reads made by rekey are marked as such.
    #[test]
    fn rm57_rekey_reads_are_marked() {
        let e = event(
            "pull",
            Some("o@a.dev"),
            serde_json::json!({"version": 4, "purpose": "rekey"}),
        );
        assert_eq!(detail(&e), "version 4 · read for rekey");
        let e = event(
            "pull",
            Some("o@a.dev"),
            serde_json::json!({"version": 4, "purpose": "other"}),
        );
        assert_eq!(detail(&e), "version 4");
        let e = event("pull", Some("o@a.dev"), serde_json::json!({"version": 4}));
        assert_eq!(detail(&e), "version 4");
    }

    /// RM-57 (I-308): WHO grows to its longest entry, within bounds.
    #[test]
    fn rm57_who_column_is_width_aware() {
        assert_eq!(who_width(["a@b.c"].into_iter()), 5);
        assert_eq!(who_width(std::iter::empty()), 3);
        let long = "a-very-long-address-for-someone@a-long-company-domain.example";
        assert_eq!(who_width([long].into_iter()), WHO_MAX);
        let cut = fit(long, WHO_MAX);
        assert_eq!(cut.chars().count(), WHO_MAX);
        assert!(cut.ends_with('…'));
        assert_eq!(fit("x@y.z", 10), "x@y.z");
    }

    /// ⚠️ A bad window must be an error, not a silent `all`. Widening a window
    /// someone narrowed on purpose is the wrong direction to be wrong in.
    #[test]
    fn a_bad_window_is_an_error_not_a_fallback() {
        for bad in [
            "7",
            "week",
            "0d",
            "-3d",
            "",
            "3651d",
            "9223372036854775807d",
        ] {
            assert!(parse_window(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
