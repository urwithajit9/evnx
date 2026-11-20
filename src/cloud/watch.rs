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
//!
//! ─── `--on-change -- <COMMAND>...` ──────────────────────────────────────────
//!
//! Runs a command once per reported change (the rule that prints the line, so
//! your own pushes are skipped unless `--include-mine`). Three rules, one
//! reason each:
//!
//! * **No shell.** The command is run as `evnx run` runs it: `program` and
//!   arguments, nothing interpreted. A key name or a vault name can never
//!   become shell syntax, and Windows and Unix behave alike. A pipeline asks
//!   for a shell: `-- sh -c '...'`.
//! * **Nothing from evnx's environment.** Every `EVNX_*` variable is stripped
//!   (the API token above all), except the two this adds: `EVNX_WATCH_VAULT`
//!   (the vault's `name/environment`) and `EVNX_WATCH_VERSION` (the new latest
//!   number). Names and numbers, never a value: watch has none to give.
//! * **Synchronous, and a failing command does not stop the watch.** The next
//!   poll waits for the command to finish, so changes that arrive meanwhile
//!   are one call afterwards, not a queue. A non-zero exit or a command that
//!   could not start prints one warning and watching goes on: a hook that
//!   failed once is not a reason to stop telling people about changes.

use anyhow::{Result, anyhow};
use colored::Colorize;
use std::time::Duration;

use super::client::Client;
use super::config::CloudConfig;
use super::sync::{
    VersionSummary, key_delta, key_delta_line, list_versions, my_user_id, resolve_target,
};
use super::vault;
use crate::core::secret::Vars;

/// ⚠️ A floor, not a suggestion. A one-second poll against a shared API is a
/// denial-of-service someone writes by accident, and the value of knowing two
/// seconds sooner is zero.
const MIN_INTERVAL_SECS: u64 = 5;
// ⓘ The default lives in the clap attribute on `CloudCommands::Watch`, so there
// is one place a reader checks and `--help` cannot disagree with the code.

/// Failed polls in a row before a warning on stderr (DEC-CL5).
const WARN_AFTER: u32 = 3;
/// Failed polls in a row before watch stops with exit 2 (owner, 2026-10-25).
const STOP_AFTER: u32 = 10;

fn newest(versions: &[VersionSummary]) -> Option<&VersionSummary> {
    versions.iter().max_by_key(|v| v.version_num)
}

/// What to say about a run of failed polls (I-229, DEC-CL5).
///
/// ⚠️ Failures were shown only with `-v`, so an expired session or an outage
/// looked like a quiet vault, for as long as the watcher ran.
#[derive(Default)]
struct PollHealth {
    failed: u32,
}

#[derive(Debug, PartialEq)]
enum Health {
    /// Too few in a row to mention (a blip), unless `-v`.
    Quiet,
    /// Say so on stderr: this many in a row.
    Warn(u32),
    /// Give up: exit 2 with what to check.
    Stop,
}

impl PollHealth {
    fn failure(&mut self) -> Health {
        self.failed += 1;
        if self.failed >= STOP_AFTER {
            Health::Stop
        } else if self.failed >= WARN_AFTER {
            Health::Warn(self.failed)
        } else {
            Health::Quiet
        }
    }

    /// A poll worked. Returns how many had failed, if that was worth a warning,
    /// so the recovery can be said too.
    fn success(&mut self) -> Option<u32> {
        let was = std::mem::take(&mut self.failed);
        (was >= WARN_AFTER).then_some(was)
    }
}

/// Whether versions newer than `last` are worth reporting: any pushed by
/// someone else, or any at all with `--include-mine` (I-229, DEC-CL5). The
/// README always said watch tells you "when someone else pushes"; it reported
/// your own pushes as well.
fn needs_report(fresh: &[VersionSummary], last: i32, me: Option<&str>, include_mine: bool) -> bool {
    fresh
        .iter()
        .filter(|v| v.version_num > last)
        .any(|v| include_mine || me != Some(v.pushed_by.as_str()))
}

/// The variables the `--on-change` command gets, and the only `EVNX_*` it gets.
///
/// A vault's label and a version number: public facts the server already
/// shows to anyone who can list the vault. Never a value.
fn hook_env(vault_label: &str, version: i32) -> Vars {
    let mut vars = Vars::new();
    vars.insert("EVNX_WATCH_VAULT".to_string(), vault_label.into());
    vars.insert("EVNX_WATCH_VERSION".to_string(), version.to_string().into());
    vars
}

