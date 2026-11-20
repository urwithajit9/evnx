//! One value read from a file or from standard input (`--from-file`).
//!
//! For a certificate, a private key, a service-account JSON: values that are
//! several lines long and that nobody should have to paste through a shell.
//! `evnx add custom NAME --from-file <path|->` and `evnx set NAME --from-file
//! <path|->` read it here, so the two commands apply the same rules:
//!
//! * at most [`CAP`] (64 KiB) of input;
//! * text only: input that is not UTF-8 (or holds a NUL byte) is refused
//!   unless `--base64` is given, which stores the file's bytes as base64 text;
//! * one final line break (`\n` or `\r\n`) is not part of the value, unless
//!   `--keep-trailing-newline`;
//! * an empty value is refused: it would write `NAME=`, which says "unset";
//! * standard input must be piped; a terminal is refused.
//!
//! The value comes back as a [`Secret`], and nothing here puts it in a message:
//! errors and [`describe`] speak in sizes only.

use std::io::{IsTerminal, Read};
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;

use super::secret::Secret;
use crate::core::gitignore::{self, Exposure};

/// The most input read: 64 KiB.
pub const CAP: usize = 64 * 1024;

/// What the flags around `--from-file` ask for.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Store the input's bytes as base64 text (binary input).
    pub base64: bool,
    /// Keep the final line break.
    pub keep_trailing_newline: bool,
}

/// `--from-file SOURCE` with its two modifiers, as a command carries them.
#[derive(Debug, Clone)]
pub struct FromFile {
    /// A path, or `-` for standard input.
    pub source: String,
    pub options: Options,
}

impl FromFile {
    /// Read the value.
    pub fn read(&self) -> Result<Secret> {
        read(&self.source, self.options)
    }
}

/// `32 KiB` style size for "is 70 KiB; the limit is 64 KiB".
fn kib(bytes: usize) -> String {
    format!("{} KiB", bytes.div_ceil(1024))
}

/// Read the value `source` names: a path, or `-` for standard input.
///
/// # Errors
///
/// An unreadable file, input over the cap, input that is not text (without
/// `--base64`), an empty value, or a terminal on standard input. Every one is
/// a usage-level refusal (exit 2); none names the value.
pub fn read(source: &str, opts: Options) -> Result<Secret> {
    let bytes = if source == "-" {
        read_stdin()?
    } else {
        read_path(Path::new(source))?
    };
    decode(&bytes, source, opts)
}

fn read_stdin() -> Result<Vec<u8>> {
    if std::io::stdin().is_terminal() {
        return Err(crate::core::verdict::usage(
            "--from-file - reads standard input, and standard input is a terminal.\n\n  \
             Pipe the value in:   evnx set NAME --from-file - < value.txt\n  \
             Or name the file:    evnx set NAME --from-file value.txt",
        ));
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(CAP as u64 + 1)
        .read_to_end(&mut bytes)
        .context("reading the value from standard input")?;
    if bytes.len() > CAP {
        return Err(too_big("standard input", None));
    }
    Ok(bytes)
}

fn read_path(path: &Path) -> Result<Vec<u8>> {
    let shown = crate::utils::path::label(&path.to_string_lossy());
    let meta = std::fs::metadata(path).map_err(|e| {
        crate::core::verdict::usage(format!("cannot read {shown} (--from-file): {}", reason(&e)))
    })?;
    if !meta.is_file() {
        return Err(crate::core::verdict::usage(format!(
            "{shown} is not a file (--from-file)"
        )));
    }
    if meta.len() > CAP as u64 {
        return Err(too_big(&shown, Some(meta.len() as usize)));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(CAP as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| {
            crate::core::verdict::usage(format!(
                "cannot read {shown} (--from-file): {}",
                reason(&e)
            ))
        })?;
    if bytes.len() > CAP {
        return Err(too_big(&shown, None));
    }
    Ok(bytes)
}

fn reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no such file".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    }
}

fn too_big(what: &str, size: Option<usize>) -> anyhow::Error {
    let is = match size {
        Some(n) => format!("is {}", kib(n)),
        None => "is over 64 KiB".to_string(),
    };
    crate::core::verdict::usage(format!(
        "{what} {is}; the limit is 64 KiB. A value that large is not an environment \
         variable: keep it as a file and put its path in the variable."
    ))
}

/// Turn the bytes into the value, by the rules above. Split from [`read`] so
/// the rules are tested without a file.
pub fn decode(bytes: &[u8], source: &str, opts: Options) -> Result<Secret> {
    if bytes.len() > CAP {
        return Err(too_big(source, Some(bytes.len())));
    }
    let shown = if source == "-" {
        "standard input".to_string()
    } else {
        crate::utils::path::label(source)
    };
    let value = if opts.base64 {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Err(crate::core::verdict::usage(format!(
                "{shown} is not UTF-8 text. For a binary file add --base64: the value is \
                 stored as base64 text, and decoded by whatever reads it."
            )));
        };
        if text.contains('\0') {
            return Err(crate::core::verdict::usage(format!(
                "{shown} holds a NUL byte, which an environment variable cannot. \
                 For a binary file add --base64."
            )));
        }
        let mut text = text;
        if !opts.keep_trailing_newline {
            if let Some(t) = text.strip_suffix('\n') {
                text = t.strip_suffix('\r').unwrap_or(t);
            }
        }
        text.to_string()
    };
    if value.is_empty() {
        return Err(crate::core::verdict::usage(format!(
            "{shown} is empty: there is no value to write."
        )));
    }
    Ok(Secret::new(value))
}

