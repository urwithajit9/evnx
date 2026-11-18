//! Aligning a `.env` to the layout of a reference file (`evnx align`).
//!
//! The reference — normally the committed, ordered `.env.example` — decides
//! section headings, documentation comments, blank lines and key order. Each
//! target keeps every assignment it has, **as written**: the lines of an
//! assignment are moved as one slice of the original text and never parsed into
//! a value and written back, so quoting, escapes, `${VAR}` references, inline
//! comments and multiline values cannot change.
//!
//! ⛔ **Nothing here returns a value or a source line in an error.** A
//! [`LayoutError`] carries line numbers, key names and fixed wording only. The
//! parser's own `Display` for an invalid key quotes the key, which on a broken
//! line is often a pasted secret, so its errors are translated here rather than
//! passed on.
//!
//! # The layout rule
//!
//! The output is the reference, line by line, with each reference assignment
//! replaced by the target's assignment for that key (or the placeholder
//! `# KEY=` when the target has none). Reference comments and blank lines are
//! copied in place.
//!
//! # Keys the target lacks
//!
//! ⚠️ **A placeholder, never a blank assignment.** `KEY=` is not the same as an
//! absent key, measured on synthetic files: Node's `??` and Compose's `${KEY-d}`
//! read an empty string rather than the default, Compose's `${KEY?}` accepts it,
//! a later `--env-file` wipes an earlier real value, and `evnx sync --check` and
//! `evnx diff` stop reporting the key, so `sync --direction reverse` never fills
//! it. A comment changes none of that, which is what lets this module promise
//! that aligning a file does not change what any reader gets from it.
//!
//! The reference's value is never copied. Exactly `# KEY=`, for a key the
//! reference has, is evnx's own line: it is dropped once the key has a real
//! value, and `# KEY=something` is an ordinary comment.
//!
//! # The comment rule
//!
//! A target's full-line comments are sorted into three kinds, in this order:
//!
//! 1. **Reference-owned.** A comment whose text (trimmed) also appears as a
//!    comment in the reference is a copy of the reference's documentation. It
//!    is dropped from where the target had it, because the reference copy is
//!    written at the reference's position. When the target holds **more**
//!    copies of that text than the reference, which copy is the target's own
//!    cannot be decided, and the file is refused ([`LayoutError::AmbiguousComment`]).
//!    The extras heading ([`EXTRAS_HEADING`]) and a placeholder are owned the
//!    same way: evnx regenerates them on every run. Copies of a placeholder are
//!    never ambiguous, because the line holds nothing to lose.
//! 2. **Attached.** Any other comment with no blank line between it and the
//!    next assignment (reference-owned comments in between do not count as a
//!    gap) belongs to that assignment and moves with it, written directly above
//!    it, below the reference's documentation for that key.
//! 3. **Standalone.** Any other comment — one followed by a blank line or the
//!    end of the file before the next assignment — stays after the nearest
//!    assignment above it, as its own paragraph with a blank line on each side.
//!    One above every assignment is a preamble and stays at the top.
//!
//! Every target-only comment is kept; none is ever dropped. The output is
//! checked before it is returned: it must parse, hold every original
//! assignment byte for byte, lose no comment, and align to itself unchanged.
//!
//! # Keys the reference does not have
//!
//! They are kept at the end, in their original order, under
//! [`EXTRAS_HEADING`], each with its attached and standalone comments.
//!
//! # Line endings
//!
//! Lines evnx writes (reference comments, blank lines, `# KEY=`) take the
//! target's line ending. A target that mixes CRLF and LF is refused rather than
//! guessed at. Whether the file ends with a newline, and a UTF-8 byte-order
//! mark, are kept as the target had them.

use std::collections::{HashMap, HashSet};
use std::fmt;

use super::parser::{ParseError, Parser, ParserConfig};

/// The comment line that heads the keys a reference does not have.
pub const EXTRAS_HEADING: &str = "# Environment-specific settings";

const BOM: char = '\u{feff}';

/// The line that holds a key's place in a file that lacks it.
fn placeholder(key: &str) -> String {
    format!("# {key}=")
}