/// Run the `--on-change` command for a change and say how it went.
///
/// Returns whether it succeeded. Never an error: see the module note.
fn run_hook(command: &[String], vault_label: &str, version: i32) -> bool {
    let Some((program, args)) = command.split_first() else {
        return true;
    };
    let env = hook_env(vault_label, version);
    // The program only: the arguments are the user's, but a line that echoes
    // them is a place a token typed into one would end up in a log.
    crate::say!(
        "    {}",
        format!("running {program} for v{version}").dimmed()
    );
    match crate::core::inject::spawn_with_env(program, args, &env, None) {
        Ok(status) if status.success() => true,
        Ok(status) => {
            let how = status.code().map_or_else(
                || "was stopped by a signal".to_string(),
                |c| format!("exited {c}"),
            );
            crate::utils::ui::warning(format!(
                "the --on-change command {program} {how} for v{version}; watching goes on."
            ));
            false
        }
        Err(e) => {
            let why = if e.kind() == std::io::ErrorKind::NotFound {
                "no such command".to_string()
            } else {
                e.to_string()
            };
            crate::utils::ui::warning(format!(
                "the --on-change command {program} did not run for v{version} ({why}); \
                 watching goes on. It is run directly, not through a shell: \
                 `-- sh -c '...'` for a pipeline."
            ));
            false
        }
    }
}

/// Print one reported change, and run the command if there is one.
#[allow(clippy::too_many_arguments)]
fn report_change(
    vault_ref: &vault::VaultRef,
    previous: Option<&VersionSummary>,
    newest_now: &VersionSummary,
    verbose: bool,
    hook: Option<&[String]>,
) {
    let line = match previous {
        Some(p) => key_delta_line(&key_delta(p, newest_now)),
        None => format!("{} keys", newest_now.key_count),
    };

    let when = super::when::stamp(&newest_now.pushed_at);
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

    if let Some(command) = hook {
        run_hook(command, &vault_ref.label(), newest_now.version_num);
    }
}

