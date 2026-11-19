//! Changing one variable in a `.env` file's text, byte for byte (`evnx set`).
//!
//! The one place evnx rewrites a single value in place. It works on the
//! physical lines the parser reports ([`Document`] spans), never on a regex:
//!
//! * the line keeps its position, its leading whitespace, its `export` prefix,
//!   the spacing around `=`, its quote style and whatever follows the value
//!   (an inline `# comment`);
//! * every other line stays as it is, CRLF or LF, with or without a final
//!   newline, with or without a byte-order mark;
//! * a name that is not there is appended after the last line.
//!
//! The result is read again before it is returned: every other entry must read
//! as it did, and the new value must read back as exactly what was asked for.
//! Anything else is an error and nothing is written.
//!
//! ⛔ A value is never part of an error message: this module handles secrets.

use anyhow::{Result, bail};

use super::envfile;
use super::parser::{
    Document, Entry, Parser, ParserConfig, closing_quote_index, is_env_key, strip_inline_comment,
};

/// What [`set`] decided.
#[derive(Debug, Clone)]
pub struct Edit {
    /// The whole new text. Equal to the old text when nothing changes.
    pub text: String,
    /// The name was not in the file and was appended.
    pub added: bool,
    /// What the name held before, as written (unexpanded); `None` when added.
    pub old: Option<String>,
    /// The line the assignment is on in the new text, 1-based.
    pub line: usize,
    /// How many assignments of the name the file had. More than one: the last
    /// is the one that counts, and the one changed.
    pub assignments: usize,
}

impl Edit {
    /// Whether the new text differs from the old one.
    pub fn changed(&self, old_text: &str) -> bool {
        self.text != old_text
    }
}

/// What the name holds now.
#[derive(Debug, Clone)]
pub struct Current {
    /// The value as written, escapes resolved, `${}` left alone.
    pub value: String,
    /// The line its assignment starts on.
    pub line: usize,
    /// How many assignments of the name the file has.
    pub assignments: usize,
}

fn document(content: &str) -> Document {
    Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    })
    .parse_document(content)
}

/// `Ok(())` when `name` can be a variable name in a `.env` file.
pub fn check_name(name: &str) -> Result<()> {
    if !is_env_key(name) {
        bail!(
            "that is not a variable name: names hold letters, digits, _ . - [ and ] \
             (for example PORT or DATABASE_URL)"
        );
    }
    Ok(())
}

fn read(content: &str, label: &str) -> Result<Document> {
    let doc = document(content);
    if let Some(p) = doc.problems.first() {
        // `describe` names the line and the reason, never the line's text.
        bail!(
            "{label} has a line evnx cannot read, so where an entry ends is not certain \
             and nothing was changed:\n  {}\n  Fix it first; `evnx validate` lists every problem.",
            p.describe()
        );
    }
    Ok(doc)
}

/// What `name` holds in `content`, or `None` when it is not assigned.
pub fn current(content: &str, name: &str, label: &str) -> Result<Option<Current>> {
    let doc = read(content, label)?;
    let all: Vec<&Entry> = doc.entries.iter().filter(|e| e.key == name).collect();
    Ok(all.last().map(|e| Current {
        value: plain(e.value.expose()),
        line: e.span.line,
        assignments: all.len(),
    }))
}

/// Every name the file assigns, in the order each first appears, with what it
/// holds now (the last assignment counts, as in [`current`]). Unexpanded.
pub fn assignments(content: &str, label: &str) -> Result<Vec<(String, super::secret::Secret)>> {
    let doc = read(content, label)?;
    let mut order: Vec<&str> = Vec::new();
    for e in &doc.entries {
        if !order.contains(&e.key.as_str()) {
            order.push(&e.key);
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|name| {
            doc.entries.iter().rev().find(|e| e.key == name).map(|e| {
                (
                    name.to_string(),
                    super::secret::Secret::new(plain(e.value.expose())),
                )
            })
        })
        .collect())
}

/// `content` with every `(name, value)` set, one after the other: what `set
/// --fill` writes. Each step is checked as [`set`] checks it, so the file only
/// changes in those values.
pub fn set_many(
    content: &str,
    values: &[(String, super::secret::Secret)],
    label: &str,
) -> Result<String> {
    let mut text = content.to_string();
    for (name, value) in values {
        text = set(&text, name, value.expose(), label)?.text;
    }
    Ok(text)
}

/// A value without the parser's internal marker for an escaped `$`.
fn plain(value: &str) -> String {
    value.replace('\u{fdd0}', "")
}

