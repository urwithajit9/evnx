//! HTTP client — the single place the CLI talks to evnx-server.
//!
//! Every cloud command goes through here, so bearer tokens, error mapping and
//! token refresh are decided once instead of in each command.
//!
//! # Errors are matched on `code`, never on status
//!
//! evnx-server answers failures with `{"error": message, "code": CODE}`. Two of
//! those codes share a status: `EMAIL_NOT_VERIFIED` and `FORBIDDEN` are both HTTP
//! 403. Branching on the status alone would tell a user who simply has not clicked
//! the verification link that they lack access to the vault, which sends them
//! looking in exactly the wrong place. [`ApiError`] is keyed on `code`.
//!
//! # Refresh happens at most once per request, and persists before retrying
//!
//! The server **revokes a refresh token the moment it is used** — rotation is
//! one-time-use. So the order is refresh → write to disk → retry. Renewing a
//! session and then failing to save it burns the only token that could have
//! renewed it again, silently signing the user out on the next command; that case
//! is [`ApiError::RefreshNotPersisted`] and it says so plainly rather than being
//! swallowed.
//!
//! # No automatic retries
//!
//! Other than the single refresh-and-retry above, a failed request stays failed.
//! The server assigns version numbers, so silently re-sending a `cloud push` after
//! an ambiguous timeout could create a duplicate version. Targeted retries can be
//! added later where they are shown to help.

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::time::Duration;

use super::creds::{SecretString, Store};

/// Supplies an `evnx_tok_` API token instead of a signed-in session.
///
/// For CI, where there is no credential store and no interactive login. When set,
/// it takes precedence over any stored session — a pipeline should use the
/// credential it was given, not one that happens to be lying around on a shared
/// runner.
pub const ENV_TOKEN: &str = "EVNX_TOKEN";

/// Time allowed to establish a connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Time allowed for a whole request. Blob transfer will want its own, larger
/// value; [`Client::with_timeout`] exists for that.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// `GET /health` response.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    /// `"ok"` when the server is serving.
    pub status: String,
    /// The server's own crate version.
    pub version: String,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    /// The server rotates this on every use; the old one is already revoked by
    /// the time we read this field.
    refresh_token: String,
}

/// Everything that can go wrong talking to evnx-server.
///
/// Messages are written for the person who typed the command, so each one says
/// what to do next where there is something to do.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No stored session for this server.
    #[error("not signed in to {server}. Run `evnx auth login` first.")]
    NotSignedIn {
        /// Server that has no session.
        server: String,
    },

    /// 401. The session is gone and refreshing did not recover it.
    #[error("your session has expired. Run `evnx auth login` to sign in again.")]
    Unauthorized,

    /// 401 to an `EVNX_TOKEN` API token (I-302, DEC-AU302).
    ///
    /// ⚠️ Not [`ApiError::Unauthorized`]. A token has no session to expire and
    /// no refresh, and the job that holds one usually cannot run `evnx auth
    /// login` — so "your session has expired" named the wrong cause and a fix
    /// it could not carry out.
    #[error(
        "the API token in EVNX_TOKEN was refused — it has been revoked, has expired, \
         or is mistyped. Create a new one on a signed-in machine with \
         `evnx auth token create`, and replace the secret where this job reads it."
    )]
    TokenRejected,

    /// 403 `EMAIL_NOT_VERIFIED`.
    #[error(
        "your email address is not verified yet — check your inbox for the link from evnx, \
             or run `evnx auth resend-verification` for a new one. \
             Vault commands stay unavailable until it is confirmed."
    )]
    EmailNotVerified,

    /// 403 `FORBIDDEN`.
    ///
    /// A newer server says why (RM-56); an older one sends neither field, and
    /// then the message is the server's sentence alone.
    #[error("{message}{}", forbidden_hint(reason.as_deref(), required_role.as_deref()))]
    Forbidden {
        /// The server's message.
        message: String,
        /// `ROLE_TOO_LOW`, `NOT_A_MEMBER`, `TOKEN_SCOPE` or `TOKEN_READ_ONLY`, when the server says.
        reason: Option<String>,
        /// With `ROLE_TOO_LOW`: the role the action needs.
        required_role: Option<String>,
    },

    /// `PLAN_LIMIT` (RM-58), whatever the status — 403, or 409 from a server
    /// that predates the code.
    #[error("{message}")]
    PlanLimit {
        /// The server's message.
        message: String,
    },

    /// 423 `LOCKED`.
    #[error("{message}")]
    Locked {
        /// The server's message.
        ///
        /// ⚠️ This said "which says how long the lock lasts". It does not: the
        /// server answers `AccountLocked` with the three words "Account
        /// temporarily locked" and no duration, for both the TOTP and the SRP
        /// lockout. A caller that wants to tell the user the lock will lift has
        /// to say so itself — see `auth::sign_in_error`.
        message: String,
    },

    /// 404.
    #[error("not found on {server}: {path}")]
    NotFound {
        /// Server queried.
        server: String,
        /// Path that returned 404.
        path: String,
    },

    /// 409. For a push this means someone else pushed first.
    #[error("{message}")]
    Conflict {
        /// The server's message.
        message: String,
    },

    /// 422.
    #[error("the server rejected the request: {message}")]
    Validation {
        /// The server's message.
        message: String,
    },

    /// 429. The message carries the retry delay.
    #[error("{message}")]
    RateLimited {
        /// The server's message, including how long to wait.
        message: String,
        /// Seconds from the `Retry-After` header, when it gave a plain number.
        retry_after: Option<u64>,
    },

    /// 5xx. The server deliberately does not say what broke, so neither do we.
    #[error("{server} reported an internal error (HTTP {status}). Nothing was changed.{}", retry_hint(*retry_after))]
    ServerError {
        /// Server that failed.
        server: String,
        /// Status returned.
        status: u16,
        /// Seconds from `Retry-After` (a 503 usually sends one).
        retry_after: Option<u64>,
    },

    /// A reply that is not evnx's: an HTML error page from a proxy, firewall or
    /// captive portal (DEC-AU5, I-203).
    #[error(
        "{server} answered HTTP {status} with a page that is not from evnx — a proxy, \
         firewall or captive portal is in the way"
    )]
    NotEvnx {
        /// Server that was addressed.
        server: String,
        /// Status returned.
        status: u16,
    },

    /// The request never completed — DNS, connection refused, TLS, timeout.
    ///
    /// Deliberately holds a flattened `reason` rather than the `reqwest::Error`
    /// as a `#[source]`. reqwest wraps hyper wraps the OS error, so keeping the
    /// chain makes a refused connection print as five nested lines, four of them
    /// noise, with the only actionable one last. See
    /// [`crate::core::http::transport_reason`].
    #[error("could not reach {server}: {reason}")]
    Transport {
        /// Server that could not be reached.
        server: String,
        /// Shortest useful description of what went wrong.
        reason: String,
    },

    /// A refusal with a code this build does not know, and a sentence that
    /// says what was wrong (I-306). The sentence is the message: the server
    /// writes it for the person reading, and a code added after this build was
    /// released is no reason to drop it.
    #[error("{message}")]
    Refused {
        /// Status returned.
        status: u16,
        /// The server's sentence.
        message: String,
    },

    /// A status or body this client does not know how to interpret.
    #[error("unexpected response from {server}: HTTP {status}")]
    Unexpected {
        /// Server that answered.
        server: String,
        /// Status returned.
        status: u16,
    },

    /// The response was not the JSON shape this command expected.
    #[error("could not read the server's response: {message}")]
    Malformed {
        /// What went wrong parsing.
        message: String,
    },

    /// The session was renewed but could not be written to disk.
    ///
    /// Its own variant because it is genuinely worse than a failed write: the
    /// server has already revoked the previous refresh token, so the session in
    /// memory is the only copy and it dies with the process.
    #[error(
        "your session was renewed but could not be saved: {message}\n\
             The previous token has already been revoked by the server, so you will \
             need to run `evnx auth login` again."
    )]
    RefreshNotPersisted {
        /// Why the write failed.
        message: String,
    },

    /// A local failure — reading the credential store, building the client.
    ///
    /// ⚠️ No `#[from]`/`#[source]`: the inner error is already rendered in full
    /// by `{0:#}`, and as a source it would be printed a second time by
    /// anyhow's `{:#}` chain.
    #[error("{0:#}")]
    Local(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Local(e)
    }
}

