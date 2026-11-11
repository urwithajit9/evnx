//! Reading what a scan walks (C1, RM-45): decoding bytes into text, and
//! whether a file is read as a `.env`. Which files are walked is
//! [`super::filters`]; what is found in them is `core::secretscan::engine`.
//! Moved from `runner.rs` unchanged.

use std::path::Path;

/// Should this file be read with the `.env` parser?
///
/// ⛔ **By file name, not by path.** This was
/// `path.to_string_lossy().contains(".env")`, which is true of every file under
/// a directory that happens to be called `.env` — and a Python virtualenv is
/// routinely named exactly that. So every file in `.env/lib/python3/site-packages`
/// was handed to the dotenv parser, and a line like `self.password = password`
/// in somebody else's library became a finding.
///
/// ⚠️ It was also true of `.env.yaml` and `.env.json`, which are *not* dotenv.
/// Those parse as nothing useful, and the raw fallback only looks at lines
/// containing `=`, so `STRIPE_SECRET_KEY: sk_live_…` in a `.env.yaml` was
/// scanned by neither path and reported clean.
pub(super) fn named_like_dotenv(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // ⚠️ `app.env` and `production.env` too (I-254): the name Docker Compose's
    // `env_file:` and many deploy tools use. Read as plain text, a `[vars]`
    // declaration never applied to them — `secret = true` reported nothing.
    let suffixed = name.len() > ".env".len() && name.ends_with(".env");
    if name != ".env" && !name.starts_with(".env.") && !suffixed {
        return false;
    }
    // ⓘ `.env.yaml` is YAML that happens to be named after `.env`. Anything with
    // a known structured-format extension is left to the content detectors,
    // which read it as text rather than as assignments. One list with
    // `doctor`, which skips its dotenv syntax check for the same files.
    !crate::core::env_name::is_structured(name)
}

/// The structured format a file is read as, by its extension (C8): YAML,
/// JSON or TOML. Checked after [`named_like_dotenv`], so `.env.json` is JSON.
pub(super) fn structured_format(path: &Path) -> Option<crate::core::secretscan::engine::Format> {
    use crate::core::secretscan::engine::Format;
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "yml" | "yaml" => Some(Format::Yaml),
        "json" => Some(Format::Json),
        "toml" => Some(Format::Toml),
        _ => None,
    }
}

/// Whether `path` is a Jupyter notebook, by extension (F14-6).
pub(super) fn is_notebook(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ipynb"))
}

/// Whether `path` is a PKCS#12 keystore (`.p12`, `.pfx`), by its content
/// (C3): a private key and its certificate, usually behind a weak password.
///
/// The file is binary, so a walk used to skip it as "not text". DER for a
/// PKCS#12 `PFX` is a SEQUENCE whose first element is INTEGER 3 (`30 len 02 01
/// 03`), which an X.509 certificate (SEQUENCE, SEQUENCE) and a PKCS#8 key
/// (INTEGER 0) are not. Checked by content, so a renamed keystore is found too.
pub(super) fn is_pkcs12(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    is_pkcs12_bytes(&head[..n])
}

pub(super) fn is_pkcs12_bytes(b: &[u8]) -> bool {
    if b.first() != Some(&0x30) || b.len() < 2 {
        return false;
    }
    // The SEQUENCE's length: short form, long form (0x81/0x82/…), or BER's
    // indefinite 0x80.
    let at = match b[1] {
        0x80 => 2,
        l if l & 0x80 != 0 => 2 + (l & 0x7f) as usize,
        _ => 2,
    };
    b.get(at..at + 3) == Some(&[0x02, 0x01, 0x03][..])
}

// ─────────────────────────────────────────────────────────────
// Decoding
// ─────────────────────────────────────────────────────────────

/// The outcome of trying to read a file as text.
pub(super) enum Decoded {
    Text(String),
    Unreadable { reason: String },
}

/// Read a file as text, decoding UTF-16 rather than giving up on it.
///
/// ⚠️ **There are two ways a UTF-16 `.env` defeated a UTF-8 read, and only one
/// of them was an error.** Fixing the error alone leaves the other.
///
/// * **With a byte-order mark**, `read_to_string` fails: `0xFF` is not valid
///   UTF-8. That surfaced as `Err`, and the `Err` arm returned `Ok(())`.
/// * **Without one, `read_to_string` succeeds** — UTF-16LE ASCII is *valid
///   UTF-8*, because every byte is either an ASCII character or `0x00`, and
///   `0x00` is a legal UTF-8 code unit. So the scanner received
///   `"A\0W\0S\0_\0S\0E\0C\0R\0E\0T\0"` and no pattern matched a single
///   key or value. It genuinely scanned the file, and genuinely found nothing.
///
/// So a NUL byte is the signal, not a decoding error: no UTF-8 text file
/// contains one, and a UTF-16 one is half NULs by construction.
pub(super) fn decode(path: &Path) -> Decoded {
    match std::fs::read(path) {
        Ok(bytes) => decode_bytes(bytes),
        Err(e) => Decoded::Unreadable {
            // ⚠️ `e` and not the path: the caller records the path, and an
            // io::Error's Display does not include it.
            reason: format!("could not be read: {e}"),
        },
    }
}

