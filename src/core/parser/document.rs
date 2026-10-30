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

    /// The strict reading: the variables and their lines, or the first problem.
    pub fn into_result(self) -> ParseResult<(IndexMap<String, String>, IndexMap<String, usize>)> {
        if self.problems.is_empty() {
            return Ok((self.vars(), self.lines()));
        }
        Err(self.problems.into_iter().next().expect("not empty").error)
    }
}
