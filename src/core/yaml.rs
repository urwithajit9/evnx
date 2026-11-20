//! The one place evnx reads YAML.
//!
//! evnx only reads YAML: the scan engine's structured leaves, `init`'s compose
//! detection and `add import`. Every YAML writer is hand-written. The reader
//! is `yaml-rust2` (pure Rust); this module turns its parser events into
//! [`Value`], so callers never see the crate.
//!
//! What a plain (unquoted) scalar becomes is pinned by `tests/yaml_reader.rs`
//! and follows the rules evnx had with `serde_yaml` (YAML 1.2 core schema):
//!
//! * `null`, `Null`, `NULL`, `~` and nothing at all are [`Value::Null`];
//! * `true`/`True`/`TRUE` and `false`/`False`/`FALSE` are [`Value::Bool`];
//!   the YAML 1.1 words `yes`, `no`, `on`, `off`, `y`, `n` are **strings**;
//! * integers (decimal, `0x`, `0o`, `0b`, a sign) and floats (`1.5`, `1e3`,
//!   `.inf`, `.nan`) are [`Value::Number`], kept **as written**: `1.10` stays
//!   `1.10`, `0x1F` stays `0x1F`. A leading zero (`007`) or an underscore
//!   (`1_000`) makes it a string;
//! * everything else, and every quoted or block scalar, is a string.
//!
//! A core tag (`!!str 1`, `!!int "7"`) picks the type; any other tag
//! (`!Ref x`) is ignored. Anchors and aliases are resolved (the alias gets a
//! copy). The merge key `<<` is **not** applied: it is an ordinary key whose
//! value is the merged mapping. A byte-order mark at the start is skipped.
//!
//! A mapping keeps the file's order. A key is the text as written (`1.10`,
//! `~`); a key that is itself a list or a mapping is an error. A key given
//! twice is an error with [`Duplicates::Refuse`], or the last one counts (in
//! the first one's place) with [`Duplicates::LastWins`].
//!
//! Limits, so a hostile file cannot exhaust memory or the stack of a caller
//! that walks the result: nesting deeper than [`MAX_DEPTH`] and more than
//! [`MAX_ALIAS_NODES`] nodes copied through aliases are errors.
//!
//! ⛔ An [`Error`] holds a position and a fixed reason, never text from the
//! file: the text may be a secret.

use std::collections::HashMap;
use std::fmt;

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// Deeper nesting than this is refused (the old reader stopped at 128 too).
pub const MAX_DEPTH: usize = 128;

/// At most this many nodes are copied through aliases in one file.
pub const MAX_ALIAS_NODES: usize = 100_000;

/// A YAML value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    /// An integer or a float, as written.
    Number(String),
    String(String),
    Seq(Vec<Value>),
    /// A mapping in the file's order; keys as written.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The entry `key` of a mapping.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Value]> {
        match self {
            Value::Seq(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// How many nodes this value is, itself included.
    fn nodes(&self) -> usize {
        1 + match self {
            Value::Seq(items) => items.iter().map(Value::nodes).sum(),
            Value::Map(m) => m.iter().map(|(_, v)| 1 + v.nodes()).sum(),
            _ => 0,
        }
    }
}

/// What to do with a key given twice in one mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplicates {
    /// An error, as the old reader did for scan and `init`.
    Refuse,
    /// The last one counts, in the first one's place (`add import`).
    LastWins,
}

/// Why a file could not be read: a position and a fixed reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// 1-based.
    pub line: usize,
    /// 1-based.
    pub column: usize,
    pub reason: &'static str,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not valid YAML (line {}, column {}): {}",
            self.line, self.column, self.reason
        )
    }
}

impl std::error::Error for Error {}

const SYNTAX: &str = "a syntax error";
const DUPLICATE: &str = "a key appears twice in one mapping";
const COMPLEX_KEY: &str = "a key is a list or a mapping";
const TOO_DEEP: &str = "nested too deeply";
const TOO_MANY_ALIASES: &str = "aliases expand to too many values";
const UNKNOWN_ALIAS: &str = "an alias names no anchor";
const MANY_DOCUMENTS: &str = "more than one document";

/// Every document of `text` (a `---`-separated stream). An empty document is
/// [`Value::Null`]; an empty text is no document at all.
///
/// # Errors
///
/// The first problem in any document; nothing is returned then.
pub fn documents(text: &str, duplicates: Duplicates) -> Result<Vec<Value>, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut b = Builder {
        duplicates,
        docs: Vec::new(),
        stack: Vec::new(),
        root: None,
        anchors: HashMap::new(),
        copied: 0,
        error: None,
    };
    let mut parser = Parser::new(text.chars());
    if let Err(e) = parser.load(&mut b, true) {
        // A problem the builder found first is the one to report.
        return Err(b.error.unwrap_or_else(|| at(e.marker(), SYNTAX)));
    }
    match b.error {
        Some(e) => Err(e),
        None => Ok(b.docs),
    }
}

