//! `evnx update --check` — the one request this command makes.
//!
//! # Why GitHub Releases and not a registry
//!
//! `release.yml` creates the GitHub Release from the `v*` tag, so a release exists
//! there the moment it exists at all. Every other channel derives from that tag and
//! can lag behind it:
//!
//! - **crates.io is published by hand.** There is no workflow for it, and the step
//!   has been skipped before — evnx-crypto's `0.1.1` and `0.1.2` are git tags and
//!   npm releases that were never `cargo publish`ed.
//! - **npm publishes on `workflow_run`** after the Release completes, and a skipped
//!   run still reports green.
//! - **winget merges on Microsoft's schedule**, which has taken 3.7 days.
//!
//! So the tag is the only source that is true by construction.
//!
//! ⚠️ **The consequence is stated to the user, not hidden:** a channel may not have
//! the newest version yet even though this says one exists. Reporting the tag and
//! saying so is more useful than reporting a registry that is itself behind.

use anyhow::{Context, Result};
use colored::Colorize;
use std::time::Duration;

use super::Channel;

/// Where the tag lives. Unauthenticated, and no identifiers are sent.
const LATEST_RELEASE: &str = "https://api.github.com/repos/urwithajit9/evnx/releases/latest";

/// Short, because this is interactive and a hang is worse than a failure.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(serde::Deserialize)]
struct Release {
    /// `v0.6.0`.
    tag_name: String,
    html_url: String,
}

pub fn run(channel: Channel) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    println!();
    // Say what is about to happen before it happens: this is the only network
    // request in the command, and a secrets tool should not surprise anyone with one.
    println!(
        "  {} api.github.com for the latest release…",
        "asking".dimmed()
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        // GitHub rejects requests with no user agent.
        .user_agent(concat!("evnx/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building the HTTP client")?;

    let resp = client
        .get(LATEST_RELEASE)
        .send()
        .map_err(|e| anyhow::anyhow!("could not reach api.github.com: {}", transport_reason(&e)))?;

    // ⚠️ 403 here is almost always the unauthenticated rate limit — 60 requests an
    // hour per IP — not a permissions problem. Saying "forbidden" would send someone
    // looking for a token they do not need.
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        anyhow::bail!(
            "api.github.com is rate-limiting this address (HTTP {}).\n\
             \x20 Unauthenticated requests are capped at 60 an hour. Wait, or check \
             https://github.com/urwithajit9/evnx/releases/latest in a browser.",
            resp.status().as_u16()
        );
    }
    if !resp.status().is_success() {
        anyhow::bail!(
            "api.github.com answered HTTP {}. Nothing was changed.",
            resp.status().as_u16()
        );
    }

    let release: Release = resp
        .json()
        .context("reading the release information from api.github.com")?;

    // The tag carries a leading `v`; the crate version does not.
    let latest = release.tag_name.trim_start_matches('v');

    println!();
    println!("  installed   {}", current.bold());
    println!("  latest      {}", latest.bold());
    println!();

    match compare(current, latest) {
        Ordering::UpToDate => {
            println!("  {} you are on the latest release.", "✓".green());
        }
        Ordering::Behind => {
            println!("  {} a newer release is available.", "→".cyan());
            println!();
            match channel.upgrade_command() {
                Some(command) => {
                    println!("  Installed via {}. Upgrade with:", channel.label().cyan());
                    println!();
                    println!("      {}", command.bold());
                }
                None => {
                    println!(
                        "  Run {} to see the upgrade commands.",
                        "evnx update".cyan()
                    );
                }
            }
            println!();
            println!("  Release notes: {}", release.html_url.dimmed());
            // ⚠️ Said every time, because it is the one way this output can mislead.
            println!();
            println!(
                "  {} a channel can lag behind the release by minutes or days. If the",
                "note:".yellow()
            );
            println!("  command above cannot find it yet, it has not propagated.");
        }
        Ordering::Ahead => {
            // A development build, or a tag that has not been published yet.
            println!(
                "  {} this build is newer than the latest release.",
                "note:".yellow()
            );
        }
    }
    println!();

    Ok(())
}

enum Ordering {
    UpToDate,
    Behind,
    Ahead,
}

/// Compare two dotted versions numerically.
///
/// ⚠️ Not a string comparison, which would put `0.10.0` **before** `0.9.0` and tell
/// a user on the newer release to upgrade to the older one. Missing or unparseable
/// components count as 0 rather than failing the whole command: a version check that
/// errors out is less useful than one that says "same".
fn compare(current: &str, latest: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .take(3)
            .collect()
    };
    let (a, b) = (parts(current), parts(latest));
    for i in 0..3 {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return if x < y {
                Ordering::Behind
            } else {
                Ordering::Ahead
            };
        }
    }
    Ordering::UpToDate
}

/// Shortest useful description of a transport failure.
///
/// Same reasoning as `cloud::client::transport_reason`: reqwest wraps hyper wraps
/// the OS error, so the default `Display` prints five nested lines with the only
/// actionable one last.
fn transport_reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!("timed out after {}s", TIMEOUT.as_secs());
    }
    let mut source: &dyn std::error::Error = e;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(a: &str, b: &str) -> &'static str {
        match compare(a, b) {
            Ordering::UpToDate => "same",
            Ordering::Behind => "behind",
            Ordering::Ahead => "ahead",
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(cmp("0.6.0", "0.6.0"), "same");
        assert_eq!(cmp("0.6.0", "0.7.0"), "behind");
        assert_eq!(cmp("0.7.0", "0.6.0"), "ahead");
        assert_eq!(cmp("0.6.0", "0.6.1"), "behind");
        assert_eq!(cmp("1.0.0", "0.9.9"), "ahead");
    }

    /// ⚠️ The bug a string comparison would have.
    ///
    /// `"0.10.0" < "0.9.0"` is true lexically, so a user on 0.10.0 would be told to
    /// downgrade. evnx will reach a two-digit minor long before a 1.0.
    #[test]
    fn a_two_digit_component_is_not_compared_as_text() {
        assert_eq!(cmp("0.10.0", "0.9.0"), "ahead");
        assert_eq!(cmp("0.9.0", "0.10.0"), "behind");
        assert_eq!(cmp("0.6.10", "0.6.9"), "ahead");
        // And the string form really would get it wrong, so the test is not vacuous.
        assert!("0.10.0" < "0.9.0");
    }

    #[test]
    fn a_missing_or_odd_component_does_not_fail_the_check() {
        assert_eq!(cmp("0.6", "0.6.0"), "same");
        assert_eq!(cmp("0.6.0", "0.6"), "same");
        // A pre-release suffix is ignored rather than rejected.
        assert_eq!(cmp("0.7.0-rc1", "0.7.0"), "same");
        assert_eq!(cmp("garbage", "0.7.0"), "behind");
    }
}
