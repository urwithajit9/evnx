//! `evnx cloud status` — what this machine is signed in to, and where from.

use anyhow::Result;
use colored::Colorize;

use super::client::Client;
use super::config::{self, CloudConfig};
use super::creds::Store;

/// Print the cloud-sync state for the resolved server.
///
/// Without `ping` this reads local files only and makes no network request, so it
/// stays useful when the server is down — "am I signed in?" and "can I reach the
/// server?" are different questions, and answering the first should not depend on
/// the second.
///
/// With `ping` it also calls `GET /health` and **returns an error when the server
/// cannot be reached**, so `evnx cloud status --ping` works as a health check in a
/// script. The local state is printed either way, before the check runs.
pub fn run(server_override: Option<&str>, ping: bool, verbose: bool) -> Result<()> {
    let server = CloudConfig::resolve_server(server_override)?;
    let store = Store::load()?;
    let now = chrono::Utc::now().timestamp();

    println!("{}", "evnx cloud".bold());
    println!("  server    {server}");

    match store.session(&server) {
        None => {
            println!("  status    {}", "not signed in".yellow());
            println!();
            println!("  No credentials for this server. Sign in with `evnx auth login`,");
            println!("  which arrives in a later release.");
        }
        Some(session) => {
            println!("  account   {}", session.email);
            if session.access_token_is_fresh(now) {
                let mins = (session.access_expires_at - now) / 60;
                println!("  status    {}", "signed in".green());
                println!("  token     valid for another {mins}m");
            } else {
                // Not an error: the refresh token is the long-lived one, and the
                // next command renews the access token without asking for a
                // password. Saying "expired" here would read as "signed out".
                println!("  status    {}", "signed in".green());
                println!(
                    "  token     {} — renewed automatically on the next command",
                    "expired".dimmed()
                );
            }
        }
    }

    // Where a bare `evnx cloud push` in this directory would go. Surfaced here
    // so it is never a guess — a binding can come from a parent directory.
    match std::env::current_dir()
        .ok()
        .and_then(|d| super::binding::read(&d).ok().flatten())
    {
        Some(bound) => {
            println!();
            println!("  vault     {}", bound.vault.bold());
            if verbose {
                println!("  bound by  {}", bound.path.display());
            }
        }
        None if verbose => {
            println!();
            println!(
                "  vault     {} — push and pull need --vault",
                "not bound".dimmed()
            );
        }
        None => {}
    }

    let others: Vec<&str> = store.servers().filter(|s| *s != server).collect();
    if !others.is_empty() {
        println!();
        println!("  Also signed in to:");
        for s in others {
            println!("    {s}");
        }
        println!("  Use --server to act on one of those instead.");
    }

    if verbose {
        println!();
        println!("  config      {}", config::config_path()?.display());
        println!("  credentials {}", config::credentials_path()?.display());
        println!("  No network request was made.");
    }

    if ping {
        println!();
        match Client::new(server.clone()).and_then(|c| c.health()) {
            Ok(health) => {
                println!(
                    "  reachable {} (server {} v{})",
                    "yes".green(),
                    health.status,
                    health.version
                );
                println!();
                println!("{}", crate::docs::CLOUD.hint_line());
                Ok(())
            }
            Err(e) => {
                println!("  reachable {}", "no".red());
                println!();
                // Returned rather than merely printed: the exit code is what a
                // health-check script reads. Re-wrapped as a flat message so the
                // reason prints on one line instead of as an anyhow chain that
                // repeats the server URL.
                Err(anyhow::anyhow!("{e}"))
            }
        }
    } else {
        println!();
        println!("{}", crate::docs::CLOUD.hint_line());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::creds::{SecretString, Session};
    use serial_test::serial;

    use crate::cloud::testutil::ConfigDirGuard;

    #[test]
    #[serial]
    fn status_works_with_no_config_at_all() {
        let _g = ConfigDirGuard::new();
        run(None, false, false).unwrap();
        run(None, false, true).unwrap();
    }

    #[test]
    #[serial]
    fn status_reads_the_session_for_the_selected_server_only() {
        let _g = ConfigDirGuard::new();
        let mut store = Store::default();
        store.set_session(
            "http://localhost:8099",
            Session {
                email: "dev@example.com".into(),
                access_token: SecretString::new("a"),
                access_expires_at: 4_000_000_000,
                refresh_token: SecretString::new("r"),
            },
        );
        store.save().unwrap();

        // Default server has no session; the localhost one does. Both render.
        run(None, false, false).unwrap();
        run(Some("http://localhost:8099"), false, true).unwrap();
    }

    #[test]
    #[serial]
    fn an_unusable_server_url_is_an_error_not_a_silent_default() {
        let _g = ConfigDirGuard::new();
        assert!(run(Some("http://api.evnx.dev"), false, false).is_err());
        assert!(run(Some("not-a-url"), false, false).is_err());
    }

    #[test]
    #[serial]
    fn ping_succeeds_against_a_healthy_server() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("GET", "/health")
            .with_status(200)
            .with_body(r#"{"status":"ok","version":"0.1.0"}"#)
            .create();

        run(Some(&server.url()), true, false).unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn ping_exits_non_zero_when_the_server_is_down() {
        // The whole point of --ping: a script can rely on the exit code.
        let _g = ConfigDirGuard::new();
        let err = run(Some("http://127.0.0.1:1"), true, false).unwrap_err();
        let rendered = format!("{err:#}");
        assert!(rendered.contains("could not reach"), "{rendered}");
        // One line, not an anyhow chain through reqwest and hyper.
        assert_eq!(rendered.lines().count(), 1, "{rendered}");
    }

    #[test]
    #[serial]
    fn without_ping_a_dead_server_is_not_an_error() {
        // Local state must still be answerable when the server is unreachable.
        let _g = ConfigDirGuard::new();
        run(Some("http://127.0.0.1:1"), false, false).unwrap();
    }
}
