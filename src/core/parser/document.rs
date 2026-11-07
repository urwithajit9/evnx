//! What a `.env` file says, line by line, including what is wrong with it.
//!
//! [`Parser::parse_document`](super::Parser::parse_document) never fails: a bad
//! line becomes a [`Problem`] and reading continues on the next one, so a
//! command can report every problem in one run, and a reader that must carry on
//! (`doctor`, `scan`) still sees every entry around a broken line.
//!
//! ⚠️ **A document holds facts, not opinions.** Duplicate keys, placeholders and
//! reader disagreements are judgements made *from* a document by `validate`,
//! `doctor` and `dialect`. `problems` holds only what the strict parser refuses,
//! which keeps `doc.is_clean() ⇔ parse_content(x).is_ok()` exact.
//!
//! ⛔ **No `Serialize`, on purpose.** `Entry::value` is a secret. Anything that
//! prints a document builds its own output and decides there whether values
//! appear, so there is no `serde_json::to_string(&doc)` that dumps them.

use indexmap::IndexMap;

use super::{ParseError, ParseResult};

/// Where something was written: 1-based lines, inclusive. `end_line > line`
/// only for a value that spans lines. `line == 0` means it did not come from a
/// file (JSON input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub end_line: usize,
}

impl Span {
    pub fn line(n: usize) -> Self {
        Span {
            line: n,
            end_line: n,
        }
    }
}

/// One assignment, as written and as read.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    /// The final value: unescaped and expanded. An entry whose expansion failed
    /// keeps its unexpanded value, and the failure is a [`Problem`].
    pub value: String,
    pub span: Span,
    /// The source text of the whole entry; a multiline value's lines are
    /// joined with `\n`.
    pub raw: String,
    pub exported: bool,
    /// `"`, `'` or a backtick; `None` for an unquoted value.
    pub quote: Option<char>,
}

/// A full-line comment. `text` is everything after the `#`, untrimmed.
#[derive(Debug, Clone)]
pub struct Comment {
    pub line: usize,
    pub text: String,
}

/// Something the strict parser refuses, with the text it was found in. An
/// unterminated quote spans from where it opened to the end of the file.
#[derive(Debug)]
pub struct Problem {
    pub span: Span,
    pub raw: String,
    pub error: ParseError,
}

impl Problem {
    /// `Line N: <reason>`, in the parser's wording. Never the line's text: a
    /// line without `=` is sometimes a pasted secret.
    pub fn describe(&self) -> String {
        let n = self.span.line;
        // The parser says "… at line N"; the line leads here instead.
        let reason = self
            .error
            .to_string()
            .replacen(&format!(" at line {n}"), "", 1);
        format!("Line {n}: {reason}")
    }
}

#[derive(Debug, Default)]
pub struct Document {
    /// Every assignment in file order, **duplicates included**: a key assigned
    /// twice has two entries, and the later one is the one that counts.
    pub entries: Vec<Entry>,
    pub comments: Vec<Comment>,
    /// Line-pass problems in line order, then expansion problems in key order.
    /// That order is what makes `problems[0]` the error the strict parser
    /// returns; sort by `span` to show them.
    pub problems: Vec<Problem>,
    /// The file began with a UTF-8 byte-order mark, which was skipped. A fact
    /// for `dialect`: python-dotenv keeps it in the first key's name.
    pub bom: bool,
}

impl Document {
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// Entries whose value holds a NUL byte (B2 P0.6). It parses, but no
    /// process can receive it: an environment is NUL-terminated C strings, so
    /// `exec` refuses it and Docker will not start the container.
    pub fn nul_bytes(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.value.contains('\0'))
    }

    /// Entries whose text holds a carriage return that does not end a CRLF
    /// line (B2 P0.5): a classic Mac line ending, or a stray paste. evnx and
    /// Docker Compose keep it in the value; python-dotenv and Node dotenv end
    /// the line there, so `K=v\rL=w` is one variable here and two there.
    pub fn lone_carriage_returns(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.raw.replace("\r\n", "").contains('\r'))
    }

    /// One value per key: the **last** assignment's value, in the **first**
    /// assignment's position. That is `IndexMap::insert`, which is what the
    /// parser always returned, and every command depends on that order.
    pub fn vars(&self) -> IndexMap<String, String> {
        let mut out = IndexMap::with_capacity(self.entries.len());
        for e in &self.entries {
            out.insert(e.key.clone(), e.value.clone());
        }
        out
    }

    /// The line each key in [`Document::vars`] was assigned on: the last
    /// assignment's opening line, in the same order.
    pub fn lines(&self) -> IndexMap<String, usize> {
        let mut out = IndexMap::with_capacity(self.entries.len());
        for e in &self.entries {
            out.insert(e.key.clone(), e.span.line);
        }
        out
    }

    /// The lines of entries that read `key`'s value before its last assignment:
    /// after an earlier assignment, before the last one. Since 0.13.0 expansion
    /// runs top to bottom (D10), so such an entry sees the earlier value, and
    /// that assignment is not dead. Literal forms (single quotes, backticks)
    /// read nothing.
    pub fn read_before_last(&self, key: &str) -> Vec<usize> {
        let lines: Vec<usize> = self
            .entries
            .iter()
            .filter(|e| e.key == key)
            .map(|e| e.span.line)
            .collect();
        let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|e| e.span.line > first && e.span.line < last)
            .filter(|e| !matches!(e.quote, Some('\'') | Some('`')))
            .filter(|e| {
                // The text after `=`. In double quotes `\$` is an escaped
                // dollar, not a reference; unquoted, it is still expanded.
                let value = e.raw.split_once('=').map_or("", |(_, v)| v);
                let value = if e.quote == Some('"') {
                    value.replace("\\$", &super::expand::LITERAL_DOLLAR.to_string())
                } else {
                    value.to_string()
                };
                super::expand::references(&value).iter().any(|n| n == key)
            })
            .map(|e| e.span.line)
            .collect()
    }

    /// References to a name assigned only **below** them: `(line, name,
    /// assigned)`. Expansion runs top to bottom (D10), so such a reference sees
    /// nothing. A `${NAME}` one is already a [`ParseError::ForwardReference`];
    /// in a file that parses, these are bare `$NAME`s, kept as written, which
    /// 0.12.0 expanded with the value from below.
    ///
    /// [`ParseError::ForwardReference`]: super::ParseError::ForwardReference
    pub fn forward_references(&self) -> Vec<(usize, String, usize)> {
        let mut out = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if matches!(e.quote, Some('\'') | Some('`')) {
                continue;
            }
            let value = e.raw.split_once('=').map_or("", |(_, v)| v);
            let value = if e.quote == Some('"') {
                value.replace("\\$", &super::expand::LITERAL_DOLLAR.to_string())
            } else {
                value.to_string()
            };
            for name in super::expand::references(&value) {
                if self.entries[..i].iter().any(|o| o.key == name) {
                    continue;
                }
                if let Some(below) = self.entries[i + 1..].iter().find(|o| o.key == name) {
                    out.push((e.span.line, name, below.span.line));
                }
            }
        }
        out
    }

    /// The strict reading: the variables and their lines, or the first problem.
    pub fn into_result(self) -> ParseResult<(IndexMap<String, String>, IndexMap<String, usize>)> {
        if self.problems.is_empty() {
            return Ok((self.vars(), self.lines()));
        }
        Err(self.problems.into_iter().next().expect("not empty").error)
    }
}
