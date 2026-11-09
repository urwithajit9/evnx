//! A string that wipes itself on drop and never appears in `Debug` output.
//!
//! Holds the Vercel token and every value read back from Vercel. `cloud` has its
//! own `SecretString`, but that module only exists with the `cloud` feature and
//! `platform` needs only `net`; the two are the same idea (same `expose`
//! convention), kept apart so neither feature drags in the other.
//!
//! Zeroizing is best effort: it cannot reach the copies `reqwest`, `serde_json`
//! or the allocator make on the way in.

use zeroize::Zeroize;

/// A token or a variable's value. `{:?}` prints `SecretString(<redacted>)`;
/// there is no `Display`.
pub struct SecretString(String);

impl SecretString {
    /// Wrap a secret.
    pub fn new(s: impl Into<String>) -> Self {
        SecretString(s.into())
    }

    /// Borrow the secret. Named so call sites read as a deliberate act.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the secret is the empty string.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SecretString {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_secret() {
        let s = SecretString::new("hunter2-xyz");
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose(), "hunter2-xyz");
    }
}