/// The one document of `text` ([`Value::Null`] when the text is empty).
///
/// # Errors
///
/// As [`documents`], and a text with more than one document.
pub fn document(text: &str, duplicates: Duplicates) -> Result<Value, Error> {
    let mut docs = documents(text, duplicates)?;
    if docs.len() > 1 {
        return Err(Error {
            line: 1,
            column: 1,
            reason: MANY_DOCUMENTS,
        });
    }
    Ok(docs.pop().unwrap_or(Value::Null))
}

fn at(mark: &Marker, reason: &'static str) -> Error {
    Error {
        line: mark.line().max(1),
        column: mark.col() + 1,
        reason,
    }
}

enum Frame {
    Seq(Vec<Value>, usize),
    Map {
        entries: Vec<(String, Value)>,
        key: Option<String>,
        anchor: usize,
    },
}

struct Builder {
    duplicates: Duplicates,
    docs: Vec<Value>,
    stack: Vec<Frame>,
    root: Option<Value>,
    anchors: HashMap<usize, Value>,
    copied: usize,
    error: Option<Error>,
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        if let Err(reason) = self.event(ev) {
            self.error = Some(at(&mark, reason));
        }
    }
}

impl Builder {
    fn event(&mut self, ev: Event) -> Result<(), &'static str> {
        match ev {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentStart => {}
            Event::DocumentEnd => {
                self.docs.push(self.root.take().unwrap_or(Value::Null));
                self.anchors.clear();
            }
            Event::SequenceStart(anchor, _) => {
                self.deeper()?;
                self.stack.push(Frame::Seq(Vec::new(), anchor));
            }
            Event::MappingStart(anchor, _) => {
                self.deeper()?;
                self.stack.push(Frame::Map {
                    entries: Vec::new(),
                    key: None,
                    anchor,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let (value, anchor) = match self.stack.pop() {
                    Some(Frame::Seq(items, anchor)) => (Value::Seq(items), anchor),
                    Some(Frame::Map {
                        entries, anchor, ..
                    }) => (Value::Map(entries), anchor),
                    None => return Err(SYNTAX),
                };
                self.insert(value, None, anchor)?;
            }
            Event::Scalar(text, style, anchor, tag) => {
                let value = scalar(&text, style, tag.as_ref());
                self.insert(value, Some(text), anchor)?;
            }
            Event::Alias(id) => {
                let value = self.anchors.get(&id).cloned().ok_or(UNKNOWN_ALIAS)?;
                self.copied += value.nodes();
                if self.copied > MAX_ALIAS_NODES {
                    return Err(TOO_MANY_ALIASES);
                }
                let key = match &value {
                    Value::String(s) | Value::Number(s) => Some(s.clone()),
                    Value::Bool(b) => Some(b.to_string()),
                    Value::Null => Some(String::new()),
                    Value::Seq(_) | Value::Map(_) => None,
                };
                self.insert(value, key, 0)?;
            }
        }
        Ok(())
    }

    fn deeper(&self) -> Result<(), &'static str> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(TOO_DEEP);
        }
        Ok(())
    }

    /// Put a finished node into its parent. `key` is its text when it is a
    /// scalar, in case the parent is a mapping waiting for a key.
    fn insert(
        &mut self,
        value: Value,
        key: Option<String>,
        anchor: usize,
    ) -> Result<(), &'static str> {
        if anchor > 0 {
            self.anchors.insert(anchor, value.clone());
        }
        match self.stack.last_mut() {
            None => self.root = Some(value),
            Some(Frame::Seq(items, _)) => items.push(value),
            Some(Frame::Map {
                entries, key: slot, ..
            }) => match slot.take() {
                None => *slot = Some(key.ok_or(COMPLEX_KEY)?),
                Some(k) => match entries.iter_mut().find(|(ek, _)| *ek == k) {
                    Some(_) if self.duplicates == Duplicates::Refuse => return Err(DUPLICATE),
                    Some(existing) => existing.1 = value,
                    None => entries.push((k, value)),
                },
            },
        }
        Ok(())
    }
}

/// What a scalar is, by its style, its tag and its text.
fn scalar(text: &str, style: TScalarStyle, tag: Option<&Tag>) -> Value {
    if let Some(tag) = tag.filter(|t| t.handle == "tag:yaml.org,2002:") {
        let plain = plain(text);
        return match (tag.suffix.as_str(), &plain) {
            ("null", Value::Null)
            | ("bool", Value::Bool(_))
            | ("int" | "float", Value::Number(_)) => plain,
            _ => Value::String(text.to_string()),
        };
    }
    if style != TScalarStyle::Plain {
        return Value::String(text.to_string());
    }
    plain(text)
}