/// The server's sentence, plus the wait when it did not already say so.
fn with_retry(message: String, retry_after: Option<u64>) -> String {
    match retry_after {
        Some(n) if !message.contains(&n.to_string()) => {
            format!("{message} Try again in {n} s.")
        }
        _ => message,
    }
}

/// " Try again in N s." — or nothing.
fn retry_hint(retry_after: Option<u64>) -> String {
    match retry_after {
        Some(n) => format!(" Try again in {n} s."),
        None => String::new(),
    }
}

/// One line saying what to do about a 403, by the server's `reason`.
fn forbidden_hint(reason: Option<&str>, required_role: Option<&str>) -> String {
    match reason {
        Some("ROLE_TOO_LOW") => match required_role {
            Some(r) => format!("\n  This needs the {r} role; ask the vault owner."),
            None => "\n  This needs a higher role on the vault; ask the vault owner.".into(),
        },
        Some("NOT_A_MEMBER") => {
            "\n  You are not a member of this vault; ask the owner to share it \
             with you using `evnx vault share`."
                .into()
        }
        Some("TOKEN_SCOPE") => "\n  This API token is restricted to another vault; create one for \
             this vault with `evnx auth token create --vault <vault>`."
            .into(),
        Some("TOKEN_READ_ONLY") => {
            "\n  This API token can only read; create one that can write with \
             `evnx auth token create --scope read_write`."
                .into()
        }
        _ => String::new(),
    }
}

/// An `ApiError` whose text was rewritten for the command that hit it.
///
/// Several commands explain a refusal in their own terms ("version 3 does not
/// exist" rather than "not found: /api/v1/…"). Flattening that into a plain
/// `anyhow!` message threw the type away, so `main` could not tell a 401 from
/// a 500. This keeps the rewritten text and the exit code, and — unlike
/// wrapping the `ApiError` as a context source — does not print the original
/// message a second time.
#[derive(Debug)]
pub struct Reworded {
    message: String,
    exit: Option<i32>,
}

impl std::fmt::Display for Reworded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Reworded {}

/// Rewrite an `ApiError`'s message, keeping its exit code.
///
/// `exit` is [`ApiError::exit_code`] of the error being replaced, taken before
/// it is matched away.
pub fn reword(exit: Option<i32>, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Reworded {
        message: message.into(),
        exit,
    })
}

/// The exit code a cloud failure asks for (X3), found anywhere in the chain.
pub fn exit_code_of(err: &anyhow::Error) -> Option<i32> {
    err.chain().find_map(|c| {
        if let Some(a) = c.downcast_ref::<ApiError>() {
            a.exit_code()
        } else if c.is::<crate::core::http::TlsRootsError>() {
            // SSL_CERT_FILE / SSL_CERT_DIR unusable: the server cannot be
            // reached the way the user asked, the same class as a transport failure.
            Some(13)
        } else {
            c.downcast_ref::<Reworded>().and_then(|r| r.exit)
        }
    })
}

impl ApiError {
    /// Whether re-running the command unchanged could plausibly succeed.
    ///
    /// Used by callers deciding whether to suggest "try again" — not by this
    /// module, which never retries on its own.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ApiError::Transport { .. }
                | ApiError::ServerError { .. }
                | ApiError::RateLimited { .. }
        )
    }

    /// The process exit code this failure asks for (decision X3), or `None` to
    /// keep the command's usual one.
    ///
    /// 0–5 belong to other commands, and `run` returns the child's own code once
    /// it has started — these apply only before that.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            ApiError::NotSignedIn { .. }
            | ApiError::Unauthorized
            | ApiError::TokenRejected
            | ApiError::RefreshNotPersisted { .. } => Some(10),
            ApiError::Forbidden { .. } | ApiError::EmailNotVerified => Some(11),
            ApiError::PlanLimit { .. } => Some(12),
            ApiError::Transport { .. } | ApiError::NotEvnx { .. } => Some(13),
            ApiError::Conflict { .. } | ApiError::Locked { .. } => Some(14),
            ApiError::ServerError { .. } | ApiError::RateLimited { .. } => Some(15),
            ApiError::NotFound { .. }
            | ApiError::Validation { .. }
            | ApiError::Refused { .. }
            | ApiError::Unexpected { .. }
            | ApiError::Malformed { .. } => None,
            // An unusable SSL_CERT_FILE / SSL_CERT_DIR (RM-68) is the same class
            // as a server that cannot be reached; any other local failure keeps
            // the command's usual code.
            ApiError::Local(e) => e
                .chain()
                .any(|c| c.is::<crate::core::http::TlsRootsError>())
                .then_some(13),
        }
    }
}

/// Error body as evnx-server sends it.
#[derive(Deserialize)]
struct ServerError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    code: String,
    /// Why a 403 `FORBIDDEN` (RM-56). Absent from older servers.
    #[serde(default)]
    reason: Option<String>,
    /// With `ROLE_TOO_LOW`: the role needed.
    #[serde(default)]
    required_role: Option<String>,
}

/// `Retry-After` as seconds. An HTTP-date is treated as unknown.
fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn is_json_content_type(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("json")
        })
        .unwrap_or(false)
}

/// How a client proves who it is.
enum Credential {
    /// A signed-in session from the credential store: short access token, rotated
    /// refresh token, renewed automatically.
    Session,
    /// An `evnx_tok_` API token from [`ENV_TOKEN`]. Long-lived, never refreshed —
    /// it is revoked or it expires.
    ApiToken(SecretString),
}

/// A client bound to one server.
pub struct Client {
    /// Canonical server URL — also the credential-store key.
    server: String,
    http: reqwest::blocking::Client,
    /// Loaded once so the permission check and its warning happen once per
    /// command rather than once per request.
    store: RefCell<Store>,
    credential: Credential,
}

impl Client {
    /// Build a client for a canonical server URL.
    ///
    /// The URL must already have been through
    /// [`super::config::canonical_server`], which is what rejects plain `http://`
    /// to a non-loopback host.
    pub fn new(server: impl Into<String>) -> Result<Self, ApiError> {
        Self::with_timeout(server, REQUEST_TIMEOUT)
    }

    /// [`Client::new`] with a different overall request timeout, for transfers
    /// that legitimately take longer than a control-plane call.
    pub fn with_timeout(server: impl Into<String>, timeout: Duration) -> Result<Self, ApiError> {
        let server = server.into();
        let http = crate::core::http::client(timeout, Some(CONNECT_TIMEOUT))
            .context("building the HTTP client")?;
        let store = Store::load().context("reading the credential store")?;

        // An explicitly supplied token wins over a stored session. On a shared CI
        // runner the stored session may belong to someone else entirely.
        let credential = match std::env::var(ENV_TOKEN) {
            Ok(t) if !t.trim().is_empty() => Credential::ApiToken(SecretString::new(t.trim())),
            _ => Credential::Session,
        };

        Ok(Client {
            server,
            http,
            store: RefCell::new(store),
            credential,
        })
    }