pub fn run(
    server_override: Option<&str>,
    vault_target: Option<String>,
    interval_secs: u64,
    once: bool,
    include_mine: bool,
    hook: Option<Vec<String>>,
    verbose: bool,
) -> Result<()> {
    if interval_secs < MIN_INTERVAL_SECS {
        return Err(anyhow!(
            "--interval {interval_secs} is below the {MIN_INTERVAL_SECS}s minimum. \
             Polling faster does not make a teammate push sooner."
        ));
    }

    if hook.as_ref().is_some_and(Vec::is_empty) {
        return Err(crate::core::verdict::usage(
            "--on-change needs a command after `--`: evnx cloud watch --on-change -- <COMMAND>",
        ));
    }
    let hook = hook.as_deref();

    let vault_target = resolve_target(vault_target)?;
    let server = CloudConfig::resolve_server(server_override)?;
    let client = Client::new(server.clone())?;
    client.require_credential()?;
    let vault_ref = vault::fetch_and_resolve(&client, &vault_target)?;

    let mut seen = list_versions(&client, &vault_ref)?;
    let mut last = newest(&seen).map(|v| v.version_num).unwrap_or(0);
    let me = my_user_id(&client);
    let mut health = PollHealth::default();

    crate::say!();
    crate::say!("  {}  {}", "evnx cloud watch".bold(), vault_ref.label());
    if last == 0 {
        crate::say!("  No versions yet. Watching for the first push.");
    } else {
        crate::say!("  At v{last}. Watching for newer versions.");
    }
    crate::say!(
        "  {}",
        format!("Polling every {interval_secs}s · metadata only, no password · Ctrl-C to stop")
            .dimmed()
    );
    if !include_mine {
        crate::say!(
            "  {}",
            "Your own pushes are skipped; --include-mine shows them.".dimmed()
        );
    }
    crate::say!();

    loop {
        if once {
            // A change is pending when the vault is past what this machine has
            // seen (the high-water mark, raised by its own push, pull and run).
            // That is the one thing a single check can compare with, and it is
            // only worth doing when there is a command to run.
            if let (Some(command), Some(mark)) = (hook, highwater_mark(&vault_ref)) {
                if let Some(newest_now) = newest(&seen).filter(|n| n.version_num > mark) {
                    if needs_report(&seen, mark, me.as_deref(), include_mine) {
                        let previous = seen.iter().find(|v| v.version_num == mark);
                        report_change(&vault_ref, previous, newest_now, verbose, Some(command));
                    }
                }
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(interval_secs));

        // ⚠️ One failed poll is a blip and the loop goes on: a watcher that
        // exits on a network hiccup is one nobody trusts to leave running. A run
        // of them is not, and is said (I-229).
        let fresh = match list_versions(&client, &vault_ref) {
            Ok(v) => {
                if let Some(n) = health.success() {
                    eprintln!("  {} polling again, after {n} failed poll(s)", "✓".green());
                }
                v
            }
            Err(e) => {
                match health.failure() {
                    Health::Quiet if verbose => {
                        eprintln!("  {} poll failed: {e}", "!".yellow())
                    }
                    Health::Quiet => {}
                    Health::Warn(n) => eprintln!(
                        "  {} {n} polls in a row have failed (stopping at {STOP_AFTER}): {e}",
                        "!".yellow()
                    ),
                    Health::Stop => {
                        return Err(super::client::reword(
                            super::client::exit_code_of(&e),
                            format!(
                                "stopped after {STOP_AFTER} failed polls in a row: {e}\n\
                                 \x20 Check the connection and your login (`evnx auth status`), \
                                 then run `evnx cloud watch` again."
                            ),
                        ));
                    }
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
        if !needs_report(&fresh, last, me.as_deref(), include_mine) {
            // Only your own pushes: move on silently, so a teammate's next push
            // is compared with what is there now.
            last = newest_now.version_num;
            seen = fresh;
            continue;
        }

        // Compare against the version we last reported, not against the
        // immediately previous one — two pushes between polls are one event to
        // the reader, and showing only the last hop would hide a key that
        // appeared and vanished.
        let previous = seen.iter().find(|v| v.version_num == last);
        report_change(&vault_ref, previous, newest_now, verbose, hook);

        last = newest_now.version_num;
        seen = fresh;
    }
}

/// This machine's mark for the vault, if it has one.
fn highwater_mark(vault_ref: &vault::VaultRef) -> Option<i32> {
    super::highwater::seen(&vault_ref.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ The floor has to be enforced, not documented. A `--interval 1` in a
    /// CI loop is how a shared API gets hammered by someone being helpful.
    fn v(n: i32, by: &str) -> VersionSummary {
        VersionSummary {
            version_num: n,
            key_count: 1,
            key_names: vec!["A".into()],
            blob_hash: format!("h{n}"),
            blob_size_bytes: 10,
            pushed_by: by.into(),
            pushed_at: "2026-10-25T09:00:00Z".into(),
        }
    }

    /// I-229 (DEC-CL5): quiet for a blip, a warning from the 3rd failure in a
    /// row, a stop at the 10th; a success resets and says it recovered only
    /// when a warning was given.
    #[test]
    fn t_i229_failed_polls_warn_then_stop() {
        let mut h = PollHealth::default();
        assert_eq!(h.failure(), Health::Quiet);
        assert_eq!(h.failure(), Health::Quiet);
        assert_eq!(h.success(), None, "two failures were never mentioned");
        for _ in 0..2 {
            assert_eq!(h.failure(), Health::Quiet);
        }
        for n in 3..STOP_AFTER {
            assert_eq!(h.failure(), Health::Warn(n));
        }
        assert_eq!(h.failure(), Health::Stop);
        assert_eq!(h.success(), Some(STOP_AFTER));
        assert_eq!(h.failure(), Health::Quiet, "the count starts again");
    }

    /// I-229: your own pushes are not news, unless asked for; one from someone
    /// else among them is.
    #[test]
    fn t_i229_only_someone_elses_push_is_reported() {
        let fresh = [v(1, "me"), v(2, "me"), v(3, "me")];
        assert!(!needs_report(&fresh, 1, Some("me"), false));
        assert!(needs_report(&fresh, 1, Some("me"), true));
        let mixed = [v(1, "me"), v(2, "bob"), v(3, "me")];
        assert!(needs_report(&mixed, 1, Some("me"), false));
        // Nothing new at all.
        assert!(!needs_report(&mixed, 3, Some("me"), true));
        // Who you are is unknown: every push is reported, as before.
        assert!(needs_report(&fresh, 1, None, false));
    }

    /// The command's environment is the vault label and the version number, and
    /// nothing else: no value, no key name.
    #[test]
    fn the_hook_environment_is_the_vault_and_the_version_only() {
        let env = hook_env("app/production", 7);
        let got: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.clone(), v.expose().to_string()))
            .collect();
        assert_eq!(
            got,
            [
                ("EVNX_WATCH_VAULT".to_string(), "app/production".to_string()),
                ("EVNX_WATCH_VERSION".to_string(), "7".to_string()),
            ]
        );
    }

    /// A failing command is a warning, not an end to watching: `run_hook` says
    /// it failed and returns, for a non-zero exit and for a command that does
    /// not exist alike.
    #[test]
    fn a_failing_or_missing_command_is_not_fatal() {
        let sh = |s: &str| vec!["sh".to_string(), "-c".to_string(), s.to_string()];
        assert!(run_hook(&sh("exit 0"), "app/production", 2));
        assert!(!run_hook(&sh("exit 3"), "app/production", 2));
        assert!(!run_hook(
            &["evnx-no-such-program-16b2".to_string()],
            "app/production",
            2
        ));
        assert!(run_hook(&[], "app/production", 2));
    }

    /// The command sees the two watch variables. (That every other `EVNX_*`
    /// variable is stripped is `inject`'s own test, `t_i227_*`, as this runs
    /// through the same launcher.)
    #[test]
    #[cfg(unix)]
    fn the_hook_sees_its_two_variables() {
        let dir = tempfile::TempDir::new().unwrap();
        let marker = dir.path().join("out");
        let script = format!(
            r#"printf '%s|%s' "$EVNX_WATCH_VAULT" "$EVNX_WATCH_VERSION" > '{}'"#,
            marker.display()
        );
        let ok = run_hook(
            &["sh".to_string(), "-c".to_string(), script],
            "app/production",
            4,
        );
        assert!(ok);
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "app/production|4");
    }

    #[test]
    fn an_interval_below_the_floor_is_refused() {
        for bad in [0, 1, 4] {
            let e = run(None, Some("x".into()), bad, true, false, None, false).unwrap_err();
            assert!(
                e.to_string().contains("minimum"),
                "interval {bad} should be refused: {e}"
            );
        }
    }
}
