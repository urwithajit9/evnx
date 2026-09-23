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

    /// 403 `EMAIL_NOT_VERIFIED`.
    #[error(
        "your email address is not verified yet — check your inbox for the link from evnx. \
             Vault commands stay unavailable until it is confirmed."
    )]
    EmailNotVerified,

    /// 403 `FORBIDDEN`.
    #[error("{message}")]
    Forbidden {
        /// The server's message.
        message: String,
    },

    /// 423 `LOCKED`.
    #[error("{message}")]
    Locked {
        /// The server's message, which says how long the lock lasts.
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
    },

    /// 5xx. The server deliberately does not say what broke, so neither do we.
    #[error("{server} reported an internal error (HTTP {status}). Nothing was changed.")]
    ServerError {
        /// Server that failed.
        server: String,
        /// Status returned.
        status: u16,
    },

    /// The request never completed — DNS, connection refused, TLS, timeout.
    ///
    /// Deliberately holds a flattened `reason` rather than the `reqwest::Error`
    /// as a `#[source]`. reqwest wraps hyper wraps the OS error, so keeping the
    /// chain makes a refused connection print as five nested lines, four of them
    /// noise, with the only actionable one last. See [`transport_reason`].
    #[error("could not reach {server}: {reason}")]
    Transport {
        /// Server that could not be reached.
        server: String,
        /// Shortest useful description of what went wrong.
        reason: String,
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
    #[error("{0:#}")]
    Local(#[from] anyhow::Error),
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
}

/// Error body as evnx-server sends it.
#[derive(Deserialize)]
struct ServerError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    code: String,
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
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(timeout)
            .user_agent(concat!("evnx/", env!("CARGO_PKG_VERSION")))
            .build()
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
                reason: transport_reason(&e),
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
                reason: transport_reason(&e),
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
        let token = self.access_token()?;
        let send = |token: &SecretString| {
            self.http
                .get(self.url(path))
                .bearer_auth(token.expose())
                .send()
                .map_err(|e| ApiError::Transport {
                    server: self.server.clone(),
                    reason: transport_reason(&e),
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
            let body = resp.text().unwrap_or_default();
            return Err(self.error_from(status, &body, path));
        }

        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: transport_reason(&e),
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

    /// Send with a bearer token, refreshing once if the server says 401.
    ///
    /// Takes a builder closure rather than a `RequestBuilder` because the request
    /// may need to be constructed twice and `RequestBuilder` is not `Clone`.
    fn send_authed<T: DeserializeOwned>(
        &self,
        path: &str,
        build: impl Fn(&reqwest::blocking::Client, String) -> reqwest::blocking::RequestBuilder,
    ) -> Result<T, ApiError> {
        let token = self.access_token()?;
        let resp = build(&self.http, self.url(path))
            .bearer_auth(token.expose())
            .send()
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: transport_reason(&e),
            })?;

        if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
            return self.decode(resp, path);
        }

        // Exactly one attempt. If the refresh token is also dead, `refresh`
        // returns Unauthorized and we stop rather than looping.
        drop(resp);
        self.refresh()?;
        let token = self.access_token()?;
        let resp = build(&self.http, self.url(path))
            .bearer_auth(token.expose())
            .send()
            .map_err(|e| ApiError::Transport {
                server: self.server.clone(),
                reason: transport_reason(&e),
            })?;
        self.decode(resp, path)
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
            return Err(ApiError::Unauthorized);
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

        let body = resp.text().unwrap_or_default();
        Err(self.error_from(status, &body, path))
    }

    /// Map a failure status plus its body to a typed error.
    fn error_from(&self, status: reqwest::StatusCode, body: &str, path: &str) -> ApiError {
        let parsed: ServerError = serde_json::from_str(body).unwrap_or(ServerError {
            error: String::new(),
            code: String::new(),
        });
        let message = if parsed.error.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            parsed.error
        };

        // `code` first: EMAIL_NOT_VERIFIED and FORBIDDEN share status 403, so the
        // status alone cannot tell them apart.
        match parsed.code.as_str() {
            "UNAUTHORIZED" => ApiError::Unauthorized,
            "EMAIL_NOT_VERIFIED" => ApiError::EmailNotVerified,
            "FORBIDDEN" => ApiError::Forbidden { message },
            "LOCKED" => ApiError::Locked { message },
            "NOT_FOUND" => ApiError::NotFound {
                server: self.server.clone(),
                path: path.to_string(),
            },
            "CONFLICT" => ApiError::Conflict { message },
            "VALIDATION_ERROR" => ApiError::Validation { message },
            "RATE_LIMITED" => ApiError::RateLimited { message },
            "INTERNAL_ERROR" => ApiError::ServerError {
                server: self.server.clone(),
                status: status.as_u16(),
            },
            // No recognised code: fall back to the status, so a proxy's error
            // page or a future code still produces something sensible.
            _ => match status.as_u16() {
                401 => ApiError::Unauthorized,
                404 => ApiError::NotFound {
                    server: self.server.clone(),
                    path: path.to_string(),
                },
                s if (500..600).contains(&s) => ApiError::ServerError {
                    server: self.server.clone(),
                    status: s,
                },
                s => ApiError::Unexpected {
                    server: self.server.clone(),
                    status: s,
                },
            },
        }
    }
}

/// Shortest useful description of a transport failure.
///
/// `reqwest::Error`'s own `Display` is "error sending request for url (...)",
/// which repeats the URL the caller already knows and says nothing about the
/// cause. The cause sits at the end of a chain through hyper down to the OS
/// error, so this walks to the root and reports that instead — "Connection
/// refused (os error 111)" rather than four wrappers around it.
///
/// Timeouts are special-cased because their root cause is an opaque internal
/// type, and because "timed out" is what the user needs to read.
fn transport_reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!("timed out after {}s", REQUEST_TIMEOUT.as_secs());
    }
    let mut root: &dyn std::error::Error = e;
    while let Some(next) = root.source() {
        root = next;
    }
    root.to_string()
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
        assert!(
            matches!(err, ApiError::ServerError { status: 502, .. }),
            "{err:?}"
        );
        m.assert();
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