    /// The server this client talks to.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// Whether this client has a usable credential — a stored session, or an
    /// API token from the environment.
    pub fn is_signed_in(&self) -> bool {
        matches!(self.credential, Credential::ApiToken(_))
            || self.store.borrow().session(&self.server).is_some()
    }

    /// Whether the credential is an API token rather than a login.
    ///
    /// Commands that manage the account refuse these: the server answers 403 for
    /// a token on `/auth/tokens` or `/auth/totp/*`, deliberately, so a leaked CI
    /// token cannot mint a replacement or enrol its own authenticator. Saying so
    /// before the request makes the reason obvious.
    pub fn is_api_token(&self) -> bool {
        matches!(self.credential, Credential::ApiToken(_))
    }

    /// Refuse unless signed in. An API token is accepted (A-37, DEC-AU3): this
    /// is for what CI does — push, pull, run, list, audit.
    pub fn require_credential(&self) -> anyhow::Result<()> {
        if self.is_signed_in() {
            return Ok(());
        }
        // The typed error, so `main` can exit 10 rather than the generic code.
        Err(ApiError::NotSignedIn {
            server: self.server.clone(),
        }
        .into())
    }

    /// Refuse an API token, then refuse when not signed in (A-37, DEC-AU3).
    ///
    /// For account management and for what changes who holds a vault. `what`
    /// completes "an API token cannot …"; `why` is zero or more lines, each
    /// starting `\x20 ` and ending `\n`, saying what a leaked token could do.
    /// The server refuses these too (403); saying so before the request makes the
    /// reason plain and sends nothing.
    ///
    /// ⚠️ Five commands had their own copy of this and a sixth guard accepted
    /// tokens, so `rotate-master-password`, `download-data` and
    /// `delete-account` went to the server with `EVNX_TOKEN` set (I-211).
    pub fn require_user_session(&self, what: &str, why: &str) -> anyhow::Result<()> {
        if self.is_api_token() {
            anyhow::bail!(
                "an API token cannot {what} — that is deliberate.\n{why}\x20 Unset {ENV_TOKEN} and run `evnx auth login`."
            );
        }
        self.require_credential()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.server, path)
    }

    /// `GET /health`. Unauthenticated, so it works before signing in.
    pub fn health(&self) -> Result<Health, ApiError> {
        let resp = self
            .http
            .get(self.url("/health"))
            .send()
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
            })?;
        self.decode(resp, "/health")
    }

    /// Unauthenticated POST — register, SRP, refresh, email verification.
    pub fn post_public<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let resp = self
            .http
            .post(self.url(path))
            .json(body)
            .send()
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
            })?;
        self.decode(resp, path)
    }

    /// Authenticated GET.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.send_authed(path, |http, url| http.get(url))
    }

    /// Authenticated GET returning raw bytes.
    ///
    /// `download_blob` answers `application/octet-stream`, not JSON — the body is
    /// `nonce(12) || ciphertext`. Going through [`Client::get`] would try to parse
    /// ciphertext as JSON and fail on the first non-UTF-8 byte.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        self.get_bytes_with_header(path, "X-Blob-Hash")
            .map(|(bytes, _)| bytes)
    }

    /// [`Client::get_bytes`], and the value of one response header if the
    /// server sent it: `download_blob` sends the version's `X-Blob-Hash`.
    pub fn get_bytes_with_header(
        &self,
        path: &str,
        header: &str,
    ) -> Result<(Vec<u8>, Option<String>), ApiError> {
        let token = self.access_token()?;
        let send = |token: &SecretString| {
            self.http
                .get(self.url(path))
                .bearer_auth(token.expose())
                .send()
                .map_err(|e| ApiError::Transport {
                    server: self.server.clone(),
                    reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
                })
        };

        let mut resp = send(&token)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            drop(resp);
            self.refresh()?;
            resp = send(&self.access_token()?)?;
        }

        if !resp.status().is_success() {
            // Reuse the JSON error mapping: a failure is an error document even
            // on a binary endpoint.
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.text().unwrap_or_default();
            return Err(self.error_from(status, &headers, &body, path));
        }

        let value = resp
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        resp.bytes()
            .map(|b| (b.to_vec(), value))
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
            })
    }

    /// Authenticated POST with a JSON body.
    pub fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let json = serde_json::to_vec(body).map_err(|e| ApiError::Malformed {
            message: format!("could not serialize the request body: {e}"),
        })?;
        self.send_authed(path, move |http, url| {
            http.post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(json.clone())
        })
    }

    /// Authenticated PUT with a JSON body.
    pub fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let json = serde_json::to_vec(body).map_err(|e| ApiError::Malformed {
            message: format!("could not serialize the request body: {e}"),
        })?;
        self.send_authed(path, move |http, url| {
            http.put(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(json.clone())
        })
    }

    /// Authenticated PATCH with a JSON body.
    pub fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let json = serde_json::to_vec(body).map_err(|e| ApiError::Malformed {
            message: format!("could not serialize the request body: {e}"),
        })?;
        self.send_authed(path, move |http, url| {
            http.patch(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(json.clone())
        })
    }

    /// Authenticated DELETE. Discards the body.
    pub fn delete(&self, path: &str) -> Result<(), ApiError> {
        let _: serde::de::IgnoredAny = self.send_authed(path, |http, url| http.delete(url))?;
        Ok(())
    }

    /// Authenticated DELETE carrying a JSON body.
    ///
    /// ⚠️ A body on DELETE is legal but unusual, and some intermediaries drop it.
    /// That is tolerable here only because of which way it fails: the one endpoint
    /// using it requires a typed confirmation in that body, so a stripped body
    /// means the server **refuses** rather than deletes. A silent success would
    /// not be acceptable; a refusal is.
    pub fn delete_with_body<B: Serialize>(&self, path: &str, body: &B) -> Result<(), ApiError> {
        let json = serde_json::to_vec(body).map_err(|e| ApiError::Malformed {
            message: format!("could not serialize the request body: {e}"),
        })?;
        let _: serde::de::IgnoredAny = self.send_authed(path, move |http, url| {
            http.delete(url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(json.clone())
        })?;
        Ok(())
    }

    /// Send with a bearer token, refreshing once if the server says 401 — or
    /// says the email is not verified (I-301).
    ///
    /// Takes a builder closure rather than a `RequestBuilder` because the request
    /// may need to be constructed twice and `RequestBuilder` is not `Clone`.
    fn send_authed<T: DeserializeOwned>(
        &self,
        path: &str,
        build: impl Fn(&reqwest::blocking::Client, String) -> reqwest::blocking::RequestBuilder,
    ) -> Result<T, ApiError> {
        let send = || -> Result<reqwest::blocking::Response, ApiError> {
            let token = self.access_token()?;
            build(&self.http, self.url(path))
                .bearer_auth(token.expose())
                .send()
                .map_err(|e| ApiError::Transport {
                    server: self.server.clone(),
                    reason: crate::core::http::transport_reason(&e, REQUEST_TIMEOUT),
                })
        };

        let resp = send()?;
        if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
            return match self.decode(resp, path) {
                // ⚠️ I-301, DEC-AU301. An access token carries `email_verified`
                // from when it was issued, so one issued before the link was
                // opened says "unverified" for up to 15 minutes — while
                // `auth status`, which asks the server, says "verified yes".
                // A refresh reissues it from the database. Once, and only for
                // a session: an API token has no refresh path.
                //
                // Retrying is safe for every method, POST included: the only
                // place the server answers EMAIL_NOT_VERIFIED is the
                // `require_verified` guard, before any handler runs.
                //
                // A refresh that fails is not the answer to "is my email
                // verified" — the original refusal is.
                Err(ApiError::EmailNotVerified) if !self.is_api_token() => {
                    if self.refresh().is_err() {
                        return Err(ApiError::EmailNotVerified);
                    }
                    self.decode(send()?, path)
                }
                other => other,
            };
        }

        // Exactly one attempt. If the refresh token is also dead, `refresh`
        // returns Unauthorized and we stop rather than looping.
        drop(resp);
        self.refresh()?;
        self.decode(send()?, path)
    }

    /// The current access token, refreshing first if it is expired or near it.
    fn access_token(&self) -> Result<SecretString, ApiError> {
        // An API token is sent as-is. It has no expiry the client can see and no
        // refresh path — it is revoked, or it lapses server-side.
        if let Credential::ApiToken(t) = &self.credential {
            return Ok(t.clone());
        }

        let now = chrono::Utc::now().timestamp();
        {
            let store = self.store.borrow();
            let session = store
                .session(&self.server)
                .ok_or_else(|| ApiError::NotSignedIn {
                    server: self.server.clone(),
                })?;
            if session.access_token_is_fresh(now) {
                return Ok(session.access_token.clone());
            }
        }
        self.refresh()?;
        let store = self.store.borrow();
        let session = store
            .session(&self.server)
            .ok_or_else(|| ApiError::NotSignedIn {
                server: self.server.clone(),
            })?;
        Ok(session.access_token.clone())
    }

    /// Exchange the refresh token for a new pair and persist it.
    ///
    /// Writing to disk is part of the operation, not a follow-up: the server has
    /// revoked the old refresh token by the time this returns, so an unsaved
    /// renewal is a lost session.
    fn refresh(&self) -> Result<(), ApiError> {
        if self.is_api_token() {
            // A 401 on an API token means revoked, expired, or wrong — none of
            // which a refresh could fix. Retrying would just repeat it.
            return Err(ApiError::TokenRejected);
        }

        let refresh_token = {
            let store = self.store.borrow();
            store
                .session(&self.server)
                .ok_or_else(|| ApiError::NotSignedIn {
                    server: self.server.clone(),
                })?
                .refresh_token
                .clone()
        };

        let fresh: RefreshResponse = self.post_public(
            "/api/v1/auth/refresh",
            &RefreshRequest {
                refresh_token: refresh_token.expose(),
            },
        )?;

        let expires_at = jwt_exp(&fresh.access_token).unwrap_or_else(|| {
            // A token we cannot read the expiry from still works; we just do not
            // know when to renew it. Treating it as immediately stale means one
            // extra refresh, never a failed request.
            chrono::Utc::now().timestamp()
        });

        {
            let mut store = self.store.borrow_mut();
            let session =
                store
                    .sessions
                    .get_mut(&self.server)
                    .ok_or_else(|| ApiError::NotSignedIn {
                        server: self.server.clone(),
                    })?;
            session.access_token = SecretString::new(fresh.access_token);
            session.refresh_token = SecretString::new(fresh.refresh_token);
            session.access_expires_at = expires_at;
        }

        self.store
            .borrow()
            .save()
            .map_err(|e| ApiError::RefreshNotPersisted {
                message: format!("{e:#}"),
            })
    }

    /// Turn a response into either the expected type or a typed error.
    fn decode<T: DeserializeOwned>(
        &self,
        resp: reqwest::blocking::Response,
        path: &str,
    ) -> Result<T, ApiError> {
        let status = resp.status();
        if status.is_success() {
            // 204 and other empty bodies still have to satisfy T; `null` is what
            // serde accepts for `()` and `IgnoredAny`.
            let text = resp.text().unwrap_or_default();
            let text = if text.trim().is_empty() {
                "null"
            } else {
                &text
            };
            return serde_json::from_str(text).map_err(|e| ApiError::Malformed {
                message: format!("{e} (from {path})"),
            });
        }

        let headers = resp.headers().clone();
        let body = resp.text().unwrap_or_default();
        Err(self.error_from(status, &headers, &body, path))
    }

    /// Map a failure status, its headers and its body to a typed error.
    fn error_from(
        &self,
        status: reqwest::StatusCode,
        headers: &reqwest::header::HeaderMap,
        body: &str,
        path: &str,
    ) -> ApiError {
        let json: Option<ServerError> = serde_json::from_str(body).ok();
        // Not evnx's: the body is not the `{error, code}` document and the
        // content type does not claim JSON either (DEC-AU5, I-203). A blank
        // body proves nothing, so it is left to the status below.
        let not_evnx = json.is_none() && !body.trim().is_empty() && !is_json_content_type(headers);
        let parsed = json.unwrap_or(ServerError {
            error: String::new(),
            code: String::new(),
            reason: None,
            required_role: None,
        });
        let retry_after = retry_after_secs(headers);
        // A real sentence from the server, as opposed to the placeholder below.
        let has_sentence = !parsed.error.trim().is_empty();
        let message = if parsed.error.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            parsed.error
        };

        // `code` first: EMAIL_NOT_VERIFIED and FORBIDDEN share status 403, so the
        // status alone cannot tell them apart.
        // RM-58: a plan limit may arrive as `reason: PLAN_LIMIT` under another
        // code (the RM-56 envelope) as well as `code: PLAN_LIMIT`.
        if parsed.reason.as_deref() == Some("PLAN_LIMIT") {
            return ApiError::PlanLimit { message };
        }
        match parsed.code.as_str() {
            "UNAUTHORIZED" => ApiError::Unauthorized,
            "EMAIL_NOT_VERIFIED" => ApiError::EmailNotVerified,
            "FORBIDDEN" => ApiError::Forbidden {
                message,
                reason: parsed.reason,
                required_role: parsed.required_role,
            },
            "PLAN_LIMIT" => ApiError::PlanLimit { message },
            "LOCKED" => ApiError::Locked { message },
            "NOT_FOUND" => ApiError::NotFound {
                server: self.server.clone(),
                path: path.to_string(),
            },
            "CONFLICT" => ApiError::Conflict { message },
            "VALIDATION_ERROR" => ApiError::Validation { message },
            "RATE_LIMITED" => ApiError::RateLimited {
                message: with_retry(message, retry_after),
                retry_after,
            },
            "INTERNAL_ERROR" => ApiError::ServerError {
                server: self.server.clone(),
                status: status.as_u16(),
                retry_after,
            },
            // No recognised code: fall back to the status, so a proxy's error
            // page or a future code still produces something sensible.
            _ => match status.as_u16() {
                401 => ApiError::Unauthorized,
                404 => ApiError::NotFound {
                    server: self.server.clone(),
                    path: path.to_string(),
                },
                s if not_evnx => ApiError::NotEvnx {
                    server: self.server.clone(),
                    status: s,
                },
                429 => ApiError::RateLimited {
                    message: with_retry(message, retry_after),
                    retry_after,
                },
                s if (500..600).contains(&s) => ApiError::ServerError {
                    server: self.server.clone(),
                    status: s,
                    retry_after,
                },
                // ⚠️ I-306: a code this build does not know, with a sentence —
                // keep the sentence. It said "unexpected response: HTTP 403"
                // and dropped it, which is what made a new code impossible to
                // add without breaking every installed CLI.
                s if has_sentence => ApiError::Refused { status: s, message },
                s => ApiError::Unexpected {
                    server: self.server.clone(),
                    status: s,
                },
            },
        }
    }
}