/// `content` with `name` set to `value`.
///
/// `label` names the file in errors.
pub fn set(content: &str, name: &str, value: &str, label: &str) -> Result<Edit> {
    check_name(name)?;
    let doc = read(content, label)?;
    let (bom, body) = match content.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", content),
    };
    let assignments = doc.entries.iter().filter(|e| e.key == name).count();
    let target = doc.entries.iter().rposition(|e| e.key == name);

    let (text, segment, line) = match target {
        Some(i) => {
            let (text, segment) = replace(body, &doc.entries[i], value)?;
            (text, segment, doc.entries[i].span.line)
        }
        None => {
            let (text, segment, line) = append(body, name, value);
            (text, segment, line)
        }
    };
    let text = format!("{bom}{text}");

    // The result reads as the old file with this one value changed, or
    // nothing is written.
    let after = document(&text);
    let same_shape = after.is_clean()
        && after.entries.len() == doc.entries.len() + usize::from(target.is_none())
        && doc.entries.iter().enumerate().all(|(i, old)| {
            let new = &after.entries[i];
            new.key == old.key
                && new.exported == old.exported
                && (Some(i) == target || (new.value == old.value && new.quote == old.quote))
        });
    // The segment alone, read the way a process would get it: expanded.
    let reads_back = Parser::default()
        .parse_content(&format!("K={segment}"))
        .map(|f| f.get("K").is_some_and(|v| v == value))
        .unwrap_or(false);
    if !same_shape || !reads_back {
        bail!(
            "changing {name} in {label} would change more than that one value, \
             so nothing was written"
        );
    }
    Ok(Edit {
        text,
        added: target.is_none(),
        old: target.map(|i| plain(doc.entries[i].value.expose())),
        line,
        assignments,
    })
}

/// The ending of the file's first line: CRLF when it has one, else LF.
fn line_ending(body: &str) -> &'static str {
    match body.find('\n') {
        Some(i) if body[..i].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// `body` with the assignment appended; the text, the written value, the line.
fn append(body: &str, name: &str, value: &str) -> (String, String, usize) {
    let ending = line_ending(body);
    let segment = envfile::quote(value).into_owned();
    let mut text = body.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str(ending);
    }
    let line = text.split_inclusive('\n').count() + 1;
    text.push_str(name);
    text.push('=');
    text.push_str(&segment);
    text.push_str(ending);
    (text, segment, line)
}

/// `body` with `entry`'s value replaced; the text and the value as written.
fn replace(body: &str, entry: &Entry, value: &str) -> Result<(String, String)> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let (first_n, last_n) = (entry.span.line, entry.span.end_line);
    if first_n == 0 || last_n > lines.len() || first_n > last_n {
        bail!("could not place the assignment in the file, so nothing was written");
    }
    let first = lines[first_n - 1];
    let Some(eq) = first.find('=') else {
        bail!("could not find the assignment's `=`, so nothing was written");
    };
    let after_eq = &first[eq + 1..];
    let blanks = after_eq
        .len()
        .saturating_sub(after_eq.trim_start_matches(blank).len());
    let value_start = eq + 1 + blanks;
    let rest = &first[value_start..];

    // What follows the old value on its last line: spacing, a comment, the
    // line ending.
    let tail: &str = match rest.chars().next() {
        Some(q @ ('"' | '\'' | '`')) if first_n == last_n => {
            let Some(close) = closing_quote_index(rest, q) else {
                bail!("could not find the closing quote, so nothing was written");
            };
            &rest[close + q.len_utf8()..]
        }
        Some('"' | '\'' | '`') => {
            // A value over several lines closes at the end of its last one.
            let last = lines[last_n - 1];
            &last[last.trim_end().len()..]
        }
        _ => {
            let ending_len = rest.len() - rest.trim_end_matches(['\n', '\r']).len();
            let content = &rest[..rest.len() - ending_len];
            let old_value = strip_inline_comment(content);
            &rest[old_value.len()..]
        }
    };
    let segment = envfile::quote_like(value, entry.quote).into_owned();

    let mut text = String::with_capacity(body.len() + segment.len());
    for l in &lines[..first_n - 1] {
        text.push_str(l);
    }
    text.push_str(&first[..value_start]);
    text.push_str(&segment);
    text.push_str(tail);
    for l in &lines[last_n..] {
        text.push_str(l);
    }
    Ok((text, segment))
}