/// How `evnx scan -` names stdin in findings and lists (RM-48).
pub(super) const STDIN_LABEL: &str = "<stdin>";

/// [`decode`] for bytes already read.
pub(super) fn decode_bytes(bytes: Vec<u8>) -> Decoded {
    // An explicit byte-order mark settles it.
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return from_utf16(rest, u16::from_le_bytes, "UTF-16LE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return from_utf16(rest, u16::from_be_bytes, "UTF-16BE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        // A UTF-8 BOM. Not an encoding problem, but left in the string it
        // becomes part of the first key's name.
        return match String::from_utf8(rest.to_vec()) {
            Ok(text) => Decoded::Text(text),
            Err(e) => Decoded::Unreadable {
                reason: format!("has a UTF-8 byte-order mark but is not valid UTF-8: {e}"),
            },
        };
    }

    // No BOM. A NUL means this is not UTF-8 text whatever `from_utf8` says.
    if bytes.contains(&0) {
        // UTF-16 without a byte-order mark. Try both parities and judge the
        // **result**, not the input.
        //
        // ⚠️ The first version of this counted NUL bytes by position — "at
        // least half the even-numbered bytes are zero, so it is UTF-16BE". A
        // six-byte binary blob, `00 FF 13 37 00 01`, satisfies that by
        // accident and was decoded into Ethiopic and Latin-Extended
        // characters, which the caller then scanned as though it were the
        // file's text. Its own test caught it.
        //
        // A ratio of input bytes cannot distinguish those cases. What can is
        // whether the decode *produced text*: a `.env` is ASCII keys, `=` and
        // newlines, with at most a few non-ASCII characters in values, and
        // never a C0 control character other than tab, newline or return.
        if bytes.len().is_multiple_of(2) && !bytes.is_empty() {
            for (unit, label) in [
                (
                    u16::from_le_bytes as fn([u8; 2]) -> u16,
                    "UTF-16LE (no BOM)",
                ),
                (
                    u16::from_be_bytes as fn([u8; 2]) -> u16,
                    "UTF-16BE (no BOM)",
                ),
            ] {
                if let Decoded::Text(text) = from_utf16(&bytes, unit, label) {
                    if looks_like_text(&text) {
                        return Decoded::Text(text);
                    }
                }
            }
        }
        return Decoded::Unreadable {
            reason: "contains NUL bytes and does not decode as UTF-16, so it is not text \
                     this scanner can read"
                .to_string(),
        };
    }

    match String::from_utf8(bytes) {
        Ok(text) => Decoded::Text(text),
        Err(e) => Decoded::Unreadable {
            reason: format!("is not valid UTF-8: {e}"),
        },
    }
}

/// Did a candidate decode actually produce text?
///
/// Two conditions, both cheap and both decisive on the cases that matter:
///
/// * **No C0 control characters** beyond tab, newline and carriage return.
///   Binary interpreted as UTF-16 lands on these almost immediately.
/// * **Mostly ASCII.** A `.env` is ASCII keys, `=`, and newlines; a non-ASCII
///   value is possible but cannot dominate the file. Binary decoded as UTF-16
///   produces characters spread across the whole BMP, so it fails this by a
///   wide margin rather than a narrow one.
fn looks_like_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut ascii = 0usize;
    let mut total = 0usize;
    for c in text.chars() {
        if c.is_control() && !matches!(c, '\t' | '\n' | '\r') {
            return false;
        }
        if c.is_ascii() {
            ascii += 1;
        }
        total += 1;
    }
    // Half, not all: a password with an accent in it is still a .env file.
    ascii * 2 >= total
}

/// Decode UTF-16 code units into a `String`.
fn from_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16, label: &str) -> Decoded {
    if !bytes.len().is_multiple_of(2) {
        return Decoded::Unreadable {
            reason: format!("looks like {label} but has an odd number of bytes"),
        };
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    match String::from_utf16(&units) {
        Ok(text) => Decoded::Text(text),
        Err(e) => Decoded::Unreadable {
            reason: format!("looks like {label} but does not decode: {e}"),
        },
    }
}

#[cfg(test)]
mod pkcs12_tests {
    use super::is_pkcs12_bytes;

    #[test]
    fn pkcs12_is_recognised_by_its_version_and_others_are_not() {
        assert!(is_pkcs12_bytes(&[
            0x30, 0x82, 0x0a, 0x1b, 0x02, 0x01, 0x03, 0x30
        ]));
        assert!(is_pkcs12_bytes(&[0x30, 0x80, 0x02, 0x01, 0x03]));
        assert!(is_pkcs12_bytes(&[0x30, 0x40, 0x02, 0x01, 0x03]));
        // X.509 certificate: SEQUENCE, SEQUENCE.
        assert!(!is_pkcs12_bytes(&[
            0x30, 0x82, 0x03, 0x0b, 0x30, 0x82, 0x02
        ]));
        // PKCS#8 private key: INTEGER 0.
        assert!(!is_pkcs12_bytes(&[
            0x30, 0x82, 0x04, 0xbe, 0x02, 0x01, 0x00
        ]));
        assert!(!is_pkcs12_bytes(b"\x89PNG\r\n"));
        assert!(!is_pkcs12_bytes(&[]));
    }
}