/// An `/api/v1/…` path built from `segments`, each one percent-encoded so it
/// stays one segment (A-43, DEC-OR2).
///
/// ⚠️ User text was interpolated into paths, so `org uninvite
/// "../members/u-bob"` sent `DELETE /orgs/o/members/u-bob`, and `a b?x=1#f`
/// became a query and a fragment (I-217). Everything outside the URL standard's
/// unreserved set (`A-Z a-z 0-9 - . _ ~`) is encoded — `/`, `?`, `#`, `%`, `@`,
/// `+`, spaces, non-ASCII.
///
/// ⚠️ `.` and `..` are **refused**, not encoded: `reqwest::Url` follows the URL
/// standard, which collapses `..`, `%2E%2E` and `.%2e` alike before sending
/// (measured: all three turned `invites/../members/u` into `members/u`). An
/// empty segment is refused too: it would address the collection instead of
/// the item.
pub(crate) fn api_path(segments: &[&str]) -> anyhow::Result<String> {
    let mut path = String::from("/api/v1");
    for seg in segments {
        if seg.is_empty() || *seg == "." || *seg == ".." {
            anyhow::bail!(
                "{seg:?} cannot be used in a request path: it would address a \
                 different resource than the one named. Nothing was sent."
            );
        }
        path.push('/');
        for b in seg.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                path.push(b as char);
            } else {
                path.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Ok(path)
}

/// Read the `exp` claim from a JWT **without verifying its signature**.
///
/// The CLI does not hold the signing key and cannot verify anything. This value
/// decides only *when to refresh pre-emptively*; it never grants access, and the
/// server validates every token on every request. The worst a bogus value can do
/// is cause one unnecessary refresh, or a 401 that the refresh-and-retry path
/// already handles.
///
/// Do not repurpose this for anything that makes a security decision.
pub(crate) fn jwt_exp(token: &str) -> Option<i64> {
    use base64::Engine;

    let payload = token.split('.').nth(1)?;
    // JWT uses base64url with the padding stripped.
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp")?.as_i64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::creds::Session;
    use crate::cloud::testutil::ConfigDirGuard;
    use serial_test::serial;

    fn signed_in(server: &str, expires_at: i64) -> Store {
        let mut store = Store::default();
        store.set_session(
            server,
            Session {
                email: "user@example.com".into(),
                access_token: SecretString::new("access-1"),
                access_expires_at: expires_at,
                refresh_token: SecretString::new("refresh-1"),
            },
        );
        store
    }

    /// A JWT-shaped string whose payload carries `exp`. Header and signature are
    /// filler — nothing here verifies them, which is the point.
    fn jwt_with_exp(exp: i64) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"exp":{exp},"scope":"user"}}"#));
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.not-a-real-signature")
    }

    #[test]
    fn jwt_exp_reads_the_claim() {
        assert_eq!(jwt_exp(&jwt_with_exp(1_800_000_000)), Some(1_800_000_000));
    }

    #[test]
    fn jwt_exp_returns_none_rather_than_panicking_on_junk() {
        for junk in ["", "a", "a.b", "a.b.c", "....", "a.!!!!.c"] {
            assert_eq!(jwt_exp(junk), None, "{junk:?}");
        }
    }

    #[test]
    #[serial]
    fn health_reports_the_server_version() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        let m = server
            .mock("GET", "/health")
            .with_status(200)
            .with_body(r#"{"status":"ok","version":"0.1.0"}"#)
            .create();

        let client = Client::new(server.url()).unwrap();
        let health = client.health().unwrap();
        assert_eq!(health.status, "ok");
        assert_eq!(health.version, "0.1.0");
        m.assert();
    }

    #[test]
    #[serial]
    fn a_request_without_a_session_says_to_sign_in() {
        let _g = ConfigDirGuard::new();
        let server = mockito::Server::new();
        let client = Client::new(server.url()).unwrap();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        assert!(matches!(err, ApiError::NotSignedIn { .. }), "{err:?}");
        assert!(err.to_string().contains("evnx auth login"));
    }

    #[test]
    #[serial]
    fn the_bearer_token_and_user_agent_are_sent() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();

        let m = server
            .mock("GET", "/api/v1/vaults")
            .match_header("authorization", "Bearer access-1")
            .match_header("user-agent", concat!("evnx/", env!("CARGO_PKG_VERSION")))
            .with_status(200)
            .with_body("[]")
            .create();

        let client = Client::new(server.url()).unwrap();
        let _: Vec<serde_json::Value> = client.get("/api/v1/vaults").unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn email_not_verified_is_distinguished_from_forbidden() {
        // Both are HTTP 403. Telling them apart is the reason ApiError keys on
        // `code`: one means "click the link in your inbox", the other means
        // "you do not have access to this vault".
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let m1 = server
            .mock("GET", "/api/v1/vaults")
            .with_status(403)
            .with_body(r#"{"error":"Please verify your email first","code":"EMAIL_NOT_VERIFIED"}"#)
            .create();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        assert!(matches!(err, ApiError::EmailNotVerified), "{err:?}");
        assert!(err.to_string().contains("inbox"));
        m1.assert();

        let m2 = server
            .mock("GET", "/api/v1/vaults/x")
            .with_status(403)
            .with_body(r#"{"error":"Insufficient permissions","code":"FORBIDDEN"}"#)
            .create();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults/x")
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden { .. }), "{err:?}");
        assert!(!err.to_string().contains("inbox"));
        m2.assert();
    }

    // ─── I-301: a session issued before the email was verified ──────────────

    const NOT_VERIFIED: &str =
        r#"{"error":"Please verify your email first","code":"EMAIL_NOT_VERIFIED"}"#;

    /// The access token carries `email_verified` from when it was issued, so
    /// one issued before the link was opened says "unverified" for up to 15
    /// minutes. A refresh reissues it from the database.
    #[test]
    #[serial]
    fn t_i301_a_session_issued_before_verifying_is_refreshed_once_and_retried() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let stale = server
            .mock("GET", "/api/v1/vaults")
            .match_header("authorization", "Bearer access-1")
            .with_status(403)
            .with_body(NOT_VERIFIED)
            .expect(1)
            .create();
        // A readable `exp`, or the client treats the new token as stale at
        // once and refreshes again before the retry.
        let reissued = jwt_with_exp(4_000_000_000);
        let refresh = server
            .mock("POST", "/api/v1/auth/refresh")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"refresh_token":"refresh-1"}"#.into(),
            ))
            .with_status(200)
            .with_body(format!(
                r#"{{"access_token":"{reissued}","refresh_token":"refresh-2"}}"#
            ))
            .expect(1)
            .create();
        let fresh = server
            .mock("GET", "/api/v1/vaults")
            .match_header("authorization", format!("Bearer {reissued}").as_str())
            .with_status(200)
            .with_body("[]")
            .expect(1)
            .create();

        let _: Vec<serde_json::Value> = client.get("/api/v1/vaults").unwrap();
        stale.assert();
        refresh.assert();
        fresh.assert();
        // The rotated refresh token is the one kept: the old one is revoked.
        let saved = Store::load().unwrap();
        assert_eq!(
            saved.session(&server.url()).unwrap().refresh_token.expose(),
            "refresh-2"
        );
    }

    /// Genuinely unverified: one refresh, one retry, then today's message.
    #[test]
    #[serial]
    fn t_i301_still_unverified_after_the_refresh_is_reported_after_one_retry() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let vaults = server
            .mock("GET", "/api/v1/vaults")
            .with_status(403)
            .with_body(NOT_VERIFIED)
            .expect(2)
            .create();
        let refresh = server
            .mock("POST", "/api/v1/auth/refresh")
            .with_status(200)
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2"}}"#,
                jwt_with_exp(4_000_000_000)
            ))
            .expect(1)
            .create();

        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        assert!(matches!(err, ApiError::EmailNotVerified), "{err:?}");
        vaults.assert();
        refresh.assert();
    }

    /// A refresh that fails is not the answer to "is my email verified":
    /// the original refusal is.
    #[test]
    #[serial]
    fn t_i301_a_failed_refresh_still_reports_not_verified() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let vaults = server
            .mock("GET", "/api/v1/vaults")
            .with_status(403)
            .with_body(NOT_VERIFIED)
            .expect(1)
            .create();
        let refresh = server
            .mock("POST", "/api/v1/auth/refresh")
            .with_status(401)
            .with_body(r#"{"error":"Authentication failed","code":"UNAUTHORIZED"}"#)
            .expect(1)
            .create();

        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        assert!(matches!(err, ApiError::EmailNotVerified), "{err:?}");
        vaults.assert();
        refresh.assert();
    }

    /// RM-58: `reason: PLAN_LIMIT` under `code: FORBIDDEN` is a plan limit too.
    #[test]
    #[serial]
    fn rm58_a_plan_limit_reason_under_forbidden_is_a_plan_limit() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();
        let _m = server
            .mock("GET", "/api/v1/pl")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"error":"your plan allows 3 vaults.","code":"FORBIDDEN","reason":"PLAN_LIMIT"}"#,
            )
            .create();
        let e = client.get::<serde_json::Value>("/api/v1/pl").unwrap_err();
        assert!(matches!(e, ApiError::PlanLimit { .. }), "{e:?}");
        assert_eq!(e.exit_code(), Some(12));
    }

    // ─── I-306: a code this build does not know ─────────────────────────────

    /// A code added to the server after this build (`PLAN_LIMIT` was the first
    /// one considered) fell back to the status, and 403 printed "unexpected
    /// response from …: HTTP 403" — the sentence explaining the refusal lost.
    #[test]
    #[serial]
    fn t_i306_an_unknown_code_keeps_the_servers_sentence() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        for (status, path, body, said) in [
            (
                403,
                "/api/v1/a",
                r#"{"error":"your plan (free) allows 3 vaults.","code":"PLAN_LIMIT"}"#,
                "your plan (free) allows 3 vaults.",
            ),
            (
                418,
                "/api/v1/b",
                r#"{"error":"a sentence from a newer server","code":"SOMETHING_NEW"}"#,
                "a sentence from a newer server",
            ),
        ] {
            let m = server
                .mock("GET", path)
                .with_status(status)
                .with_body(body)
                .create();
            let err = client.get::<serde_json::Value>(path).unwrap_err();
            assert_eq!(err.to_string(), said, "{err:?}");
            m.assert();
        }
    }

    /// No sentence to keep — a proxy's error page — still names the status.
    #[test]
    #[serial]
    fn t_i306_a_body_without_a_sentence_still_names_the_status() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let m = server
            .mock("GET", "/api/v1/vaults")
            .with_status(403)
            .with_header("content-type", "text/html")
            .with_body("<html><body>Forbidden by proxy</body></html>")
            .create();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        assert!(
            matches!(err, ApiError::NotEvnx { status: 403, .. }),
            "{err:?}"
        );
        m.assert();
    }

    #[test]
    #[serial]
    fn every_server_error_code_maps_to_its_own_variant() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        // (status, machine code, what the mapped error should be)
        type Case = (u16, &'static str, fn(&ApiError) -> bool);
        let cases: &[Case] = &[
            (423, "LOCKED", |e| matches!(e, ApiError::Locked { .. })),
            (404, "NOT_FOUND", |e| matches!(e, ApiError::NotFound { .. })),
            (409, "CONFLICT", |e| matches!(e, ApiError::Conflict { .. })),
            (422, "VALIDATION_ERROR", |e| {
                matches!(e, ApiError::Validation { .. })
            }),
            (429, "RATE_LIMITED", |e| {
                matches!(e, ApiError::RateLimited { .. })
            }),
            (500, "INTERNAL_ERROR", |e| {
                matches!(e, ApiError::ServerError { .. })
            }),
        ];

        for (status, code, is_expected) in cases {
            let path = format!("/api/v1/probe/{code}");
            let m = server
                .mock("GET", path.as_str())
                .with_status(*status as usize)
                .with_body(format!(r#"{{"error":"msg for {code}","code":"{code}"}}"#))
                .create();
            let err = client.get::<serde_json::Value>(&path).unwrap_err();
            assert!(is_expected(&err), "{code} produced {err:?}");
            m.assert();
        }
    }

    #[test]
    #[serial]
    fn an_unrecognised_code_falls_back_to_the_status() {
        // A proxy or load balancer can answer with HTML the server never wrote.
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();

        let m = server
            .mock("GET", "/api/v1/vaults")
            .with_status(502)
            .with_body("<html>Bad Gateway</html>")
            .create();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();
        // A page that is not evnx's is not "the server reported an error".
        assert!(
            matches!(err, ApiError::NotEvnx { status: 502, .. }),
            "{err:?}"
        );
        m.assert();
    }

    /// One GET against a mock that answers `status` with `body`.
    fn failure(status: usize, headers: &[(&str, &str)], body: &str) -> ApiError {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let client = Client::new(server.url()).unwrap();
        let mut m = server.mock("GET", "/api/v1/probe").with_status(status);
        for (k, v) in headers {
            m = m.with_header(*k, v);
        }
        let m = m.with_body(body).create();
        let err = client
            .get::<serde_json::Value>("/api/v1/probe")
            .unwrap_err();
        m.assert();
        err
    }

    #[test]
    #[serial]
    fn rm56_forbidden_reasons_add_one_hint_each() {
        let e = failure(
            403,
            &[],
            r#"{"error":"Forbidden","code":"FORBIDDEN","reason":"ROLE_TOO_LOW","required_role":"admin"}"#,
        );
        assert!(matches!(e, ApiError::Forbidden { .. }), "{e:?}");
        let t = e.to_string();
        assert!(t.starts_with("Forbidden"), "{t}");
        assert!(
            t.contains("needs the admin role; ask the vault owner"),
            "{t}"
        );

        let e = failure(
            403,
            &[],
            r#"{"error":"Forbidden","code":"FORBIDDEN","reason":"NOT_A_MEMBER"}"#,
        );
        assert!(e.to_string().contains("`evnx vault share`"), "{e}");

        let e = failure(
            403,
            &[],
            r#"{"error":"Forbidden","code":"FORBIDDEN","reason":"TOKEN_SCOPE"}"#,
        );
        assert!(
            e.to_string().contains("evnx auth token create --vault"),
            "{e}"
        );
    }

    /// RM-56: evnx-server V0 also sends `TOKEN_READ_ONLY` for a read-only
    /// token that tried to write.
    #[test]
    #[serial]
    fn rm56_a_read_only_token_is_told_how_to_get_a_writing_one() {
        let e = failure(
            403,
            &[],
            r#"{"error":"Insufficient permissions","code":"FORBIDDEN","reason":"TOKEN_READ_ONLY"}"#,
        );
        assert!(e.to_string().contains("--scope read_write"), "{e}");
        assert_eq!(e.exit_code(), Some(11));
    }

    #[test]
    #[serial]
    fn rm56_forbidden_without_a_reason_is_todays_message() {
        let e = failure(403, &[], r#"{"error":"nope","code":"FORBIDDEN"}"#);
        assert_eq!(e.to_string(), "nope");
        // A reason this build does not know adds nothing either.
        let e = failure(
            403,
            &[],
            r#"{"error":"nope","code":"FORBIDDEN","reason":"SOMETHING_NEW"}"#,
        );
        assert_eq!(e.to_string(), "nope");
    }

    #[test]
    #[serial]
    fn rm58_plan_limit_on_403_and_on_legacy_409() {
        for status in [403, 409] {
            let e = failure(
                status,
                &[],
                r#"{"error":"your plan allows 3 vaults.","code":"PLAN_LIMIT"}"#,
            );
            assert!(matches!(e, ApiError::PlanLimit { .. }), "{status}: {e:?}");
            assert_eq!(e.exit_code(), Some(12));
        }
        // No code: a legacy 409 stays a conflict.
        let e = failure(409, &[], r#"{"error":"x","code":"CONFLICT"}"#);
        assert!(matches!(e, ApiError::Conflict { .. }), "{e:?}");
    }

    #[test]
    #[serial]
    fn i203_a_json_500_is_a_server_error_and_a_proxy_page_is_not_evnx() {
        let e = failure(500, &[], r#"{"error":"boom","code":"INTERNAL_ERROR"}"#);
        assert!(
            matches!(e, ApiError::ServerError { status: 500, .. }),
            "{e:?}"
        );

        let e = failure(502, &[("content-type", "text/html")], "<html>502</html>");
        assert!(matches!(e, ApiError::NotEvnx { status: 502, .. }), "{e:?}");
        let t = e.to_string();
        assert!(
            t.contains("answered HTTP 502 with a page that is not from evnx"),
            "{t}"
        );
        assert!(t.contains("proxy, firewall or captive portal"), "{t}");

        // A 5xx whose body is not parseable but claims JSON is still evnx's.
        let e = failure(500, &[("content-type", "application/json")], "not json");
        assert!(matches!(e, ApiError::ServerError { .. }), "{e:?}");

        // 404 stays NotFound whatever the body.
        let e = failure(404, &[("content-type", "text/html")], "<html>404</html>");
        assert!(matches!(e, ApiError::NotFound { .. }), "{e:?}");
    }

    #[test]
    #[serial]
    fn retry_after_is_read_for_429_and_503() {
        let e = failure(
            429,
            &[("retry-after", "30")],
            r#"{"error":"Too many requests","code":"RATE_LIMITED"}"#,
        );
        match &e {
            ApiError::RateLimited { retry_after, .. } => assert_eq!(*retry_after, Some(30)),
            other => panic!("{other:?}"),
        }
        assert!(
            e.to_string().contains("try again in 30 s")
                || e.to_string().contains("Try again in 30 s"),
            "{e}"
        );

        let e = failure(
            503,
            &[("retry-after", "7")],
            r#"{"error":"down","code":"INTERNAL_ERROR"}"#,
        );
        match &e {
            ApiError::ServerError { retry_after, .. } => assert_eq!(*retry_after, Some(7)),
            other => panic!("{other:?}"),
        }
        assert!(e.to_string().contains("Try again in 7 s"), "{e}");

        // An HTTP-date is unknown, not an error.
        let e = failure(
            429,
            &[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")],
            r#"{"error":"Too many requests","code":"RATE_LIMITED"}"#,
        );
        assert!(
            matches!(
                e,
                ApiError::RateLimited {
                    retry_after: None,
                    ..
                }
            ),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "Too many requests");
    }

    #[test]
    fn x3_exit_codes() {
        let s = || "s".to_string();
        let cases: Vec<(ApiError, Option<i32>)> = vec![
            (ApiError::NotSignedIn { server: s() }, Some(10)),
            (ApiError::Unauthorized, Some(10)),
            (ApiError::TokenRejected, Some(10)),
            (ApiError::RefreshNotPersisted { message: s() }, Some(10)),
            (
                ApiError::Forbidden {
                    message: s(),
                    reason: None,
                    required_role: None,
                },
                Some(11),
            ),
            (ApiError::EmailNotVerified, Some(11)),
            (ApiError::PlanLimit { message: s() }, Some(12)),
            (
                ApiError::Transport {
                    server: s(),
                    reason: s(),
                },
                Some(13),
            ),
            (
                ApiError::NotEvnx {
                    server: s(),
                    status: 502,
                },
                Some(13),
            ),
            (ApiError::Conflict { message: s() }, Some(14)),
            (ApiError::Locked { message: s() }, Some(14)),
            (
                ApiError::ServerError {
                    server: s(),
                    status: 500,
                    retry_after: None,
                },
                Some(15),
            ),
            (
                ApiError::RateLimited {
                    message: s(),
                    retry_after: None,
                },
                Some(15),
            ),
            (
                ApiError::NotFound {
                    server: s(),
                    path: s(),
                },
                None,
            ),
            (ApiError::Validation { message: s() }, None),
            (ApiError::Malformed { message: s() }, None),
        ];
        for (e, want) in cases {
            assert_eq!(e.exit_code(), want, "{e:?}");
        }
    }

    #[test]
    fn the_exit_code_survives_context_and_rewording_without_repeating_text() {
        use anyhow::Context;
        let e: Result<(), ApiError> = Err(ApiError::Unauthorized);
        let wrapped = e.context("could not list your vaults").unwrap_err();
        assert_eq!(exit_code_of(&wrapped), Some(10));
        let printed = format!("{wrapped:#}");
        assert_eq!(
            printed.matches("session has expired").count(),
            1,
            "{printed}"
        );

        let r = reword(Some(14), "custom text");
        assert_eq!(exit_code_of(&r), Some(14));
        assert_eq!(format!("{r:#}"), "custom text");

        // Local does not print its inner error twice.
        let l = anyhow::Error::from(ApiError::Local(anyhow::anyhow!("inner").context("outer")));
        assert_eq!(format!("{l:#}"), "outer: inner");
        assert_eq!(exit_code_of(&l), None);
        let tls = anyhow::Error::new(crate::core::http::TlsRootsError {
            variable: "SSL_CERT_FILE",
            path: "/x.pem".into(),
            reason: "that file does not exist".into(),
        })
        .context("building the HTTP client");
        assert_eq!(exit_code_of(&tls), Some(13));
        let local_tls = ApiError::Local(tls);
        assert_eq!(local_tls.exit_code(), Some(13));
    }

    #[test]
    #[serial]
    fn a_401_triggers_one_refresh_and_a_retry() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();

        let first = server
            .mock("GET", "/api/v1/vaults")
            .match_header("authorization", "Bearer access-1")
            .with_status(401)
            .with_body(r#"{"error":"Authentication failed","code":"UNAUTHORIZED"}"#)
            .expect(1)
            .create();
        let refreshed = server
            .mock("POST", "/api/v1/auth/refresh")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"refresh_token":"refresh-1"}"#.into(),
            ))
            .with_status(200)
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2"}}"#,
                jwt_with_exp(4_000_000_000)
            ))
            .expect(1)
            .create();
        let second = server
            .mock("GET", "/api/v1/vaults")
            .match_header(
                "authorization",
                mockito::Matcher::Regex("Bearer ey.*".into()),
            )
            .with_status(200)
            .with_body("[]")
            .expect(1)
            .create();

        let client = Client::new(server.url()).unwrap();
        let _: Vec<serde_json::Value> = client.get("/api/v1/vaults").unwrap();

        first.assert();
        refreshed.assert();
        second.assert();
    }

    #[test]
    #[serial]
    fn the_rotated_refresh_token_is_written_to_disk() {
        // The server revokes the old refresh token on use. If the new one is not
        // persisted, the next command is signed out with no way back.
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();

        let _m1 = server
            .mock("GET", "/api/v1/vaults")
            .match_header("authorization", "Bearer access-1")
            .with_status(401)
            .with_body(r#"{"error":"x","code":"UNAUTHORIZED"}"#)
            .create();
        let _m2 = server
            .mock("POST", "/api/v1/auth/refresh")
            .with_status(200)
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2"}}"#,
                jwt_with_exp(1_900_000_000)
            ))
            .create();
        let _m3 = server
            .mock("GET", "/api/v1/vaults")
            .match_header(
                "authorization",
                mockito::Matcher::Regex("Bearer ey.*".into()),
            )
            .with_status(200)
            .with_body("[]")
            .create();

        let client = Client::new(server.url()).unwrap();
        let _: Vec<serde_json::Value> = client.get("/api/v1/vaults").unwrap();

        let on_disk = Store::load().unwrap();
        let session = on_disk.session(&server.url()).unwrap();
        assert_eq!(session.refresh_token.expose(), "refresh-2");
        assert_eq!(session.access_expires_at, 1_900_000_000);
    }

    #[test]
    #[serial]
    fn a_dead_refresh_token_fails_once_instead_of_looping() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();

        let call = server
            .mock("GET", "/api/v1/vaults")
            .with_status(401)
            .with_body(r#"{"error":"x","code":"UNAUTHORIZED"}"#)
            .expect(1)
            .create();
        let refresh = server
            .mock("POST", "/api/v1/auth/refresh")
            .with_status(401)
            .with_body(r#"{"error":"Authentication failed","code":"UNAUTHORIZED"}"#)
            .expect(1)
            .create();

        let client = Client::new(server.url()).unwrap();
        let err = client
            .get::<serde_json::Value>("/api/v1/vaults")
            .unwrap_err();

        assert!(matches!(err, ApiError::Unauthorized), "{err:?}");
        // Exactly one of each: no retry storm against a server that is refusing.
        call.assert();
        refresh.assert();
    }

    #[test]
    #[serial]
    fn an_expired_access_token_refreshes_before_the_request_not_after() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        // Already expired, so the client should not even try the old token.
        signed_in(&server.url(), 1).save().unwrap();

        let refresh = server
            .mock("POST", "/api/v1/auth/refresh")
            .with_status(200)
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2"}}"#,
                jwt_with_exp(4_000_000_000)
            ))
            .expect(1)
            .create();
        let call = server
            .mock("GET", "/api/v1/vaults")
            .match_header(
                "authorization",
                mockito::Matcher::Regex("Bearer ey.*".into()),
            )
            .with_status(200)
            .with_body("[]")
            .expect(1)
            .create();

        let client = Client::new(server.url()).unwrap();
        let _: Vec<serde_json::Value> = client.get("/api/v1/vaults").unwrap();
        refresh.assert();
        call.assert();
    }

    #[test]
    #[serial]
    fn an_unreachable_server_is_a_transport_error_naming_the_host() {
        let _g = ConfigDirGuard::new();
        // Port 1 on loopback: nothing listens, so the connection is refused
        // immediately rather than hanging.
        let client = Client::new("http://127.0.0.1:1").unwrap();
        let err = client.health().unwrap_err();
        assert!(matches!(err, ApiError::Transport { .. }), "{err:?}");
        assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
        assert!(err.is_transient());
    }

    #[test]
    #[serial]
    fn delete_accepts_an_empty_body() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let m = server
            .mock("DELETE", "/api/v1/vaults/abc")
            .with_status(204)
            .create();

        let client = Client::new(server.url()).unwrap();
        client.delete("/api/v1/vaults/abc").unwrap();
        m.assert();
    }

    #[test]
    #[serial]
    fn a_success_body_that_is_not_the_expected_shape_is_reported_clearly() {
        let _g = ConfigDirGuard::new();
        let mut server = mockito::Server::new();
        signed_in(&server.url(), 4_000_000_000).save().unwrap();
        let _m = server
            .mock("GET", "/api/v1/vaults")
            .with_status(200)
            .with_body(r#"{"unexpected":"shape"}"#)
            .create();

        let client = Client::new(server.url()).unwrap();
        let err = client
            .get::<Vec<serde_json::Value>>("/api/v1/vaults")
            .unwrap_err();
        assert!(matches!(err, ApiError::Malformed { .. }), "{err:?}");
        assert!(err.to_string().contains("/api/v1/vaults"));
    }
}

