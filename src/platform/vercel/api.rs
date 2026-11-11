//! The Vercel REST calls `diff` and `pull` need, and nothing else.
//!
//! Facts this is written to, all verified live (design §2a):
//!
//! * `GET /v10/projects/{id}/env` lists variables and **never returns a value**.
//!   A variable is a key plus a set of targets, and one key may have several
//!   entries with different target sets.
//! * `GET /v1/projects/{id}/env/{envId}` returns a **Config** value. For a
//!   **Secret** (`type: sensitive`, or `visibility: secret`) it returns no
//!   `value` field at all. That absence means write-only: it is never read as an
//!   empty string. The deprecated `decrypt` parameter is not used.
//! * Rate limits come back as `x-ratelimit-remaining` and `x-ratelimit-reset`
//!   (Unix seconds). A 429 waits until the reset (capped), three requests at
//!   most, then exit 15.
//!
//! The one HTTP client is `core::http::client`; the token goes in the
//! `Authorization` header and nowhere else. No error and no log line here holds
//! a value.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use reqwest::header::{HeaderMap, ACCEPT};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;

use crate::platform::error::PlatformError;
use crate::platform::secret::SecretString;

/// Where the API lives. Overridden by `EVNX_VERCEL_API_URL` / the hidden
/// `--api-url`, which is how the tests reach a mock server.
pub const DEFAULT_BASE: &str = "https://api.vercel.com";

/// Time allowed to establish a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed for one request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Requests made for one call before giving up on 429.
const MAX_TRIES: u32 = 3;
/// The longest evnx waits for a rate limit to clear, in one go.
const MAX_WAIT_SECS: u64 = 60;
/// A listing that never ends is a server bug; stop.
const MAX_PAGES: usize = 200;

/// How a team is named in the query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Team {
    /// `team_…`, sent as `teamId`.
    Id(String),
    /// A slug, sent as `slug`.
    Slug(String),
}

impl Team {
    /// Classify what the user gave.
    pub fn from_ref(t: &str) -> Team {
        if t.starts_with("team_") {
            Team::Id(t.to_string())
        } else {
            Team::Slug(t.to_string())
        }
    }

    fn query(&self) -> (&'static str, &str) {
        match self {
            Team::Id(t) => ("teamId", t),
            Team::Slug(t) => ("slug", t),
        }
    }
}

/// Config is readable; Secret is write-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `encrypted` or `plain`: the value can be read back.
    Config,
    /// `sensitive` (legacy) or `visibility: secret`: the value can never be read.
    Secret,
}

impl Kind {
    /// The word the plan prints.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Config => "config",
            Kind::Secret => "secret",
        }
    }
}

/// One variable entry as the list returns it. No value: the list never has one.
#[derive(Debug, Clone)]
pub struct RemoteVar {
    /// Vercel's id for this entry.
    pub id: String,
    /// The variable's name.
    pub key: String,
    /// Config or Secret, from the list.
    pub kind: Kind,
    /// The Vercel `type` as sent (`encrypted`, `plain`, `sensitive`, …).
    pub raw_type: String,
    /// Environments this entry applies to.
    pub target: Vec<String>,
    /// A preview entry scoped to one branch.
    pub git_branch: Option<String>,
    /// `updatedAt` exactly as sent (a number of milliseconds), for the state file.
    pub updated_at: Value,
}

