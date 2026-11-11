//! How a platform request fails, and the exit code each failure asks for.
//!
//! The codes are `cloud`'s contract (X3): 10 the token was refused, 11 it may
//! not do that, 13 the server could not be reached, 15 the server failed or the
//! rate limit never cleared, 14 a variable changed on Vercel since the last
//! pull or push. Everything else keeps the usual 2.
//!
//! Nothing here carries a value: a message names a status, a key or an id, and
//! the one server sentence it may repeat (a 4xx `error.message`) is cut short.

/// Everything that can go wrong talking to a platform's API.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// 401.
    #[error(
        "Vercel refused the token (HTTP 401): it is mistyped, expired or revoked.\n  \
         Create one at https://vercel.com/account/tokens and pass it in VERCEL_TOKEN \
         or --token-file."
    )]
    Unauthorized,

    /// 403 with `error.invalidToken: true`: the token itself is bad, which is
    /// what a 401 would say. Vercel answers some bad tokens this way.
    #[error(
        "Vercel refused the token (HTTP 403): the token is invalid or expired; \
         create one at vercel.com/account/tokens.\n  \
         Pass it in VERCEL_TOKEN or --token-file."
    )]
    InvalidToken,

    /// 403.
    #[error(
        "Vercel says this token may not read that (HTTP 403){}.\n  \
         The token needs access to the project, and to its team when it belongs to one \
         (--team / VERCEL_ORG_ID).",
        detail_suffix(detail.as_deref())
    )]
    Forbidden {
        /// The server's sentence, when it sent one.
        detail: Option<String>,
    },

    /// 404.
    #[error(
        "Vercel answered 404 for {what}.\n  \
         Check --project / VERCEL_PROJECT_ID, and --team / VERCEL_ORG_ID for a team project."
    )]
    NotFound {
        /// What was asked for.
        what: String,
    },

    /// Any other 4xx.
    #[error("Vercel refused the request (HTTP {status}){}", detail_suffix(detail.as_deref()))]
    Refused {
        /// Status returned.
        status: u16,
        /// The server's sentence, when it sent one.
        detail: Option<String>,
    },

    /// 5xx.
    #[error("Vercel reported a server error (HTTP {status}). Nothing was changed.")]
    ServerError {
        /// Status returned.
        status: u16,
    },

    /// 429, and still 429 after the last try.
    #[error(
        "Vercel's rate limit was still in force after {tries} tries. \
         Nothing was changed; try again in a minute."
    )]
    RateLimited {
        /// How many requests were made.
        tries: u32,
    },

    /// The request never completed.
    #[error("could not reach {host}: {reason}")]
    Transport {
        /// Host that could not be reached.
        host: String,
        /// Shortest useful description (see `core::http::transport_reason`).
        reason: String,
    },

    /// A status this client does not interpret.
    #[error("unexpected response from Vercel: HTTP {status}")]
    Unexpected {
        /// Status returned.
        status: u16,
    },

    /// The body was not the JSON this client expects.
    #[error("could not read Vercel's response: {0}")]
    Malformed(String),
}

fn detail_suffix(detail: Option<&str>) -> String {
    match detail {
        Some(d) if !d.is_empty() => format!(": {d}"),
        _ => String::new(),
    }
}

impl PlatformError {
    /// The process exit code this failure asks for, or `None` for the usual 2.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            PlatformError::Unauthorized | PlatformError::InvalidToken => Some(10),
            PlatformError::Forbidden { .. } => Some(11),
            PlatformError::Transport { .. } => Some(13),
            PlatformError::ServerError { .. } | PlatformError::RateLimited { .. } => Some(15),
            PlatformError::NotFound { .. }
            | PlatformError::Refused { .. }
            | PlatformError::Unexpected { .. }
            | PlatformError::Malformed(_) => None,
        }
    }
}

/// The exit code a platform failure asks for, found anywhere in the chain.
/// An unusable `SSL_CERT_FILE` is the same class as an unreachable server.
pub fn exit_code_of(err: &anyhow::Error) -> Option<i32> {
    err.chain().find_map(|c| {
        if let Some(p) = c.downcast_ref::<PlatformError>() {
            p.exit_code()
        } else if c.is::<crate::core::http::TlsRootsError>() {
            Some(13)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_follow_the_cloud_contract() {
        assert_eq!(PlatformError::Unauthorized.exit_code(), Some(10));
        assert_eq!(PlatformError::InvalidToken.exit_code(), Some(10));
        assert_eq!(
            PlatformError::Forbidden { detail: None }.exit_code(),
            Some(11)
        );
        assert_eq!(
            PlatformError::Transport {
                host: "h".into(),
                reason: "r".into()
            }
            .exit_code(),
            Some(13)
        );
        assert_eq!(
            PlatformError::ServerError { status: 500 }.exit_code(),
            Some(15)
        );
        assert_eq!(
            PlatformError::RateLimited { tries: 3 }.exit_code(),
            Some(15)
        );
        assert_eq!(
            PlatformError::NotFound { what: "x".into() }.exit_code(),
            None
        );
    }

    #[test]
    fn the_code_is_found_through_context() {
        let e = anyhow::Error::new(PlatformError::Unauthorized).context("listing variables");
        assert_eq!(exit_code_of(&e), Some(10));
        assert_eq!(exit_code_of(&anyhow::anyhow!("plain")), None);
    }
}