#[cfg(test)]
mod api_path_tests {
    use super::*;

    /// What the HTTP client will really send for `path` — `reqwest::Url` applies
    /// the URL standard's dot-segment rules, so this is the wire path.
    fn on_the_wire(path: &str) -> String {
        let u = reqwest::Url::parse(&format!("http://h{path}")).unwrap();
        let mut s = u.path().to_string();
        if let Some(q) = u.query() {
            s.push('?');
            s.push_str(q);
        }
        if let Some(f) = u.fragment() {
            s.push('#');
            s.push_str(f);
        }
        s
    }

    /// I-217, DEC-OR2: user text was interpolated into URL paths, so
    /// `org uninvite "../members/u-bob"` sent `DELETE /orgs/o/members/u-bob`.
    #[test]
    fn t_i217_user_text_stays_inside_its_segment() {
        for (seg, wire) in [
            (
                "../members/u-bob",
                "/api/v1/orgs/o/invites/..%2Fmembers%2Fu-bob",
            ),
            ("a b?x=1#f", "/api/v1/orgs/o/invites/a%20b%3Fx%3D1%23f"),
            ("%2E%2E", "/api/v1/orgs/o/invites/%252E%252E"),
            ("inv-1_x.y~z", "/api/v1/orgs/o/invites/inv-1_x.y~z"),
            ("日本", "/api/v1/orgs/o/invites/%E6%97%A5%E6%9C%AC"),
        ] {
            let p = api_path(&["orgs", "o", "invites", seg]).unwrap();
            assert_eq!(on_the_wire(&p), wire, "{seg:?} built {p}");
        }
    }

    /// An email in a path keeps working (it is how `vault share` looks a
    /// recipient up), but cannot leave its segment.
    #[test]
    fn t_i217_an_email_is_one_segment() {
        let p = api_path(&["users", "a+b@example.com", "public-key"]).unwrap();
        assert_eq!(
            on_the_wire(&p),
            "/api/v1/users/a%2Bb%40example.com/public-key"
        );
        let p = api_path(&["users", "x/../../vaults", "public-key"]).unwrap();
        assert_eq!(
            on_the_wire(&p),
            "/api/v1/users/x%2F..%2F..%2Fvaults/public-key"
        );
    }

    /// The URL standard collapses `.` and `..` segments — and their encoded
    /// forms — before sending, so no encoding makes them safe: refused.
    #[test]
    fn t_i217_dot_segments_and_empty_segments_are_refused() {
        for seg in ["..", ".", ""] {
            let err = api_path(&["orgs", "o", "invites", seg]).unwrap_err();
            assert!(err.to_string().contains("request path"), "{seg:?}: {err}");
        }
    }
}
