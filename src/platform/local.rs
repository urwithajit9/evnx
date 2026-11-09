//! The local `.env` file a platform command compares with or pulls into.
//!
//! Which file: `--file`, else `.env.<environment>` when it exists, else `.env`,
//! the rule `cloud push` and `cloud pull` use. The run always says which.
//!
//! Editing keeps every byte it does not need to change: comments, blank lines,
//! order, `export` prefixes, a byte-order mark, CRLF or LF per line, and the
//! trailing comment on a line whose value is replaced. A new key goes at the
//! end. The result is read back and compared with what was intended before
//! anything is written.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use zeroize::Zeroizing;

use crate::core::parser::{Document, Entry, Parser, ParserConfig};

/// How the file was chosen, so the command can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// `--file <path>`.
    Explicit,
    /// `.env.<environment>`, because it exists.
    MatchedEnvironment,
    /// Plain `.env`.
    Default,
}

impl Choice {
    /// The parenthesised note printed after the file name.
    pub fn note(self) -> &'static str {
        match self {
            Choice::Explicit => "--file",
            Choice::MatchedEnvironment => "matched the environment",
            Choice::Default => "default",
        }
    }
}

/// Pick the file. A derived name that does not exist falls back to `.env`: most
/// projects have only that, and the caller reports the choice every time.
pub fn resolve(dir: &Path, explicit: Option<&Path>, environment: &str) -> (PathBuf, Choice) {
    if let Some(p) = explicit {
        return (p.to_path_buf(), Choice::Explicit);
    }
    let derived = dir.join(format!(".env.{environment}"));
    if derived.is_file() {
        return (derived, Choice::MatchedEnvironment);
    }
    (dir.join(".env"), Choice::Default)
}

/// A local file, read.
pub struct LocalFile {
    /// Where it is (or would be).
    pub path: PathBuf,
    /// Its text; `None` when the file does not exist.
    pub text: Option<Zeroizing<String>>,
    /// Every entry, with spans.
    pub doc: Document,
    /// One value per key, the last assignment winning.
    pub vars: IndexMap<String, String>,
}

fn parser() -> Parser {
    Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    })
}