/// Why a file cannot be aligned. Shown to the user, so every variant is
/// line numbers, key names and fixed text — never a value or a source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// The parser refuses this line. `reason` is evnx's own wording.
    Malformed { line: usize, reason: &'static str },
    /// The key is assigned more than once; which one counts is not a layout
    /// decision to make.
    Duplicate { key: String, lines: Vec<usize> },
    /// The file mixes CRLF and LF line endings.
    MixedLineEndings { crlf: usize, lf: usize },
    /// A comment the reference also has appears more often here than there.
    AmbiguousComment {
        line: usize,
        here: usize,
        reference: usize,
    },
    /// The file is not UTF-8 text.
    NotUtf8,
    /// A self-check on the result failed. A bug in evnx, not in the file.
    Internal(&'static str),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::Malformed { line, reason } => write!(f, "line {line}: {reason}"),
            LayoutError::Duplicate { key, lines } => {
                let lines: Vec<String> = lines.iter().map(usize::to_string).collect();
                write!(
                    f,
                    "line {}: {key} is assigned more than once (lines {}); keep one",
                    lines[0],
                    lines.join(", ")
                )
            }
            LayoutError::MixedLineEndings { crlf, lf } => write!(
                f,
                "mixed line endings ({crlf} CRLF, {lf} LF); convert the file to one style first"
            ),
            LayoutError::AmbiguousComment {
                line,
                here,
                reference,
            } => write!(
                f,
                "line {line}: this comment also appears in the template, and this file has \
                 {here} copies of it to the template's {reference}, so which one is this file's \
                 own cannot be decided; edit or remove a copy"
            ),
            LayoutError::NotUtf8 => write!(f, "not UTF-8 text"),
            LayoutError::Internal(what) => {
                write!(f, "internal check failed ({what}); this is a bug in evnx")
            }
        }
    }
}

/// The parser's error, in words that cannot carry file content.
///
/// ⚠️ Exhaustive on purpose: a new [`ParseError`] variant must be worded here
/// before it can be shown, rather than reaching output through its `Display`.
fn reason(e: &ParseError) -> &'static str {
    match e {
        ParseError::InvalidFormat { message, .. } => {
            if message.contains("'='") {
                "not an assignment, comment or blank line (no '=')"
            } else if message.contains("closing") {
                "unexpected text after a closing quote"
            } else if message.contains("export") {
                "`export` is followed by no key"
            } else {
                "invalid format"
            }
        }
        // ⛔ Never the key: on a broken line it is often the secret itself.
        ParseError::InvalidKey { key, .. } if key.is_empty() => "no key before the '='",
        ParseError::InvalidKey { .. } => {
            "invalid key name (not shown); a key holds letters, digits, _, ., -, [ and ]"
        }
        ParseError::UnterminatedString { .. } => "a quoted value opened here is never closed",
        ParseError::UndefinedVariable { .. }
        | ParseError::RequiredVariable { .. }
        | ParseError::CircularExpansion { .. }
        | ParseError::ExpansionDepthExceeded { .. }
        | ParseError::ForwardReference { .. } => "the value could not be read",
        ParseError::FileNotFound { .. } | ParseError::FileReadError(_) => "could not be read",
        ParseError::Utf16 { .. } => "the file is UTF-16; save it as UTF-8",
    }
}

/// Values are never read: expansion is off, so a reference to a variable from
/// the process environment is not an error, and nothing is interpreted.
fn parser() -> Parser {
    Parser::new(ParserConfig {
        allow_expansion: false,
        ..Default::default()
    })
}

#[derive(Debug, Clone)]
struct Line {
    /// The line without its terminator. A lone `\r` inside it stays: the
    /// parser does not end a line there either.
    text: String,
    /// `"\n"`, `"\r\n"`, or `""` for a last line with no newline.
    eol: &'static str,
}

