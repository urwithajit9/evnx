//! One type for every value that might be a secret (WP-17).
//!
//! A value read from a `.env` file, a vault or a platform, or a token. Every value
//! is treated as a secret; whether it *is* one is decided later (`classify`).
//!
//! The contract:
//!
//! * `Debug` prints `Secret(<redacted, N bytes>)`: `dbg!`, `{:?}`, a panic message
//!   or a failing `assert_eq!` never shows the text. The length tells empty from set.
//! * There is **no** `Display`, `Serialize`, `Deref<Target = str>` or `AsRef<str>`.
//!   Printing or writing a value is a deliberate call to [`Secret::expose`], and
//!   `tests/architecture.rs` lists the files that may call it, each with a reason.
//! * Serde writers that must emit the text use [`serialize_exposed`] or
//!   [`exposed_map`], which are on the same allow-list.
//! * The text is wiped on drop. Best effort: growth copies inside the parser,
//!   `serde_json`, `reqwest` and the allocator are not reached.
//! * Comparison is plain `==` (not constant time): local data, no oracle.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serializer};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A value that wipes itself on drop and never appears in `Debug` output.
#[derive(Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct Secret(String);

/// The name tokens and platform values were given before there was one type.
pub type SecretString = Secret;

impl Secret {
    /// Wrap a value.
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    /// The only way to read the text. Named so call sites read as a deliberate
    /// act; every file that calls it is on the allow-list in `tests/architecture.rs`.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Length in characters (what reports print as `<N chars>`).
    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }

    /// Whether the value is the empty string.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<redacted, {} bytes>)", self.0.len())
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret(s.to_string())
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Secret {}

impl PartialEq<str> for Secret {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}
impl PartialEq<&str> for Secret {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}
impl PartialEq<String> for Secret {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Secret)
    }
}

/// `#[serde(serialize_with = "secret::serialize_exposed")]`: write the text.
/// For files that must hold the value (credentials, request bodies).
pub fn serialize_exposed<S: Serializer>(v: &Secret, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(v.expose())
}

/// A borrowed `name -> text` view of a map, for writers (`serde_json`, templates).
pub fn exposed_map(vars: &IndexMap<String, Secret>) -> IndexMap<&str, &str> {
    vars.iter().map(|(k, v)| (k.as_str(), v.expose())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canary() -> String {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        format!(
            "cnry-{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        )
    }

    #[test]
    fn debug_never_holds_the_text_but_holds_the_length() {
        let c = canary();
        let s = Secret::new(c.clone());
        let d = format!("{s:?}");
        assert!(!d.contains(&c[..5]), "{d}");
        assert_eq!(d, format!("Secret(<redacted, {} bytes>)", c.len()));
        assert_eq!(
            format!("{:?}", Secret::new("")),
            "Secret(<redacted, 0 bytes>)"
        );
        let nested = format!("{:#?}", Some(vec![s]));
        assert!(!nested.contains(&c[..5]));
    }

    #[test]
    fn clone_and_equality() {
        let a = Secret::from("x=1");
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a, "x=1");
        assert_eq!(a, *"x=1");
        assert_eq!(a, String::from("x=1"));
        assert_ne!(a, Secret::from("x=2"));
        assert_eq!(a.len(), 3);
        assert_eq!(Secret::from("é").char_count(), 1);
        assert_eq!(Secret::from("é").len(), 2);
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn deserializes_from_a_json_string_and_exposes_through_the_map_helper() {
        let s: Secret = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(s.expose(), "abc");
        let mut m = IndexMap::new();
        m.insert("K".to_string(), s);
        let json = serde_json::to_string(&exposed_map(&m)).unwrap();
        assert_eq!(json, r#"{"K":"abc"}"#);
    }

    #[test]
    fn serialize_exposed_writes_the_text() {
        #[derive(serde::Serialize)]
        struct W {
            #[serde(serialize_with = "serialize_exposed")]
            v: Secret,
        }
        let json = serde_json::to_string(&W {
            v: Secret::from("t0k"),
        })
        .unwrap();
        assert_eq!(json, r#"{"v":"t0k"}"#);
    }

    #[test]
    fn zeroize_wipes_the_held_text() {
        let mut s = Secret::from("wipe-me");
        s.zeroize();
        assert!(s.is_empty());
        assert_eq!(s.expose(), "");
    }
}