/// What a list call returned, all pages.
#[derive(Debug, Default)]
pub struct Listing {
    /// Every entry.
    pub vars: Vec<RemoteVar>,
    /// Production variables this token cannot see (`hiddenProductionEnvCount`).
    pub hidden_production: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Target {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct RawEnv {
    id: String,
    key: String,
    #[serde(default, rename = "type")]
    type_: String,
    #[serde(default)]
    target: Option<Target>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default, rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<Value>,
}

impl RawEnv {
    fn into_var(self) -> RemoteVar {
        let kind = if matches!(self.type_.as_str(), "sensitive" | "secret")
            || self.visibility.as_deref() == Some("secret")
        {
            Kind::Secret
        } else {
            Kind::Config
        };
        RemoteVar {
            id: self.id,
            key: self.key,
            kind,
            raw_type: self.type_,
            target: match self.target {
                Some(Target::One(t)) => vec![t],
                Some(Target::Many(t)) => t,
                None => Vec::new(),
            },
            git_branch: self.git_branch.filter(|b| !b.is_empty()),
            updated_at: self.updated_at.unwrap_or(Value::Null),
        }
    }
}

/// A client for one project.
pub struct Api {
    http: reqwest::blocking::Client,
    base: Url,
    host: String,
    token: SecretString,
    team: Option<Team>,
    /// Do not send before this Unix second: the last answer said none were left.
    pause_until: Cell<u64>,
}

impl Api {
    /// A client for `base` (see [`parse_base`]).
    pub fn new(base: Url, token: SecretString, team: Option<Team>) -> Result<Api> {
        let host = base.host_str().unwrap_or("the Vercel API").to_string();
        Ok(Api {
            http: crate::core::http::client(REQUEST_TIMEOUT, Some(CONNECT_TIMEOUT))?,
            base,
            host,
            token,
            team,
            pause_until: Cell::new(0),
        })
    }

    fn url(&self, segments: &[&str], extra: &[(&str, &str)]) -> Url {
        let mut url = self.base.clone();
        {
            // `base` is http(s), which can always be a base; `parse_base` checked.
            let mut path = url.path_segments_mut().expect("an http(s) URL has a path");
            path.pop_if_empty().extend(segments);
        }
        {
            let mut q = url.query_pairs_mut();
            if let Some(t) = &self.team {
                let (k, v) = t.query();
                q.append_pair(k, v);
            }
            for (k, v) in extra {
                q.append_pair(k, v);
            }
        }
        url
    }

    /// Every variable entry of the project, following `pagination.next`.
    pub fn list_envs(&self, project: &str) -> Result<Listing> {
        let mut listing = Listing::default();
        let mut until: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let extra: Vec<(&str, &str)> =
                until.as_deref().map(|u| ("until", u)).into_iter().collect();
            let url = self.url(&["v10", "projects", project, "env"], &extra);
            let body = self.get_json(url, &format!("the variables of project {project}"))?;
            let envs = body
                .get("envs")
                .and_then(Value::as_array)
                .ok_or_else(|| PlatformError::Malformed("the list has no `envs` array".into()))?;
            for e in envs {
                let raw: RawEnv = serde_json::from_value(e.clone()).map_err(|_| {
                    PlatformError::Malformed("a variable has no id or no key".into())
                })?;
                listing.vars.push(raw.into_var());
            }
            let hidden = body
                .get("hiddenProductionEnvCount")
                .or_else(|| body.pointer("/pagination/hiddenProductionEnvCount"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            listing.hidden_production = listing.hidden_production.max(hidden);

            let next = match body.pointer("/pagination/next") {
                Some(Value::Number(n)) => Some(n.to_string()),
                Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            match next {
                Some(n) if until.as_deref() != Some(n.as_str()) => until = Some(n),
                _ => return Ok(listing),
            }
        }
        Err(PlatformError::Malformed("the list never ended".into()).into())
    }

    /// The value of one **Config** variable. `None` is a variable Vercel will
    /// not reveal: write-only, never an empty string.
    pub fn read_value(&self, project: &str, id: &str) -> Result<Option<SecretString>> {
        if !is_plain_id(id) {
            return Err(PlatformError::Malformed("a variable id is not an id".into()).into());
        }
        let url = self.url(&["v1", "projects", project, "env", id], &[]);
        let body = self.get_json(url, "a variable")?;
        match body.get("value") {
            Some(Value::String(s)) => Ok(Some(SecretString::new(s.as_str()))),
            None | Some(Value::Null) => Ok(None),
            Some(_) => Err(PlatformError::Malformed("a value is not a string".into()).into()),
        }
    }

    /// One GET: pacing, the 429 wait, and the status mapping.
    fn get_json(&self, url: Url, what: &str) -> Result<Value, PlatformError> {
        let mut tries = 0;
        loop {
            tries += 1;
            self.pace();
            let resp = self
                .http
                .get(url.clone())
                .bearer_auth(self.token.expose())
                .header(ACCEPT, "application/json")
                .send()
                .map_err(|e| PlatformError::Transport {
                    host: self.host.clone(),
                    reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
                })?;
            let status = resp.status();
            let headers = resp.headers().clone();
            self.note_limits(&headers);

            if status == StatusCode::TOO_MANY_REQUESTS {
                if tries >= MAX_TRIES {
                    return Err(PlatformError::RateLimited { tries });
                }
                let wait = wait_secs(&headers, now_secs(), tries);
                std::thread::sleep(Duration::from_secs(wait));
                continue;
            }

            let text = resp.text().map_err(|e| PlatformError::Transport {
                host: self.host.clone(),
                reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
            })?;
            return match status.as_u16() {
                200..=299 => serde_json::from_str(&text)
                    .map_err(|_| PlatformError::Malformed("the body is not JSON".into())),
                401 => Err(PlatformError::Unauthorized),
                403 if invalid_token(&text) => Err(PlatformError::InvalidToken),
                403 => Err(PlatformError::Forbidden {
                    detail: server_sentence(&text),
                }),
                404 => Err(PlatformError::NotFound {
                    what: what.to_string(),
                }),
                s @ 400..=499 => Err(PlatformError::Refused {
                    status: s,
                    detail: server_sentence(&text),
                }),
                s @ 500..=599 => Err(PlatformError::ServerError { status: s }),
                s => Err(PlatformError::Unexpected { status: s }),
            };
        }
    }

    /// Sleep out a rate limit the last response said was used up.
    fn pace(&self) {
        let now = now_secs();
        let until = self.pause_until.get();
        if until > now {
            std::thread::sleep(Duration::from_secs((until - now).min(MAX_WAIT_SECS)));
        }
        self.pause_until.set(0);
    }

    fn note_limits(&self, headers: &HeaderMap) {
        let remaining = header_u64(headers, "x-ratelimit-remaining");
        let reset = header_u64(headers, "x-ratelimit-reset");
        if let (Some(0), Some(reset)) = (remaining, reset) {
            self.pause_until.set(reset);
        }
    }
}

/// An id that is safe as a path segment.
fn is_plain_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Seconds to wait after a 429: `Retry-After`, else until `x-ratelimit-reset`,
/// else a short growing pause; never more than 60 s.
pub fn wait_secs(headers: &HeaderMap, now: u64, attempt: u32) -> u64 {
    let wait = header_u64(headers, "retry-after")
        .or_else(|| header_u64(headers, "x-ratelimit-reset").map(|r| r.saturating_sub(now)))
        .unwrap_or(u64::from(attempt));
    wait.min(MAX_WAIT_SECS)
}

/// The `error.message` of a Vercel error body, cut short. Only used for 4xx
/// answers to GETs, which carry no value.
fn server_sentence(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let m = v.pointer("/error/message")?.as_str()?;
    let m: String = m.chars().filter(|c| !c.is_control()).take(200).collect();
    (!m.is_empty()).then_some(m)
}

/// Whether a 403 body says the token itself is bad:
/// `{"error":{"code":"forbidden","invalidToken":true}}`.
fn invalid_token(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/invalidToken").and_then(Value::as_bool))
        == Some(true)
}

/// Validate and normalise the API base. Plain `http://` is allowed only for a
/// loopback address (the tests, a local proxy): the token travels in a header.
pub fn parse_base(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim())
        .map_err(|e| anyhow::anyhow!("the Vercel API URL {raw:?} is not a URL ({e})"))?;
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("the Vercel API URL must not carry a user name or password");
    }
    if url.cannot_be_a_base() || url.host().is_none() {
        anyhow::bail!("the Vercel API URL {raw:?} has no host");
    }
    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
            let loopback = host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::Ipv4Addr>()
                    .is_ok_and(|a| a.is_loopback())
                || host
                    .parse::<std::net::Ipv6Addr>()
                    .is_ok_and(|a| a.is_loopback());
            if !loopback {
                anyhow::bail!(
                    "refusing plain http:// to {}: the Vercel token travels in a request header. \
                     Use https://, or a loopback address for local testing.",
                    url.host_str().unwrap_or("that host")
                );
            }
        }
        other => anyhow::bail!("unsupported URL scheme {other:?} for the Vercel API; use https://"),
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn the_wait_is_until_the_reset_and_capped() {
        let now = 1_000;
        assert_eq!(
            wait_secs(&headers(&[("x-ratelimit-reset", "1007")]), now, 1),
            7
        );
        assert_eq!(
            wait_secs(&headers(&[("x-ratelimit-reset", "900")]), now, 1),
            0
        );
        assert_eq!(
            wait_secs(&headers(&[("x-ratelimit-reset", "999999")]), now, 1),
            60
        );
        assert_eq!(
            wait_secs(
                &headers(&[("retry-after", "3"), ("x-ratelimit-reset", "1030")]),
                now,
                1
            ),
            3
        );
        assert_eq!(wait_secs(&headers(&[]), now, 2), 2);
    }

    #[test]
    fn a_secret_is_known_by_type_or_visibility() {
        let kind = |j: &str| serde_json::from_str::<RawEnv>(j).unwrap().into_var().kind;
        assert_eq!(
            kind(r#"{"id":"a","key":"K","type":"sensitive"}"#),
            Kind::Secret
        );
        assert_eq!(
            kind(r#"{"id":"a","key":"K","type":"encrypted","visibility":"secret"}"#),
            Kind::Secret
        );
        assert_eq!(
            kind(r#"{"id":"a","key":"K","type":"encrypted"}"#),
            Kind::Config
        );
        assert_eq!(kind(r#"{"id":"a","key":"K","type":"plain"}"#), Kind::Config);
    }

    #[test]
    fn a_target_may_be_a_string_a_list_or_absent() {
        let t = |j: &str| serde_json::from_str::<RawEnv>(j).unwrap().into_var().target;
        assert_eq!(
            t(r#"{"id":"a","key":"K","target":"production"}"#),
            ["production"]
        );
        assert_eq!(
            t(r#"{"id":"a","key":"K","target":["preview","development"]}"#),
            ["preview", "development"]
        );
        assert!(t(r#"{"id":"a","key":"K"}"#).is_empty());
    }

    #[test]
    fn the_api_url_must_be_https_or_loopback() {
        assert!(parse_base("https://api.vercel.com").is_ok());
        assert!(parse_base("http://127.0.0.1:1234").is_ok());
        assert!(parse_base("http://localhost:1234/x").is_ok());
        assert!(parse_base("http://example.com").is_err());
        assert!(parse_base("ftp://example.com").is_err());
        assert!(parse_base("https://user:pw@example.com").is_err());
        assert!(parse_base("not a url").is_err());
    }

    #[test]
    fn segments_are_encoded_not_interpreted() {
        let api = Api::new(
            parse_base("https://api.vercel.com").unwrap(),
            SecretString::new("t"),
            Some(Team::from_ref("team_1")),
        )
        .unwrap();
        let u = api.url(&["v10", "projects", "a/b", "env"], &[("until", "5")]);
        assert_eq!(
            u.as_str(),
            "https://api.vercel.com/v10/projects/a%2Fb/env?teamId=team_1&until=5"
        );
        assert_eq!(Team::from_ref("acme"), Team::Slug("acme".into()));
    }

    #[test]
    fn a_403_is_a_bad_token_only_when_vercel_says_so() {
        assert!(invalid_token(
            r#"{"error":{"code":"forbidden","invalidToken":true}}"#
        ));
        assert!(!invalid_token(
            r#"{"error":{"code":"forbidden","invalidToken":false}}"#
        ));
        assert!(!invalid_token(r#"{"error":{"code":"forbidden"}}"#));
        assert!(!invalid_token("not json"));
    }

    #[test]
    fn an_error_sentence_is_short_and_one_line() {
        let long = "x".repeat(500);
        let body = format!(r#"{{"error":{{"message":"{long}\n"}}}}"#);
        assert!(server_sentence(&body).unwrap().len() <= 200);
        assert_eq!(server_sentence("nope"), None);
    }
}