/// `1,842 bytes, 31 lines`: what a dry run says instead of the value.
pub fn describe(value: &Secret) -> String {
    let text = value.expose();
    let lines = if text.is_empty() {
        0
    } else {
        text.split('\n').count() - usize::from(text.ends_with('\n') && text.split('\n').count() > 1)
    };
    format!(
        "{} {}, {} {}",
        grouped(text.len()),
        if text.len() == 1 { "byte" } else { "bytes" },
        lines,
        if lines == 1 { "line" } else { "lines" }
    )
}

/// `1842` as `1,842`.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A warning for a source file git would commit: inside a repository, and
/// either tracked or not ignored. `None` for stdin, a file outside any
/// repository, or an ignored one.
pub fn exposure_warning(source: &str) -> Option<String> {
    if source == "-" {
        return None;
    }
    let path = Path::new(source);
    let abs = std::fs::canonicalize(path).ok()?;
    let dir = abs.parent()?;
    let name = abs.file_name()?.to_string_lossy().into_owned();
    if !gitignore::in_work_tree(dir) {
        return None;
    }
    let shown = crate::utils::path::label(source);
    match gitignore::exposure(dir, &name) {
        Exposure::Protected => None,
        Exposure::Unignored => Some(format!(
            "{shown} is inside this repository and git does not ignore it: the next \
             `git add .` commits the value. Add it to .gitignore, or keep it outside the repository."
        )),
        Exposure::Tracked => Some(format!(
            "{shown} is tracked by git, so the value is already in the repository. \
             Untrack it (`git rm --cached {shown}`) and rotate the value."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> Options {
        Options::default()
    }

    #[test]
    fn one_final_newline_is_stripped_and_only_one() {
        assert_eq!(decode(b"abc\n", "f", o()).unwrap().expose(), "abc");
        assert_eq!(decode(b"abc\r\n", "f", o()).unwrap().expose(), "abc");
        assert_eq!(decode(b"abc\n\n", "f", o()).unwrap().expose(), "abc\n");
        assert_eq!(decode(b"a\nb\n", "f", o()).unwrap().expose(), "a\nb");
    }

    #[test]
    fn keep_trailing_newline_keeps_it() {
        let k = Options {
            keep_trailing_newline: true,
            ..o()
        };
        assert_eq!(decode(b"abc\n", "f", k).unwrap().expose(), "abc\n");
        assert_eq!(decode(b"abc\r\n", "f", k).unwrap().expose(), "abc\r\n");
    }

    #[test]
    fn crlf_inside_stays() {
        assert_eq!(decode(b"a\r\nb\r\n", "f", o()).unwrap().expose(), "a\r\nb");
    }

    #[test]
    fn the_cap_is_64_kib_and_the_message_says_sizes() {
        let ok = vec![b'a'; CAP];
        assert!(decode(&ok, "f", o()).is_ok());
        let big = vec![b'a'; 70 * 1024];
        let e = decode(&big, "big.pem", o()).unwrap_err().to_string();
        assert!(e.contains("is 70 KiB; the limit is 64 KiB"), "{e}");
    }

    #[test]
    fn binary_is_refused_without_base64_and_encoded_with_it() {
        let bytes = [0xff, 0xfe, 0x00, 0x01];
        let e = decode(&bytes, "k.bin", o()).unwrap_err().to_string();
        assert!(e.contains("--base64"), "{e}");
        let b = Options {
            base64: true,
            ..o()
        };
        assert_eq!(decode(&bytes, "k.bin", b).unwrap().expose(), "//4AAQ==");
        assert!(decode(b"a\0b", "f", o()).is_err());
    }

    #[test]
    fn base64_keeps_the_newline_it_encodes() {
        let b = Options {
            base64: true,
            ..o()
        };
        assert_eq!(decode(b"hi\n", "f", b).unwrap().expose(), "aGkK");
    }

    #[test]
    fn an_empty_value_is_refused() {
        assert!(decode(b"", "f", o()).is_err());
        assert!(decode(b"\n", "f", o()).is_err());
    }

    #[test]
    fn describe_counts_bytes_and_lines_without_the_value() {
        let v = Secret::new("a\nb\nc");
        assert_eq!(describe(&v), "5 bytes, 3 lines");
        assert_eq!(describe(&Secret::new("x")), "1 byte, 1 line");
        let big = Secret::new("x".repeat(1842));
        assert_eq!(describe(&big), "1,842 bytes, 1 line");
    }

    #[test]
    fn errors_never_hold_the_value() {
        let e = decode("secret-canary\u{0}".as_bytes(), "f", o())
            .unwrap_err()
            .to_string();
        assert!(!e.contains("secret-canary"), "{e}");
    }
}
