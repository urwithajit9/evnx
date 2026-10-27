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
use serde::Serialize;
use std::time::Duration;

use super::Channel;
use crate::core::http;
use crate::core::verdict::Verdict;
use crate::utils::ui;

/// Where the tag lives. Unauthenticated, and no identifiers are sent.
const LATEST_RELEASE: &str = "https://api.github.com/repos/urwithajit9/evnx/releases/latest";

/// Shown when a release carries no `html_url`.
const RELEASES_PAGE: &str = "https://github.com/urwithajit9/evnx/releases/latest";

/// Short, because this is interactive and a hang is worse than a failure.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `--json`'s shape. Bumped when a consumer would notice (S-SF4).
const SCHEMA: u32 = 1;

#[derive(serde::Deserialize)]
struct Release {
    /// `v0.6.0`.
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
}

/// Where to ask.
///
/// ⚠️ `EVNX_UPDATE_URL` is read only by a **debug** build (DEC-SF10), so the
/// tests can answer from a loopback server. A release binary always asks
/// GitHub, and nothing in its environment can point it elsewhere.
fn latest_release_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("EVNX_UPDATE_URL") {
        if !url.is_empty() {
            return url;
        }
    }
    LATEST_RELEASE.to_string()
}

pub fn run(channel: Channel, json: bool) -> Result<Verdict> {
    let quiet = ui::is_quiet();

    // Say what is about to happen before it happens: this is the only network
    // request in the command, and a secrets tool should not surprise anyone with
    // one. On stderr, so stdout is only the answer (`--json` is parsed).
    if !quiet {
        eprintln!();
        eprintln!(
            "  {} api.github.com for the latest release…",
            "asking".dimmed()
        );
    }

    let report = Report::new(env!("CARGO_PKG_VERSION"), &fetch()?, channel);
    if json {
        println!("{}", serde_json::to_string_pretty(&report.payload())?);
    } else if quiet {
        // DEC-SF5: the status line and the command.
        println!("{}", report.status_line());
        if report.status == Ordering::Behind {
            super::print_commands_only(channel);
        }
    } else {
        report.print();
    }
    Ok(report.verdict())
}

/// The one request: the latest release, or why there is no answer.
fn fetch() -> Result<Release> {
    let client = http::client(TIMEOUT, None).context("building the HTTP client")?;

    let resp = client.get(latest_release_url()).send().map_err(|e| {
        anyhow::anyhow!(
            "could not reach api.github.com: {}",
            http::transport_reason(&e, TIMEOUT)
        )
    })?;

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

    resp.json()
        .context("reading the release information from api.github.com")
}

/// The answer, apart from how it is printed.
struct Report {
    installed: &'static str,
    latest: String,
    status: Ordering,
    channel: Channel,
    release_url: String,
}

#[derive(Serialize)]
struct Payload<'a> {
    schema: u32,
    installed: &'a str,
    latest: &'a str,
    /// `current`, `behind` or `ahead`.
    status: &'static str,
    channel: &'static str,
    /// `null` when evnx cannot name one command (see [`Channel::candidates`]).
    command: Option<&'static str>,
    release_url: &'a str,
}

impl Report {
    fn new(installed: &'static str, release: &Release, channel: Channel) -> Report {
        // The tag carries a leading `v`; the crate version does not.
        let latest = release.tag_name.trim_start_matches('v').to_string();
        Report {
            status: compare(installed, &latest),
            installed,
            latest,
            channel,
            release_url: release
                .html_url
                .clone()
                .unwrap_or_else(|| RELEASES_PAGE.to_string()),
        }
    }

    /// DEC-SF4: 1 when a newer release exists, the answer a script asks for.
    fn verdict(&self) -> Verdict {
        match self.status {
            Ordering::Behind => Verdict::Findings,
            Ordering::UpToDate | Ordering::Ahead => Verdict::Clean,
        }
    }

    fn payload(&self) -> Payload<'_> {
        Payload {
            schema: SCHEMA,
            installed: self.installed,
            latest: &self.latest,
            status: match self.status {
                Ordering::UpToDate => "current",
                Ordering::Behind => "behind",
                Ordering::Ahead => "ahead",
            },
            channel: self.channel.id(),
            command: self.channel.upgrade_command(),
            release_url: &self.release_url,
        }
    }

    fn status_line(&self) -> String {
        match self.status {
            Ordering::UpToDate => format!("{} you are on the latest release.", "✓".green()),
            Ordering::Behind => format!("{} a newer release is available.", "→".cyan()),
            // A development build, or a tag that has not been published yet.
            Ordering::Ahead => format!(
                "{} this build is newer than the latest release.",
                "note:".yellow()
            ),
        }
    }

    fn print(&self) {
        println!();
        println!("  installed   {}", self.installed.bold());
        println!("  latest      {}", self.latest.bold());
        println!();
        println!("  {}", self.status_line());

        if self.status == Ordering::Behind {
            println!();
            super::print_upgrade(self.channel, false);
            println!();
            println!("  Release notes: {}", self.release_url.dimmed());
            // ⚠️ Said wherever it can happen, because it is the one way this
            // output can mislead. Not for the install script, which downloads
            // from this very release (UP-4).
            if self.channel.can_lag() {
                println!();
                println!(
                    "  {} a channel can lag behind the release by minutes or days. If the",
                    "note:".yellow()
                );
                println!("  command above cannot find it yet, it has not propagated.");
            }
        }
        println!();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// T-SF-41, T-063: a fourth component is ignored, and so is a pre-release
    /// suffix on the latest tag.
    #[test]
    fn four_parts_and_a_pre_release_latest() {
        assert_eq!(cmp("0.7.0", "0.7.0.1"), "same");
        assert_eq!(cmp("0.7.0", "0.7.1-rc1"), "behind");
    }

    fn release(tag: &str, url: Option<&str>) -> Release {
        Release {
            tag_name: tag.to_string(),
            html_url: url.map(str::to_string),
        }
    }

    /// I-198, DEC-SF4: only "behind" is a finding; the `v` is dropped before
    /// comparing; a release with no page gets the releases list.
    #[test]
    fn t_i198_only_behind_is_a_finding() {
        let behind = Report::new("0.6.0", &release("v0.7.0", None), Channel::Cargo);
        assert_eq!(behind.latest, "0.7.0");
        assert_eq!(behind.verdict(), Verdict::Findings);
        assert_eq!(behind.release_url, RELEASES_PAGE);
        for tag in ["v0.6.0", "0.6.0", "v0.5.9"] {
            let r = Report::new("0.6.0", &release(tag, Some("u")), Channel::Cargo);
            assert_eq!(r.verdict(), Verdict::Clean, "{tag}");
        }
    }

    /// S-SF4: the JSON fields, with `command` null where evnx cannot name one.
    #[test]
    fn t_i198_the_payload_names_status_channel_and_command() {
        let r = Report::new("0.6.0", &release("v0.7.0", Some("u")), Channel::Cargo);
        let v = serde_json::to_value(r.payload()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "schema": 1,
                "installed": "0.6.0",
                "latest": "0.7.0",
                "status": "behind",
                "channel": "cargo",
                "command": "cargo install evnx --all-features --force",
                "release_url": "u",
            })
        );
        let r = Report::new("0.6.0", &release("v0.7.0", Some("u")), Channel::Unknown);
        assert!(serde_json::to_value(r.payload()).unwrap()["command"].is_null());
    }
}
