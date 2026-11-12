//! The CI key bundle — `EVNX_KEY_BUNDLE`.
//!
//! An API token authenticates; it does not decrypt. To decrypt, CI needs the
//! account's Argon2 salt and sealed keypair, which `GET /auth/me` used to hand
//! to any credential. A server can stop serving key material to API tokens, so a
//! signed-in machine exports it once into a bundle, and CI reads the bundle from
//! its own secret store instead.
//!
//! ```text
//! evnxkb1_<base64url, no padding, of {"v":1,"user_id":…,"argon2_salt":…,"encrypted_private_key":…}>
//! ```
//!
//! The bundle holds **no plaintext secret**. It is the same sealed material
//! `/auth/me` returns: the keypair is encrypted under a key derived from the
//! master password, so the bundle alone opens nothing. It is still stored like
//! the token, as a CI secret, and never printed unless the user asks.

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Environment variable holding the bundle.
pub const ENV_KEY_BUNDLE: &str = "EVNX_KEY_BUNDLE";
/// Environment variable naming a file whose first line is the bundle.
pub const ENV_KEY_BUNDLE_FILE: &str = "EVNX_KEY_BUNDLE_FILE";

/// Prefix and format version of the string form.
pub const PREFIX: &str = "evnxkb1_";

/// The only payload version this build writes and reads.
const VERSION: u8 = 1;

/// What `unlock_with` needs, plus the account it belongs to.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct KeyBundle {
    /// Payload version.
    pub v: u8,
    /// The account the material belongs to.
    pub user_id: String,
    /// Argon2id salt, base64 (not a secret).
    pub argon2_salt: String,
    /// The keypair sealed under the master key, base64.
    pub encrypted_private_key: String,
}

impl KeyBundle {
    /// A version-1 bundle for an account.
    pub fn new(user_id: &str, argon2_salt: &str, encrypted_private_key: &str) -> Self {
        KeyBundle {
            v: VERSION,
            user_id: user_id.to_string(),
            argon2_salt: argon2_salt.to_string(),
            encrypted_private_key: encrypted_private_key.to_string(),
        }
    }

    /// The `evnxkb1_…` string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a bundle of strings serialises");
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    /// Parse the string form. The error says what is wrong and never echoes the
    /// input.
    pub fn decode(text: &str) -> Result<Self> {
        let body = text.trim().strip_prefix(PREFIX).ok_or_else(|| {
            anyhow!(
                "the key bundle does not start with {PREFIX} — it is not a key bundle from \
                 `evnx auth key-bundle` (or it was cut short when it was pasted)"
            )
        })?;
        let raw = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| anyhow!("the key bundle is damaged: its body is not valid base64url"))?;
        let b: KeyBundle = serde_json::from_slice(&raw)
            .map_err(|_| anyhow!("the key bundle is damaged: its contents are not readable"))?;
        if b.v != VERSION {
            return Err(anyhow!(
                "this key bundle is version {}, which this evnx does not read. Update evnx, or \
                 create a new bundle with `evnx auth key-bundle`",
                b.v
            ));
        }
        if b.user_id.is_empty() || b.argon2_salt.is_empty() || b.encrypted_private_key.is_empty() {
            return Err(anyhow!("the key bundle is incomplete: a field is empty"));
        }
        Ok(b)
    }
}

/// The bundle CI was given, from [`ENV_KEY_BUNDLE`] or the first line of the
/// file named by [`ENV_KEY_BUNDLE_FILE`]. `Ok(None)` when neither is set.
pub fn from_env() -> Result<Option<KeyBundle>> {
    if let Ok(v) = std::env::var(ENV_KEY_BUNDLE) {
        if !v.trim().is_empty() {
            return KeyBundle::decode(&v)
                .map(Some)
                .with_context(|| format!("reading {ENV_KEY_BUNDLE}"));
        }
    }
    if let Ok(p) = std::env::var(ENV_KEY_BUNDLE_FILE) {
        if !p.trim().is_empty() {
            let text = std::fs::read_to_string(p.trim())
                .with_context(|| format!("reading the file named by {ENV_KEY_BUNDLE_FILE}"))?;
            let first = text.lines().next().unwrap_or("");
            return KeyBundle::decode(first)
                .map(Some)
                .with_context(|| format!("reading the file named by {ENV_KEY_BUNDLE_FILE}"));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KeyBundle {
        KeyBundle::new("u-1", "c2FsdA", "c2VhbGVk")
    }

    #[test]
    fn a_bundle_round_trips() {
        let s = sample().encode();
        assert!(s.starts_with(PREFIX), "{s}");
        assert!(!s.contains(['\n', ' ', '=', '+', '/']), "{s}");
        assert_eq!(KeyBundle::decode(&format!("  {s}\r\n")).unwrap(), sample());
    }

    #[test]
    fn malformed_bundles_are_named_and_not_echoed() {
        for bad in [
            "".to_string(),
            "nonsense".to_string(),
            format!("{PREFIX}!!!"),
            format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(b"not json")),
            format!(
                "{PREFIX}{}",
                URL_SAFE_NO_PAD.encode(
                    br#"{"v":9,"user_id":"a","argon2_salt":"b","encrypted_private_key":"c"}"#
                )
            ),
            format!(
                "{PREFIX}{}",
                URL_SAFE_NO_PAD.encode(
                    br#"{"v":1,"user_id":"","argon2_salt":"b","encrypted_private_key":"c"}"#
                )
            ),
        ] {
            let e = KeyBundle::decode(&bad).unwrap_err().to_string();
            assert!(e.contains("key bundle"), "{e}");
            assert!(bad.is_empty() || !e.contains(&bad), "echoed: {e}");
        }
    }
}
