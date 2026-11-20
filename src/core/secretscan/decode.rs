//! One level of decoding (RM-71): a value that is base64, hex or
//! percent-encoded text is decoded once, and the text is checked against the
//! format rules.
//!
//! * **One level.** What comes out is never decoded again.
//! * **Capped.** A candidate is 16 to [`MAX_CANDIDATE`] characters, and one
//!   file offers at most [`MAX_CANDIDATES_PER_FILE`] of them.
//! * **Text or nothing.** The result must be valid UTF-8 with no control
//!   characters but tab and line breaks. Images, hashes, ciphertext and random
//!   base64 decode to binary and are dropped here, before any rule sees them.
//! * **Format rules only** (D8): the caller checks the text against fixed
//!   prefixes, private-key headers and connection strings, never against
//!   entropy or names, because decoded random bytes always look random.

use base64::Engine as _;

/// Shortest and longest candidate, in characters.
pub const MIN_CANDIDATE: usize = 16;
pub const MAX_CANDIDATE: usize = 4096;
/// The most candidates one file may offer for decoding, so a minified bundle
/// of tokens costs a bounded amount.
pub const MAX_CANDIDATES_PER_FILE: usize = 10_000;

/// How a value was encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Base64,
    Hex,
    Url,
}

impl Encoding {
    /// The name findings carry in `encoded`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Base64 => "base64",
            Encoding::Hex => "hex",
            Encoding::Url => "url",
        }
    }
}

/// The candidate tokens of one line: runs of the characters base64, hex and
/// percent-encoding use, within the length bounds.
pub fn tokens(line: &str) -> Vec<&str> {
    let is_tok = |c: char| c.is_ascii_alphanumeric() || "+/_%.~-".contains(c);
    let mut out = Vec::new();
    let mut start = None;
    let bytes = line.as_bytes();
    for (i, c) in line
        .char_indices()
        .chain(std::iter::once((line.len(), ' ')))
    {
        match (is_tok(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                // Up to two `=` right after the run are base64 padding.
                let mut end = i;
                while end < line.len() && end - i < 2 && bytes[end] == b'=' {
                    end += 1;
                }
                let t = line[s..end].trim_matches(['.', '~']);
                if (MIN_CANDIDATE..=MAX_CANDIDATE).contains(&t.len()) {
                    out.push(t);
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Every way `token` decodes to text: at most one per encoding.
#[must_use]
pub fn decode(token: &str) -> Vec<(Encoding, String)> {
    let mut out = Vec::new();
    if token.contains('%') {
        if let Some(t) = percent(token) {
            out.push((Encoding::Url, t));
        }
        return out;
    }
    if let Some(t) = hex(token) {
        out.push((Encoding::Hex, t));
    }
    if let Some(t) = base64(token) {
        out.push((Encoding::Base64, t));
    }
    out
}

/// Valid UTF-8 text that a person could have written: no control characters
/// but tab, CR and LF, and long enough to hold something.
fn printable(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    let ok = text.chars().count() >= 8
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'));
    ok.then_some(text)
}

fn hex(token: &str) -> Option<String> {
    if !token.len().is_multiple_of(2) || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes: Option<Vec<u8>> = token
        .as_bytes()
        .chunks(2)
        .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
        .collect();
    printable(bytes?)
}

fn base64(token: &str) -> Option<String> {
    if !token.len().is_multiple_of(4) {
        return None;
    }
    let std = token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    let url = token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'='));
    let bytes = if std {
        base64::engine::general_purpose::STANDARD
            .decode(token)
            .ok()?
    } else if url {
        base64::engine::general_purpose::URL_SAFE
            .decode(token)
            .ok()?
    } else {
        return None;
    };
    printable(bytes)
}

fn percent(token: &str) -> Option<String> {
    let b = token.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut escapes = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3)?;
            let v = std::str::from_utf8(h)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())?;
            out.push(v);
            escapes += 1;
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    if escapes == 0 {
        return None;
    }
    printable(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_hex_and_percent_decode_to_text() {
        let b = base64::engine::general_purpose::STANDARD.encode("password=hunter2hunter2");
        assert_eq!(
            decode(&b),
            vec![(Encoding::Base64, "password=hunter2hunter2".into())]
        );
        let h = "70617373776f72643d68756e74657232";
        assert_eq!(decode(h)[0], (Encoding::Hex, "password=hunter2".into()));
        let u = "postgres%3A%2F%2Fu%3Ap%40db.example.com%2Fapp";
        assert_eq!(
            decode(u),
            vec![(Encoding::Url, "postgres://u:p@db.example.com/app".into())]
        );
    }

    #[test]
    fn binary_and_random_values_do_not_decode() {
        // A PNG header, a SHA-256 in hex, and random base64.
        let png = base64::engine::general_purpose::STANDARD.encode([
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 1, 2, 3, 4,
        ]);
        assert!(decode(&png).is_empty());
        assert!(
            decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").is_empty()
        );
        assert!(decode("Zk9x2QmL7vRtB4nYc8WdHs3JaP6eUo1G").is_empty());
        // Not a multiple of four, and one level only: no `%` and no digits of text.
        assert!(decode("abcdefghijklmnopq").is_empty());
    }

    #[test]
    fn tokens_are_bounded() {
        assert!(tokens("a: short").is_empty());
        let long = "A".repeat(MAX_CANDIDATE + 1);
        assert!(tokens(&long).is_empty());
        assert_eq!(
            tokens("key: \"QUJDREVGR0hJSktMTU5PUA==\""),
            ["QUJDREVGR0hJSktMTU5PUA=="]
        );
        // `NAME=value`: the `=` is not part of the value.
        assert_eq!(
            tokens("BLOB=70617373776f72643d68756e74657232"),
            ["70617373776f72643d68756e74657232"]
        );
    }
}
