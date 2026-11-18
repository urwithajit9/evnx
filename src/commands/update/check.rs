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

/// Where the tag lives. Unauthenticated unless `GITHUB_TOKEN` (or `GH_TOKEN`)
/// is set, and no other identifiers are sent.
const LATEST_RELEASE: &str = "https://api.github.com/repos/urwithajit9/evnx/releases/latest";

/// Shown when a release carries no `html_url`.
const RELEASES_PAGE: &str = "https://github.com/urwithajit9/evnx/releases/latest";

/// The page whose redirect names the latest tag: `…/releases/latest` answers
/// `302 Location: …/releases/tag/vX.Y.Z`, and costs no API quota.
const LATEST_REDIRECT: &str = "https://github.com/urwithajit9/evnx/releases/latest";

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

/// Where the redirect fallback asks. A **debug** build with `EVNX_UPDATE_URL`
/// set asks only what `EVNX_UPDATE_REDIRECT_URL` names (none when unset), so
/// the tests never reach the real site.
fn redirect_url() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("EVNX_UPDATE_URL").is_some_and(|v| !v.is_empty()) {
            return std::env::var("EVNX_UPDATE_REDIRECT_URL")
                .ok()
                .filter(|u| !u.is_empty());
        }
        if let Ok(url) = std::env::var("EVNX_UPDATE_REDIRECT_URL") {
            if !url.is_empty() {
                return Some(url);
            }
        }
    }
    Some(LATEST_REDIRECT.to_string())
}

/// The token for the API request, if one is set. Never printed, never logged.
fn token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

pub fn run(channel: Channel, json: bool, other_copies: usize, verbose: bool) -> Result<Verdict> {
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

    let report = Report::new(env!("CARGO_PKG_VERSION"), &fetch(verbose)?, channel);
    if json {
        crate::core::json::emit(&report.payload(other_copies), SCHEMA, None)?;
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

/// The latest release: from the API, and when that cannot answer (rate limit,
/// any other failure), from the redirect, which has no quota. `-v` says which
/// route answered.
fn fetch(verbose: bool) -> Result<Release> {
    let api = match fetch_api() {
        Ok(release) => {
            if verbose {
                ui::verbose_stderr("latest release: answered by the API");
            }
            return Ok(release);
        }
        Err(e) => e,
    };
    let Some(url) = redirect_url() else {
        return Err(api);
    };
    match fetch_redirect(&url) {
        Ok(release) => {
            if verbose {
                ui::verbose_stderr(format!("the API did not answer ({api:#})"));
                ui::verbose_stderr("latest release: answered by the releases/latest redirect");
            }
            Ok(release)
        }
        // The first failure is the one worth reading; the fallback is a spare.
        Err(_) => Err(api),
    }
}

/// The API request. With a token it is sent as a bearer, to this one URL.
fn fetch_api() -> Result<Release> {
    let client = http::client(TIMEOUT, None).context("building the HTTP client")?;

    let url = latest_release_url();
    // The host and port the request went to, not a fixed name.
    let host = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| u.port().map_or(h.to_string(), |p| format!("{h}:{p}")))
        })
        .unwrap_or_else(|| "api.github.com".to_string());
    let mut request = client.get(url);
    let authenticated = if let Some(token) = token() {
        request = request.bearer_auth(token);
        true
    } else {
        false
    };
    let resp = request.send().map_err(|e| {
        anyhow::anyhow!(
            "could not reach {host}: {}",
            http::transport_reason(&e, TIMEOUT)
        )
    })?;

    // ⚠️ 403 here is almost always the rate limit — 60 requests an hour per IP
    // without a token — not a permissions problem. Saying "forbidden" would send
    // someone looking for a token they may not need.
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        anyhow::bail!(
            "api.github.com is rate-limiting this address (HTTP {}).\n\
             \x20 {} Wait, or check \
             https://github.com/urwithajit9/evnx/releases/latest in a browser.",
            resp.status().as_u16(),
            if authenticated {
                "The token in GITHUB_TOKEN did not lift it."
            } else {
                "Requests without a token are capped at 60 an hour; set GITHUB_TOKEN to raise it."
            }
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

/// `GET <releases/latest>` without following the redirect, and the tag read from
/// `Location: …/tag/vX.Y.Z`. No token is sent: this goes to github.com, not
/// the API.
fn fetch_redirect(url: &str) -> Result<Release> {
    let client = http::client_with(TIMEOUT, None, false).context("building the HTTP client")?;
    let resp = client.get(url).send().map_err(|e| {
        anyhow::anyhow!(
            "could not reach {url}: {}",
            http::transport_reason(&e, TIMEOUT)
        )
    })?;
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .filter(|_| resp.status().is_redirection())
        .ok_or_else(|| anyhow::anyhow!("{url} did not redirect to a release"))?;
    release_from_location(location)
        .ok_or_else(|| anyhow::anyhow!("{url} redirected to something that is not a release tag"))
}

/// `https://github.com/o/r/releases/tag/v0.15.0` -> that release.
fn release_from_location(location: &str) -> Option<Release> {
    let tag = location
        .split_once("/releases/tag/")?
        .1
        .split(['?', '#'])
        .next()?
        .trim_end_matches('/');
    (tag.starts_with(|c: char| c == 'v' || c.is_ascii_digit()) && !tag.contains('/')).then(|| {
        Release {
            tag_name: tag.to_string(),
            html_url: Some(location.to_string()),
        }
    })
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
    /// Other copies of evnx on this machine: `evnx update --list` names them.
    other_copies: usize,
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

    fn payload(&self, other_copies: usize) -> Payload<'_> {
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
            other_copies,
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

    /// The redirect names the tag; a path that is not a tag names nothing.
    #[test]
    fn wp15_check_redirect_parses_tag() {
        let r = release_from_location("https://github.com/urwithajit9/evnx/releases/tag/v0.15.1")
            .unwrap();
        assert_eq!(r.tag_name, "v0.15.1");
        assert!(release_from_location("https://github.com/urwithajit9/evnx/releases").is_none());
        assert!(release_from_location("https://github.com/x/y/releases/tag/").is_none());
        assert!(release_from_location("https://github.com/x/y/releases/tag/v1/../x").is_none());
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
        let v = serde_json::to_value(r.payload(0)).unwrap();
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
                "other_copies": 0,
            })
        );
        let r = Report::new("0.6.0", &release("v0.7.0", Some("u")), Channel::Unknown);
        assert!(serde_json::to_value(r.payload(0)).unwrap()["command"].is_null());
    }
}