impl Line {
    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Split exactly as `str::lines` does, which is what the parser numbers by,
/// but keeping each terminator.
fn split_lines(content: &str) -> Vec<Line> {
    content
        .split_inclusive('\n')
        .map(|seg| {
            if let Some(t) = seg.strip_suffix("\r\n") {
                Line {
                    text: t.to_string(),
                    eol: "\r\n",
                }
            } else if let Some(t) = seg.strip_suffix('\n') {
                Line {
                    text: t.to_string(),
                    eol: "\n",
                }
            } else {
                Line {
                    text: seg.to_string(),
                    eol: "",
                }
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
enum Item {
    /// Lines `first..=last` (0-based) hold the assignment of `key`.
    Assign {
        key: String,
        first: usize,
        last: usize,
    },
    Comment(usize),
    Blank(usize),
}

/// A file read for layout: its lines, and what each one is.
#[derive(Debug, Clone)]
struct Layout {
    bom: bool,
    lines: Vec<Line>,
    items: Vec<Item>,
    /// The terminator every line uses, when the file has one.
    eol: Option<&'static str>,
    /// The last line ends with a newline (or the file is empty).
    final_newline: bool,
}

impl Layout {
    fn read(content: &str, check_endings: bool) -> Result<Layout, Vec<LayoutError>> {
        let (bom, body) = match content.strip_prefix(BOM) {
            Some(rest) => (true, rest),
            None => (false, content),
        };
        let lines = split_lines(body);
        let doc = parser().parse_document(body);

        let mut errors: Vec<LayoutError> = Vec::new();
        let mut problems: Vec<_> = doc.problems.iter().collect();
        problems.sort_by_key(|p| p.span.line);
        for p in problems {
            errors.push(LayoutError::Malformed {
                line: p.span.line,
                reason: reason(&p.error),
            });
        }

        let mut seen: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut order: Vec<&str> = Vec::new();
        for e in &doc.entries {
            let at = seen.entry(e.key.as_str()).or_default();
            if at.is_empty() {
                order.push(e.key.as_str());
            }
            at.push(e.span.line);
        }
        for key in order {
            let lines = &seen[key];
            if lines.len() > 1 {
                errors.push(LayoutError::Duplicate {
                    key: key.to_string(),
                    lines: lines.clone(),
                });
            }
        }

        let crlf = lines.iter().filter(|l| l.eol == "\r\n").count();
        let lf = lines.iter().filter(|l| l.eol == "\n").count();
        if check_endings && crlf > 0 && lf > 0 {
            errors.push(LayoutError::MixedLineEndings { crlf, lf });
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        // 1-based line → the entry that starts there.
        let starts: HashMap<usize, &super::parser::Entry> =
            doc.entries.iter().map(|e| (e.span.line, e)).collect();
        let comments: HashSet<usize> = doc.comments.iter().map(|c| c.line).collect();

        let mut items = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let n = i + 1;
            if let Some(e) = starts.get(&n) {
                let last = e.span.end_line - 1;
                if last < i || last >= lines.len() {
                    return Err(vec![LayoutError::Internal("an assignment's span")]);
                }
                items.push(Item::Assign {
                    key: e.key.clone(),
                    first: i,
                    last,
                });
                i = last + 1;
                continue;
            }
            if comments.contains(&n) {
                items.push(Item::Comment(i));
            } else if lines[i].is_blank() {
                items.push(Item::Blank(i));
            } else {
                return Err(vec![LayoutError::Internal(
                    "a line the parser did not place",
                )]);
            }
            i += 1;
        }

        let eol = match (crlf, lf) {
            (0, 0) => None,
            (c, l) if c >= l => Some("\r\n"),
            _ => Some("\n"),
        };
        let final_newline = lines.last().is_none_or(|l| !l.eol.is_empty());
        Ok(Layout {
            bom,
            lines,
            items,
            eol,
            final_newline,
        })
    }

    fn keys(&self) -> impl Iterator<Item = &str> {
        self.items.iter().filter_map(|it| match it {
            Item::Assign { key, .. } => Some(key.as_str()),
            _ => None,
        })
    }

    fn comment_text(&self, i: usize) -> &str {
        self.lines[i].text.trim()
    }
}

/// A key whose place changed: 1-based lines in the target and in the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub key: String,
    pub from_line: usize,
    pub to_line: usize,
}

/// A key the target lacks, whose place is held by `# KEY=` at `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub key: String,
    pub line: usize,
}

/// What aligning one target produces, and what changed — names and line
/// numbers only, so any of it can be printed.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The aligned file. Holds the target's values: never print it.
    pub output: String,
    /// `output` differs from the target.
    pub changed: bool,
    /// Keys out of reference order in the target, and where each goes. Keys
    /// that only shifted because others moved are not listed.
    pub moved: Vec<Moved>,
    /// Keys the reference has and the target does not. Still absent afterwards.
    pub missing: Vec<Missing>,
    /// Keys the reference lacks, kept under [`EXTRAS_HEADING`].
    pub extra: Vec<Moved>,
    /// Comment lines only the target has, all kept.
    pub target_comments: usize,
}

/// A reference file, read once and applied to any number of targets.
#[derive(Debug, Clone)]
pub struct Reference {
    layout: Layout,
    keys: HashSet<String>,
    /// How many times each trimmed comment text appears.
    comment_counts: HashMap<String, usize>,
}

impl Reference {
    /// Read a reference. Errors are every problem found, in line order.
    pub fn parse(content: &str) -> Result<Reference, Vec<LayoutError>> {
        // The reference's own line endings are never written, so mixing them
        // there is harmless.
        let layout = Layout::read(content, false)?;
        let keys = layout.keys().map(str::to_string).collect();
        let mut comment_counts: HashMap<String, usize> = HashMap::new();
        for it in &layout.items {
            if let Item::Comment(i) = it {
                *comment_counts
                    .entry(layout.comment_text(*i).to_string())
                    .or_default() += 1;
            }
        }
        Ok(Reference {
            layout,
            keys,
            comment_counts,
        })
    }

    /// Align `target` to this reference.
    pub fn align(&self, target: &str) -> Result<Plan, Vec<LayoutError>> {
        let plan = self.align_unchecked(target)?;
        self.verify(target, &plan)?;
        Ok(plan)
    }

    /// How many copies of `text` a target may hold and still have every copy
    /// counted as the reference's (or evnx's) own.
    fn owned_allowance(&self, text: &str) -> Option<usize> {
        let n = self.comment_counts.get(text).copied().unwrap_or(0);
        let n = n + usize::from(text == EXTRAS_HEADING || self.is_placeholder(text));
        (n > 0).then_some(n)
    }

    /// Whether `text` is exactly the placeholder of a key this reference has.
    fn is_placeholder(&self, text: &str) -> bool {
        text.strip_prefix("# ")
            .and_then(|t| t.strip_suffix('='))
            .is_some_and(|key| self.keys.contains(key))
    }

    fn align_unchecked(&self, target: &str) -> Result<Plan, Vec<LayoutError>> {
        let t = Layout::read(target, true)?;

        // ── Comment ownership ────────────────────────────────────────────────
        let mut counts: HashMap<&str, (usize, usize)> = HashMap::new(); // text → (count, first line)
        for it in &t.items {
            if let Item::Comment(i) = it {
                let c = counts.entry(t.comment_text(*i)).or_insert((0, *i));
                c.0 += 1;
            }
        }
        let mut ambiguous: Vec<LayoutError> = counts
            .iter()
            .filter(|(text, _)| !self.is_placeholder(text))
            .filter_map(|(text, &(here, first))| {
                let allowed = self.owned_allowance(text)?;
                (here > allowed).then_some(LayoutError::AmbiguousComment {
                    line: first + 1,
                    here,
                    reference: allowed,
                })
            })
            .collect();
        if !ambiguous.is_empty() {
            ambiguous.sort_by_key(|e| match e {
                LayoutError::AmbiguousComment { line, .. } => *line,
                _ => 0,
            });
            return Err(ambiguous);
        }
        let owned = |i: usize| self.owned_allowance(t.comment_text(i)).is_some();

        // ── Attachment ───────────────────────────────────────────────────────
        // `None` anchors the preamble, above every assignment.
        let mut attached: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut standalone: HashMap<Option<&str>, Vec<Vec<usize>>> = HashMap::new();
        let mut blocks: HashMap<&str, (usize, usize)> = HashMap::new();
        let mut target_order: Vec<&str> = Vec::new();
        let mut anchor: Option<&str> = None;
        let mut pending: Vec<usize> = Vec::new();
        let mut target_comments = 0;
        for it in &t.items {
            match it {
                Item::Comment(i) if owned(*i) => {}
                Item::Comment(i) => {
                    target_comments += 1;
                    pending.push(*i);
                }
                Item::Blank(_) => {
                    if !pending.is_empty() {
                        standalone
                            .entry(anchor)
                            .or_default()
                            .push(std::mem::take(&mut pending));
                    }
                }
                Item::Assign { key, first, last } => {
                    attached.insert(key, std::mem::take(&mut pending));
                    blocks.insert(key, (*first, *last));
                    target_order.push(key);
                    anchor = Some(key);
                }
            }
        }
        if !pending.is_empty() {
            standalone.entry(anchor).or_default().push(pending);
        }

        // ── Emission ─────────────────────────────────────────────────────────
        let mut out = Out::default();
        let emit_standalone = |out: &mut Out, at: Option<&str>| {
            for para in standalone.get(&at).into_iter().flatten() {
                out.sep();
                for &i in para {
                    out.text(&t.lines[i].text);
                }
                out.sep();
            }
        };
        let emit_key = |out: &mut Out, key: &str| {
            for &i in attached.get(key).into_iter().flatten() {
                out.text(&t.lines[i].text);
            }
            let (first, last) = blocks[key];
            out.mark(key);
            for line in &t.lines[first..=last] {
                out.text(&line.text);
            }
            emit_standalone(out, Some(key));
        };

        emit_standalone(&mut out, None);
        let mut missing_keys: Vec<&str> = Vec::new();
        let r = &self.layout;
        for it in &r.items {
            match it {
                Item::Comment(i) | Item::Blank(i) => out.text(&r.lines[*i].text),
                Item::Assign { key, .. } if blocks.contains_key(key.as_str()) => {
                    emit_key(&mut out, key)
                }
                Item::Assign { key, .. } => {
                    out.mark(key);
                    out.text(&placeholder(key));
                    missing_keys.push(key);
                }
            }
        }
        let extras: Vec<&str> = target_order
            .iter()
            .copied()
            .filter(|k| !self.keys.contains(*k))
            .collect();
        if !extras.is_empty() {
            out.sep();
            out.text(EXTRAS_HEADING);
            for key in &extras {
                emit_key(&mut out, key);
            }
        }

        // An empty target has no line ending of its own to keep.
        let eol = t.eol.or(r.eol).unwrap_or("\n");
        let final_newline = t.final_newline;
        let (body, to_line) = out.render(eol, final_newline);
        let mut output = String::with_capacity(body.len() + 3);
        if t.bom {
            output.push(BOM);
        }
        output.push_str(&body);

        // ── Report ───────────────────────────────────────────────────────────
        let from_line = |k: &str| blocks[k].0 + 1;
        let kept: Vec<&str> = target_order
            .iter()
            .copied()
            .filter(|k| self.keys.contains(*k))
            .collect();
        let in_place = longest_increasing(&kept.iter().map(|k| to_line[*k]).collect::<Vec<_>>());
        let moved = kept
            .iter()
            .enumerate()
            .filter(|(i, _)| !in_place.contains(i))
            .map(|(_, k)| Moved {
                key: k.to_string(),
                from_line: from_line(k),
                to_line: to_line[*k],
            })
            .collect();
        let missing = missing_keys
            .iter()
            .map(|k| Missing {
                key: k.to_string(),
                line: to_line[*k],
            })
            .collect();
        let extra = extras
            .iter()
            .map(|k| Moved {
                key: k.to_string(),
                from_line: from_line(k),
                to_line: to_line[*k],
            })
            .collect();

        Ok(Plan {
            changed: output != target,
            output,
            moved,
            missing,
            extra,
            target_comments,
        })
    }

    /// The promises in the module note, checked on the result before anyone
    /// can write it.
    fn verify(&self, target: &str, plan: &Plan) -> Result<(), Vec<LayoutError>> {
        let fail = |what| Err(vec![LayoutError::Internal(what)]);
        let p = parser();
        let strip = |s: &str| s.strip_prefix(BOM).unwrap_or(s).to_string();
        let before = p.parse_document(&strip(target));
        let after = p.parse_document(&strip(&plan.output));
        if !after.is_clean() {
            return fail("the result does not parse");
        }

        // Exactly the target's assignments, byte for byte: nothing else may
        // become something a reader of the file can see.
        let original: HashMap<&str, &str> = before
            .entries
            .iter()
            .map(|e| (e.key.as_str(), e.raw.expose()))
            .collect();
        let mut seen = HashSet::new();
        for e in &after.entries {
            if !seen.insert(e.key.as_str()) {
                return fail("a key is written twice");
            }
            match original.get(e.key.as_str()) {
                Some(raw) if *raw == e.raw.expose() => {}
                Some(_) => return fail("an assignment changed"),
                None => return fail("an assignment appeared"),
            }
        }
        if seen.len() != original.len() {
            return fail("an assignment went missing");
        }

        // No comment is lost: every text appears at least as often as before.
        let count = |doc: &super::parser::Document| {
            let mut m: HashMap<String, usize> = HashMap::new();
            for c in &doc.comments {
                *m.entry(c.text.trim().to_string()).or_default() += 1;
            }
            m
        };
        let (b, a) = (count(&before), count(&after));
        // The extras heading and a placeholder are evnx's own lines: the heading
        // goes when the extras do, a placeholder when the key gets a value.
        let heading = EXTRAS_HEADING.trim_start_matches('#').trim();
        if b.iter()
            .filter(|(text, _)| {
                text.as_str() != heading && !self.is_placeholder(&format!("# {text}"))
            })
            .any(|(text, n)| a.get(text).copied().unwrap_or(0) < *n)
        {
            return fail("a comment went missing");
        }

        // Aligning the result again changes nothing.
        match self.align_unchecked(&plan.output) {
            Ok(again) if again.output == plan.output => Ok(()),
            _ => fail("aligning the result again would change it"),
        }
    }
}

/// The lines of a result before line endings are chosen.
#[derive(Default)]
struct Out {
    lines: Vec<OutLine>,
    /// Key → index into `lines` of its assignment's first line.
    marks: Vec<(String, usize)>,
}

enum OutLine {
    Text(String),
    /// A blank line that is written only between two non-blank lines, so
    /// paragraphs evnx separates never gain a second blank line.
    Sep,
}

impl Out {
    fn text(&mut self, s: &str) {
        self.lines.push(OutLine::Text(s.to_string()));
    }
    fn sep(&mut self) {
        self.lines.push(OutLine::Sep);
    }
    fn mark(&mut self, key: &str) {
        self.marks.push((key.to_string(), self.lines.len()));
    }

    /// The text, and the 1-based line each marked key starts on.
    fn render(self, eol: &str, final_newline: bool) -> (String, HashMap<String, usize>) {
        let blank = |s: &str| s.trim().is_empty();
        let mut kept: Vec<&str> = Vec::new();
        // `index[i]` = the output line number (0-based) that `lines[i]` became.
        let mut index = vec![0; self.lines.len() + 1];
        for (i, line) in self.lines.iter().enumerate() {
            index[i] = kept.len();
            match line {
                OutLine::Text(s) => kept.push(s),
                OutLine::Sep => {
                    let after_text = kept.last().is_some_and(|l| !blank(l));
                    let before_text = self.lines[i + 1..]
                        .iter()
                        .find_map(|l| match l {
                            OutLine::Text(s) => Some(!blank(s)),
                            OutLine::Sep => None,
                        })
                        .unwrap_or(false);
                    if after_text && before_text {
                        kept.push("");
                    }
                }
            }
        }
        index[self.lines.len()] = kept.len();
        let mut body = String::new();
        for (n, l) in kept.iter().enumerate() {
            body.push_str(l);
            if n + 1 < kept.len() || final_newline {
                body.push_str(eol);
            }
        }
        let lines = self
            .marks
            .into_iter()
            .map(|(k, i)| (k, index[i] + 1))
            .collect();
        (body, lines)
    }
}

/// Indices of one longest strictly increasing subsequence of `xs`, earliest
/// first on ties: the keys that are already in reference order.
fn longest_increasing(xs: &[usize]) -> HashSet<usize> {
    let n = xs.len();
    let mut len = vec![1usize; n];
    let mut prev = vec![usize::MAX; n];
    for i in 0..n {
        for j in 0..i {
            if xs[j] < xs[i] && len[j] + 1 > len[i] {
                len[i] = len[j] + 1;
                prev[i] = j;
            }
        }
    }
    let mut out = HashSet::new();
    let Some(mut i) = (0..n).max_by_key(|&i| (len[i], std::cmp::Reverse(i))) else {
        return out;
    };
    loop {
        out.insert(i);
        if prev[i] == usize::MAX {
            break;
        }
        i = prev[i];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &str = "\
# App settings

# === Database ===
# Host of the primary
DB_HOST=localhost
DB_PORT=5432

# === Cache ===
REDIS_URL=redis://localhost
";

    fn align(reference: &str, target: &str) -> Plan {
        Reference::parse(reference).unwrap().align(target).unwrap()
    }

    fn errors(reference: &str, target: &str) -> Vec<LayoutError> {
        Reference::parse(reference)
            .unwrap()
            .align(target)
            .unwrap_err()
    }

    #[test]
    fn orders_and_groups_like_the_reference_and_keeps_values() {
        let p = align(REF, "REDIS_URL=redis://dev\nDB_PORT=5433\nDB_HOST=db.dev\n");
        assert_eq!(
            p.output,
            "\
# App settings

# === Database ===
# Host of the primary
DB_HOST=db.dev
DB_PORT=5433

# === Cache ===
REDIS_URL=redis://dev
"
        );
        assert!(p.changed);
        assert!(p.missing.is_empty() && p.extra.is_empty());
        let moved: Vec<&str> = p.moved.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(moved.len(), 2, "{moved:?}");
    }

    #[test]
    fn every_ordering_of_a_target_gives_the_same_file() {
        let lines = ["DB_HOST=a", "DB_PORT=b", "REDIS_URL=c"];
        let perms = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let outs: HashSet<String> = perms
            .iter()
            .map(|p| {
                let t: String = p.iter().map(|&i| format!("{}\n", lines[i])).collect();
                align(REF, &t).output
            })
            .collect();
        assert_eq!(outs.len(), 1, "{outs:?}");
    }

    #[test]
    fn assignments_are_moved_byte_for_byte() {
        let target = concat!(
            "REDIS_URL=\"redis://u:p@h:6379/0?a=b#frag\" # inline note\n",
            "DB_PORT='5432'\n",
            "export DB_HOST=\"tab\\there \\\"q\\\" \\$NOT ${DB_PORT} $HOME\"\n",
        );
        let p = align(REF, target);
        for line in target.lines() {
            assert!(p.output.contains(&format!("{line}\n")), "{line}");
        }
    }

    #[test]
    fn a_multiline_value_moves_whole() {
        let target = "DB_PORT=1\nREDIS_URL=\"line1\n# not a comment\n\nline4\"\nDB_HOST='-----BEGIN\nKEY\n-----END'\n";
        let p = align(REF, target);
        assert!(
            p.output
                .contains("DB_HOST='-----BEGIN\nKEY\n-----END'\nDB_PORT=1\n")
        );
        assert!(
            p.output
                .ends_with("REDIS_URL=\"line1\n# not a comment\n\nline4\"\n")
        );
        assert_eq!(p.target_comments, 0, "inside a value is not a comment");
    }

    #[test]
    fn missing_keys_get_a_placeholder_and_extras_go_last_in_order() {
        let p = align(REF, "ZED=1\nDB_HOST=h\nALPHA=2\n");
        assert_eq!(
            p.output,
            "\
# App settings

# === Database ===
# Host of the primary
DB_HOST=h
# DB_PORT=

# === Cache ===
# REDIS_URL=

# Environment-specific settings
ZED=1
ALPHA=2
"
        );
        let missing: Vec<_> = p.missing.iter().map(|a| (a.key.as_str(), a.line)).collect();
        assert_eq!(missing, [("DB_PORT", 6), ("REDIS_URL", 9)]);
        let extra: Vec<_> = p
            .extra
            .iter()
            .map(|m| (m.key.as_str(), m.from_line, m.to_line))
            .collect();
        assert_eq!(extra, [("ZED", 1, 12), ("ALPHA", 3, 13)]);
        let again = align(REF, &p.output);
        assert!(!again.changed && again.output == p.output);
    }

    #[test]
    fn reference_values_are_never_copied() {
        let p = align("A=reference-only-value # ref note\n", "");
        assert_eq!(p.output, "# A=\n");
    }

    #[test]
    fn target_comments_attach_down_or_stay_after_their_key() {
        let target = "\
# preamble only here

REDIS_URL=r
# about the port
DB_PORT=p

# standalone after port

# === Database ===
DB_HOST=h
# trailing
";
        let p = align(REF, target);
        assert_eq!(
            p.output,
            "\
# preamble only here

# App settings

# === Database ===
# Host of the primary
DB_HOST=h

# trailing

# about the port
DB_PORT=p

# standalone after port

# === Cache ===
REDIS_URL=r
"
        );
        assert_eq!(p.target_comments, 4);
        assert!(!align(REF, &p.output).changed, "idempotent");
    }

    #[test]
    fn a_copied_heading_is_not_duplicated() {
        let target = "# === Database ===\nDB_PORT=1\nDB_HOST=2\n";
        let p = align(REF, target);
        assert_eq!(p.output.matches("# === Database ===").count(), 1);
        assert_eq!(p.target_comments, 0);
    }

    #[test]
    fn more_copies_of_a_reference_comment_than_the_reference_is_refused() {
        let e = errors(REF, "# === Cache ===\nA=1\n# === Cache ===\nB=1\n");
        assert_eq!(
            e,
            [LayoutError::AmbiguousComment {
                line: 1,
                here: 2,
                reference: 1
            }]
        );
    }

    #[test]
    fn crlf_and_missing_final_newline_are_kept() {
        let p = align(REF, "REDIS_URL=r\r\nDB_HOST=h");
        assert!(p.output.contains("\r\n") && !p.output.replace("\r\n", "").contains('\n'));
        assert!(p.output.ends_with("REDIS_URL=r"), "{:?}", p.output);
        assert!(p.output.contains("DB_HOST=h\r\n# DB_PORT=\r\n"));
        assert!(!align(REF, &p.output).changed);

        let p = align(REF, "DB_HOST=h");
        assert!(!p.output.ends_with('\n') && !p.output.contains('\r'));
    }

    #[test]
    fn mixed_line_endings_are_refused() {
        assert_eq!(
            errors(REF, "A=1\r\nB=2\n"),
            [LayoutError::MixedLineEndings { crlf: 1, lf: 1 }]
        );
    }

    #[test]
    fn a_bom_is_kept() {
        let p = align(REF, "\u{feff}DB_HOST=h\n");
        assert!(p.output.starts_with("\u{feff}# App settings\n"));
        assert!(!align(REF, &p.output).changed);
    }

    #[test]
    fn duplicates_and_malformed_lines_are_errors_without_content() {
        let secret = "SYNTHETIC_MARKER_Zq9";
        let target =
            format!("A=1\nA=2\n{secret}/x=1\n{secret}\nB=\"{secret}\" {secret}\nC=\"{secret}\n");
        let e = errors(REF, &target);
        let text: Vec<String> = e.iter().map(ToString::to_string).collect();
        assert_eq!(e.len(), 5, "{text:?}");
        assert!(
            matches!(&e[4], LayoutError::Duplicate { key, lines } if key == "A" && *lines == [1, 2])
        );
        for t in &text {
            assert!(!t.contains(secret), "{t}");
        }
        assert!(Reference::parse(&target).is_err(), "the reference too");
    }

    #[test]
    fn a_missing_key_stays_absent_for_every_reader() {
        let p = align("A=\nB=template\n", "A=1\n");
        assert_eq!(p.output, "A=1\n# B=\n");
        let doc = parser().parse_document(&p.output);
        assert_eq!(doc.entries.len(), 1, "a comment, not an assignment");
        assert!(!align("A=\nB=template\n", &p.output).changed, "idempotent");
    }

    #[test]
    fn a_placeholder_goes_when_the_key_gets_a_value() {
        let reference = "A=\nB=\n";
        let first = align(reference, "A=1\n").output;
        // `sync --direction reverse` and setup scripts append at the end.
        let p = align(reference, &format!("{first}B=2\n"));
        assert_eq!(p.output, "A=1\nB=2\n");
        assert!(p.missing.is_empty());
        assert_eq!(p.target_comments, 0, "the placeholder was never the file's");
    }

    #[test]
    fn only_the_exact_placeholder_of_a_reference_key_is_evnxs() {
        // A value in it, or a key the reference lacks: an ordinary comment, kept.
        let p = align("A=\nB=\n", "# A=old\n# C=\nA=1\nB=2\n");
        assert_eq!(p.output, "# A=old\n# C=\nA=1\nB=2\n");
        assert_eq!(p.target_comments, 2);
    }

    #[test]
    fn copies_of_a_placeholder_are_not_ambiguous() {
        let p = align("A=\nB=\n", "# B=\nA=1\n# B=\n# B=\n");
        assert_eq!(p.output, "A=1\n# B=\n");
    }

    #[test]
    fn a_reference_comment_that_looks_like_a_placeholder_is_stable() {
        let reference = "# A=\nA=\nB=\n";
        let p = align(reference, "B=1\n");
        assert_eq!(p.output, "# A=\n# A=\nB=1\n");
        assert!(!align(reference, &p.output).changed, "idempotent");
        let filled = align(reference, "A=2\nB=1\n");
        assert_eq!(filled.output, "# A=\nA=2\nB=1\n");
    }

    #[test]
    fn the_extras_heading_goes_when_the_extras_do() {
        let first = align("A=\n", "B=2\nA=1\n");
        assert_eq!(
            first.output,
            "A=1\n\n# Environment-specific settings\nB=2\n"
        );
        // B has since been added to the reference: no extras, no heading.
        let p = align("A=\nB=\n", &first.output);
        assert_eq!(p.output, "A=1\nB=2\n");
        assert!(p.extra.is_empty() && p.moved.is_empty());
    }

    #[test]
    fn an_aligned_file_is_unchanged() {
        let p = align(REF, REF);
        assert!(!p.changed && p.moved.is_empty() && p.missing.is_empty());
    }

    #[test]
    fn longest_increasing_prefers_the_earliest() {
        let s = longest_increasing(&[3, 1, 2]);
        assert_eq!(s, HashSet::from([1, 2]));
        assert!(longest_increasing(&[]).is_empty());
    }
}