/// Read `path`. Missing is not an error here (`text` is `None`); a file the
/// parser refuses is, because where an entry ends is then a guess and nothing
/// may be edited.
pub fn read(path: &Path) -> Result<LocalFile> {
    let text = match std::fs::read(path) {
        Ok(b) => Some(Zeroizing::new(String::from_utf8(b).map_err(|_| {
            anyhow::anyhow!("{} is not UTF-8 text", path.display())
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let doc = parser().parse_document(text.as_deref().map_or("", String::as_str));
    if let Some(p) = doc.problems.first() {
        // `describe` names the line and the reason, never the line's text.
        bail!(
            "{} has a line evnx cannot read:\n  {}\n  Fix it first; `evnx validate` lists every problem.",
            path.display(),
            p.describe()
        );
    }
    let vars = doc.vars();
    Ok(LocalFile {
        path: path.to_path_buf(),
        text,
        doc,
        vars,
    })
}

/// Change `text`: give each `(key, value)` in `set` its new value in place (the
/// last assignment of a key that is already there), and append each in `add`.
///
/// Fails, changing nothing, if reading the result back does not give exactly the
/// old values with these changes.
pub fn edit(
    text: &str,
    doc: &Document,
    set: &[(&str, &str)],
    add: &[(&str, &str)],
) -> Result<Zeroizing<String>> {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let eol = if lines.iter().any(|l| l.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };

    // Which entry each `set` key replaces: its last assignment.
    let mut replace: IndexMap<usize, (&Entry, &str)> = IndexMap::new();
    for (key, value) in set {
        let Some(entry) = doc.entries.iter().rev().find(|e| e.key == *key) else {
            bail!("internal: {key} is not in the file it should change");
        };
        replace.insert(entry.span.line, (entry, value));
    }

    let mut out = Zeroizing::new(String::with_capacity(text.len() + 64));
    out.push_str(bom);
    let mut i = 0;
    while i < lines.len() {
        let line_no = i + 1;
        if let Some((entry, value)) = replace.get(&line_no) {
            let last = lines[entry.span.end_line.min(lines.len()) - 1];
            let ending = line_ending(last);
            out.push_str(&rewrite(entry, lines[i], value));
            out.push_str(ending);
            i = entry.span.end_line.max(line_no);
        } else {
            out.push_str(lines[i]);
            i += 1;
        }
    }
    if !add.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(eol);
        }
        for (key, value) in add {
            out.push_str(&crate::core::envfile::line(key, value));
            out.push_str(eol);
        }
    }

    // Read it back: the old values, with exactly these changes.
    let mut expected = doc.vars();
    for (k, v) in set.iter().chain(add.iter()) {
        expected.insert((*k).to_string(), (*v).to_string());
    }
    let reread = parser().parse_document(&out);
    if !reread.problems.is_empty() || reread.vars() != expected {
        bail!("the edited file would not read back as intended, so nothing was written");
    }
    Ok(out)
}

fn line_ending(line: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

/// The entry's first line with its value replaced: same indentation, same
/// `export`, and the same trailing comment.
fn rewrite(entry: &Entry, first_line: &str, value: &str) -> String {
    let indent: String = first_line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let export = if entry.exported { "export " } else { "" };
    let comment = if entry.span.line == entry.span.end_line {
        trailing_comment(first_line.trim_end_matches(['\r', '\n']))
    } else {
        String::new()
    };
    format!(
        "{indent}{export}{}{comment}",
        crate::core::envfile::line(&entry.key, value)
    )
}

/// The ` # comment` after the value on a one-line entry, spacing included, or
/// nothing. A `#` is a comment only after whitespace, outside quotes.
fn trailing_comment(line: &str) -> String {
    let Some(eq) = line.find('=') else {
        return String::new();
    };
    let v = line[eq + 1..].trim_start();
    let tail = match v.chars().next() {
        Some(q @ ('"' | '\'' | '`')) => {
            let mut escaped = false;
            let mut close = None;
            for (i, c) in v.char_indices().skip(1) {
                if q == '"' && escaped {
                    escaped = false;
                } else if q == '"' && c == '\\' {
                    escaped = true;
                } else if c == q {
                    close = Some(i + c.len_utf8());
                    break;
                }
            }
            close.map(|c| &v[c..])
        }
        _ => {
            let bytes = v.as_bytes();
            v.match_indices('#')
                .find(|(at, _)| *at > 0 && bytes[at - 1].is_ascii_whitespace())
                .map(|(at, _)| &v[v[..at].trim_end().len()..])
        }
    };
    match tail {
        Some(t) if t.trim_start().starts_with('#') => t.trim_end().to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_text(text: &str, set: &[(&str, &str)], add: &[(&str, &str)]) -> String {
        let doc = parser().parse_document(text);
        edit(text, &doc, set, add).unwrap().to_string()
    }

    #[test]
    fn a_value_changes_in_place_and_everything_else_stays() {
        let text = "# header\nA=1\n\n# about B\nexport B=\"old\" # keep me\nC=3\n";
        let out = edit_text(text, &[("B", "new value")], &[("D", "4")]);
        assert_eq!(
            out,
            "# header\nA=1\n\n# about B\nexport B=\"new value\" # keep me\nC=3\nD=4\n"
        );
    }

    #[test]
    fn crlf_bom_and_a_missing_final_newline_are_kept() {
        let text = "\u{feff}A=1\r\nB=2";
        let out = edit_text(text, &[("A", "9")], &[("C", "3")]);
        assert_eq!(out, "\u{feff}A=9\r\nB=2\r\nC=3\r\n");
    }

    #[test]
    fn the_last_of_a_duplicated_key_is_the_one_replaced() {
        let out = edit_text("A=1\nB=x\nA=2\n", &[("A", "7")], &[]);
        assert_eq!(out, "A=1\nB=x\nA=7\n");
    }

    #[test]
    fn a_multiline_value_is_replaced_whole() {
        let text = "A=\"line1\nline2\"\nB=2\n";
        let out = edit_text(text, &[("A", "one")], &[]);
        assert_eq!(out, "A=one\nB=2\n");
    }

    #[test]
    fn values_that_need_quoting_read_back_exactly() {
        let out = edit_text("A=1\n", &[("A", "p@ss #1 $x")], &[("B", "a\"b")]);
        let doc = parser().parse_document(&out);
        assert_eq!(doc.vars()["A"], "p@ss #1 $x");
        assert_eq!(doc.vars()["B"], "a\"b");
    }

    #[test]
    fn a_hash_inside_an_unquoted_value_is_not_a_comment() {
        assert_eq!(trailing_comment("A=Tr0ub#3"), "");
        assert_eq!(trailing_comment("A=v  # why"), "  # why");
        assert_eq!(trailing_comment("A=\"a # b\" # why"), " # why");
        assert_eq!(trailing_comment("A='x'"), "");
    }

    #[test]
    fn adding_to_an_empty_or_new_file_works() {
        assert_eq!(edit_text("", &[], &[("A", "1")]), "A=1\n");
    }

    #[test]
    fn the_file_is_chosen_like_cloud_push() {
        let d = tempfile::tempdir().unwrap();
        let (p, c) = resolve(d.path(), None, "production");
        assert_eq!(
            (p.file_name().unwrap().to_str().unwrap(), c),
            (".env", Choice::Default)
        );
        std::fs::write(d.path().join(".env.production"), "").unwrap();
        let (p, c) = resolve(d.path(), None, "production");
        assert_eq!(p.file_name().unwrap(), ".env.production");
        assert_eq!(c, Choice::MatchedEnvironment);
        let (p, c) = resolve(d.path(), Some(Path::new("x.env")), "production");
        assert_eq!((p.to_str().unwrap(), c), ("x.env", Choice::Explicit));
    }

    #[test]
    fn an_unreadable_line_stops_before_anything_can_change() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".env");
        std::fs::write(&f, "A=1\n=oops\n").unwrap();
        let e = read(&f).err().expect("a file the parser refuses");
        assert!(e.to_string().contains("cannot read"), "{e}");
    }
}
