//! `evnx cloud status` — what this machine is signed in to, and where from.

use anyhow::Result;
use colored::Colorize;

use super::config::{self, CloudConfig};
use super::creds::Store;

/// Print the cloud-sync state for the resolved server.
///
/// Reads only local files and makes no network request, so it stays useful when
/// the server is unreachable — "am I signed in?" and "can I reach the server?"
/// are different questions and this one answers the first.
pub fn run(server_override: Option<&str>, verbose: bool) -> Result<()> {
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

    println!();
    println!("{}", crate::docs::CLOUD.hint_line());
    Ok(())
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
        run(None, false).unwrap();
        run(None, true).unwrap();
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
        run(None, false).unwrap();
        run(Some("http://localhost:8099"), true).unwrap();
    }

    #[test]
    #[serial]
    fn an_unusable_server_url_is_an_error_not_a_silent_default() {
        let _g = ConfigDirGuard::new();
        assert!(run(Some("http://api.evnx.dev"), false).is_err());
        assert!(run(Some("not-a-url"), false).is_err());
    }
}