/// Space or tab: the spacing the parser skips after `=`.
fn blank(c: char) -> bool {
    c.is_whitespace() && c != '\n' && c != '\r'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(content: &str, name: &str, value: &str) -> String {
        set(content, name, value, ".env").unwrap().text
    }

    #[test]
    fn changes_one_line_and_keeps_the_rest() {
        let src = "# top\nA=1\n# about B\nexport B=\"two\" # keep me\nC='3'\n\nD=4";
        assert_eq!(
            run(src, "B", "x y"),
            "# top\nA=1\n# about B\nexport B=\"x y\" # keep me\nC='3'\n\nD=4"
        );
        assert_eq!(run(src, "C", "9"), src.replace("C='3'", "C='9'"));
        assert_eq!(run(src, "D", "5"), src.replace("D=4", "D=5"));
        assert_eq!(run(src, "A", "7"), src.replace("A=1", "A=7"));
    }

    #[test]
    fn keeps_crlf_bom_and_spacing() {
        let src = "\u{feff}A = 1\r\nB=2\r\n";
        assert_eq!(run(src, "A", "9"), "\u{feff}A = 9\r\nB=2\r\n");
        assert_eq!(run(src, "N", "v"), "\u{feff}A = 1\r\nB=2\r\nN=v\r\n");
    }

    #[test]
    fn appends_when_missing_and_adds_the_missing_newline() {
        assert_eq!(run("A=1", "B", "2"), "A=1\nB=2\n");
        assert_eq!(run("A=1\n", "B", "2"), "A=1\nB=2\n");
        assert_eq!(run("", "B", "2"), "B=2\n");
    }

    #[test]
    fn quotes_what_needs_it_and_keeps_the_style_otherwise() {
        assert_eq!(run("A=1\n", "A", "a b"), "A=\"a b\"\n");
        assert_eq!(run("A=1\n", "A", "a$b"), "A='a$b'\n");
        assert_eq!(run("A='x'\n", "A", "it's"), "A=\"it's\"\n");
        assert_eq!(run("A=\"x\"\n", "A", "plain"), "A=\"plain\"\n");
        assert_eq!(run("A=\n", "A", "v"), "A=v\n");
        assert_eq!(run("A=v\n", "A", ""), "A=\n");
    }

    #[test]
    fn a_multiline_value_is_replaced_whole() {
        let src = "A=1\nK=\"l1\nl2\"\nB=2\n";
        assert_eq!(run(src, "K", "one"), "A=1\nK=\"one\"\nB=2\n");
        assert_eq!(
            run("K=1\nB=2\n", "K", "x\ny"),
            "K=1\nB=2\n".replace("K=1", "K=\"x\\ny\"")
        );
    }

    #[test]
    fn the_last_of_two_assignments_is_the_one_changed() {
        let e = set("A=1\nB=2\nA=3\n", "A", "9", ".env").unwrap();
        assert_eq!(e.text, "A=1\nB=2\nA=9\n");
        assert_eq!((e.assignments, e.line, e.old.as_deref()), (2, 3, Some("3")));
    }

    #[test]
    fn assignments_lists_each_name_once_with_its_last_value() {
        let got = assignments("A=1\nB=\nA=3\nexport C=\"x y\"\n", ".env").unwrap();
        let got: Vec<(&str, &str)> = got.iter().map(|(n, v)| (n.as_str(), v.expose())).collect();
        assert_eq!(got, [("A", "3"), ("B", ""), ("C", "x y")]);
    }

    #[test]
    fn set_many_changes_only_those_values() {
        let src = "# top\nA=\nB=keep # note\nC=YOUR_KEY_HERE\n";
        let values = vec![
            ("A".to_string(), super::super::secret::Secret::new("1")),
            ("C".to_string(), super::super::secret::Secret::new("a b")),
        ];
        assert_eq!(
            set_many(src, &values, ".env").unwrap(),
            "# top\nA=1\nB=keep # note\nC=\"a b\"\n".replace("C=\"a b\"", "C=\"a b\"")
        );
    }

    #[test]
    fn an_unreadable_file_is_refused() {
        let err = set("A=1\nnot a line\n", "A", "2", ".env").unwrap_err();
        assert!(err.to_string().contains("Line 2"), "{err}");
    }

    #[test]
    fn a_value_is_never_in_an_error() {
        let err = set("A=\"open\n", "A", "hunter2-secret", ".env").unwrap_err();
        assert!(!err.to_string().contains("hunter2"), "{err}");
        assert!(check_name("bad name").is_err());
    }

    #[test]
    fn a_comment_like_value_after_a_space_is_kept_as_the_tail() {
        assert_eq!(run("A=old   # why\n", "A", "new"), "A=new   # why\n");
        assert_eq!(run("A=#fff\n", "A", "#000"), "A=\"#000\"\n");
    }
}