/// An unquoted, untagged scalar.
fn plain(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        _ if is_int(text) || is_float(text) => Value::Number(text.to_string()),
        _ => Value::String(text.to_string()),
    }
}

/// `007` and `-012` are strings (YAML 1.2), not numbers.
fn leading_zero(text: &str) -> bool {
    let t = text.strip_prefix(['-', '+']).unwrap_or(text);
    t.len() > 1 && t.starts_with('0') && t[1..].bytes().all(|b| b.is_ascii_digit())
}

/// An integer of any size: decimal, `0x`, `0o` or `0b`, with a sign.
fn is_int(text: &str) -> bool {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let digits = |s: &str, radix: u32| !s.is_empty() && s.chars().all(|c| c.is_digit(radix));
    if let Some(rest) = unsigned.strip_prefix("0x") {
        return digits(rest, 16);
    }
    if let Some(rest) = unsigned.strip_prefix("0o") {
        return digits(rest, 8);
    }
    if let Some(rest) = unsigned.strip_prefix("0b") {
        return digits(rest, 2);
    }
    digits(unsigned, 10) && !leading_zero(text)
}

/// A finite float as Rust reads it, or `.inf`, `-.inf`, `.nan` and their
/// capitalised forms. `inf` and `nan` alone are strings.
fn is_float(text: &str) -> bool {
    let unsigned = match text.strip_prefix('+') {
        Some(rest) if rest.starts_with(['+', '-']) => return false,
        Some(rest) => rest,
        None => text,
    };
    if matches!(unsigned, ".inf" | ".Inf" | ".INF")
        || matches!(text, "-.inf" | "-.Inf" | "-.INF" | ".nan" | ".NaN" | ".NAN")
    {
        return true;
    }
    !leading_zero(text) && unsigned.parse::<f64>().is_ok_and(f64::is_finite)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Value {
        document(text, Duplicates::Refuse).unwrap()
    }

    #[test]
    fn plain_scalars_follow_the_core_schema() {
        let v = one("a: yes\nb: true\nc: ~\nd: 1.10\ne: 007\nf: 0x1F\ng: inf\nh: .inf\n");
        let m = v.as_map().unwrap();
        assert_eq!(m[0].1, Value::String("yes".into()));
        assert_eq!(m[1].1, Value::Bool(true));
        assert_eq!(m[2].1, Value::Null);
        assert_eq!(m[3].1, Value::Number("1.10".into()));
        assert_eq!(m[4].1, Value::String("007".into()));
        assert_eq!(m[5].1, Value::Number("0x1F".into()));
        assert_eq!(m[6].1, Value::String("inf".into()));
        assert_eq!(m[7].1, Value::Number(".inf".into()));
    }

    #[test]
    fn duplicates_are_refused_or_the_last_counts() {
        let e = document("a: 1\nb: 2\na: 3\n", Duplicates::Refuse).unwrap_err();
        assert_eq!(e.reason, DUPLICATE);
        assert_eq!(e.line, 3);
        let v = document("a: 1\nb: 2\na: 3\n", Duplicates::LastWins).unwrap();
        assert_eq!(
            v,
            Value::Map(vec![
                ("a".into(), Value::Number("3".into())),
                ("b".into(), Value::Number("2".into())),
            ])
        );
    }

    #[test]
    fn deep_nesting_and_alias_bombs_are_refused() {
        let deep = format!("{}x{}", "[".repeat(200), "]".repeat(200));
        assert!(documents(&deep, Duplicates::Refuse).is_err());
        let block: String = (0..200)
            .map(|i| format!("{}k:\n", " ".repeat(i)))
            .collect::<String>();
        assert_eq!(
            documents(&block, Duplicates::Refuse).unwrap_err().reason,
            TOO_DEEP
        );
        let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
        for (i, c) in "bcdefghij".chars().enumerate() {
            let prev = (b'a' + i as u8) as char;
            bomb.push_str(&format!(
                "{c}: &{c} [{}]\n",
                vec![format!("*{prev}"); 10].join(", ")
            ));
        }
        assert_eq!(
            documents(&bomb, Duplicates::Refuse).unwrap_err().reason,
            TOO_MANY_ALIASES
        );
    }

    #[test]
    fn an_error_holds_no_text_from_the_file() {
        let e = documents("a: 'hunter2-secret\n  b: [", Duplicates::Refuse).unwrap_err();
        assert!(!e.to_string().contains("hunter2"), "{e}");
        assert!(e.to_string().starts_with("not valid YAML (line "), "{e}");
    }
}
