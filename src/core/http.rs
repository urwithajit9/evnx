//! The one HTTP client evnx builds, and how a failed request is worded (A-35,
//! DEC-SF9).
//!
//! `update --check` and every `cloud` command built their own
//! `reqwest::blocking::Client` and carried their own copy of the transport
//! wording, which differed only in the timeout it named. One builder is where
//! the user agent, the timeouts and — once DEC-SF1 is decided (I-192, C-08) —
//! the trusted root certificates are set, so a fix lands for every caller at
//! once.
//!
//! ⚠️ `migrate`'s GitHub destination still builds `Client::new()` with no
//! timeout. Moving it here would give it one, which changes `migrate`: that is
//! R-migrate's decision, and `tests/architecture.rs` records it.

use std::time::Duration;

/// Sent with every request. GitHub rejects requests with no user agent.
pub const USER_AGENT: &str = concat!("evnx/", env!("CARGO_PKG_VERSION"));

/// A blocking client with evnx's user agent, an overall `timeout`, and, when
/// given, a separate `connect_timeout`.
pub fn client(
    timeout: Duration,
    connect_timeout: Option<Duration>,
) -> reqwest::Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT);
    if let Some(connect) = connect_timeout {
        builder = builder.connect_timeout(connect);
    }
    builder.build()
}

/// Shortest useful description of a request that never completed.
///
/// `reqwest::Error`'s own `Display` is "error sending request for url (...)",
/// which repeats the URL the caller already knows and says nothing about the
/// cause. The cause sits at the end of a chain through hyper down to the OS
/// error, so this walks to the root and reports that instead — "Connection
/// refused (os error 111)" rather than four wrappers around it.
///
/// Timeouts are special-cased because their root cause is an opaque internal
/// type, and because "timed out" is what the user needs to read; `timeout` is
/// the limit the caller set.
pub fn transport_reason(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        return format!("timed out after {}s", timeout.as_secs());
    }
    let mut root: &dyn std::error::Error = e;
    while let Some(next) = root.source() {
        root = next;
    }
    root.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused connection reads as one line naming the OS error, not a
    /// chain through reqwest and hyper.
    #[test]
    fn a_refused_connection_is_one_line() {
        let timeout = Duration::from_secs(5);
        let c = client(timeout, Some(Duration::from_secs(2))).unwrap();
        let err = c.get("http://127.0.0.1:9/").send().unwrap_err();
        let reason = transport_reason(&err, timeout);
        assert_eq!(reason.lines().count(), 1, "{reason}");
        assert!(!reason.is_empty());
    }
}
