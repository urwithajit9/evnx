//! Cutting entries out of a `.env` file's text, byte for byte (R-1 … R-9).
//!
//! ⛔ The only code in evnx that deletes entries from an env file
//! (`tests/architecture.rs::r1_only_remove_deletes_env_entries`). It works on
//! whole physical lines found by [`Document`] spans, never on a regex: a
//! multiline value, a duplicate or an `export` prefix is what a line-based
//! `sed` gets wrong.

use std::collections::BTreeSet;

use anyhow::{bail, Result};

use crate::core::parser::{Document, Parser, ParserConfig};

/// One entry taken out: the key, the line its assignment starts on, and every
/// physical line removed with it (its attached comment included), 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub name: String,
    pub line: usize,
    pub lines: Vec<usize>,
}

/// The result of [`cut_entries`]: the new text and what left it.
#[derive(Debug, Clone)]
pub struct Cut {
    pub text: String,
    pub removed: Vec<Removed>,
}

/// Read `content` the way `remove` reads it: line pass only. Expansion is a
/// reading of values, and a removal never looks at a value.
pub fn document(content: &str) -> Document {
    Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    })
    .parse_document(content)
}

/// Every key the file assigns, in file order, duplicates included.
pub fn keys(doc: &Document) -> Vec<String> {
    doc.entries.iter().map(|e| e.key.clone()).collect()
}

/// Remove every entry whose key is in `names` from `content`.
///
/// Kept exactly: every other byte, CRLF or LF per line, a byte-order mark, and
/// whether the file ends with a newline (R-6, R-7, R-8). `label` names the file
/// in errors. Fails, changing nothing, when the file has a line the parser
/// refuses: where an entry ends is then a guess (R-13), and so is when the
/// result would still parse the same.
pub fn cut_entries(content: &str, names: &BTreeSet<String>, label: &str) -> Result<Cut> {
    let doc = document(content);
    if let Some(p) = doc.problems.first() {
        // `describe` names the line and the reason, never the line's text.
        bail!(
            "{label} has a line evnx cannot read, so where an entry ends is not certain \
             and nothing was changed:\n  {}\n  Fix it first; `evnx validate` lists every problem.",
            p.describe()
        );
    }
    let (bom, body) = match content.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", content),
    };
    // `split_inclusive` keeps each line's own ending, and numbers lines the way
    // the parser's `str::lines` does.
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let comment_lines: BTreeSet<usize> = doc.comments.iter().map(|c| c.line).collect();
    let is_header = |n: usize| {
        doc.comments
            .iter()
            .find(|c| c.line == n)
            .is_some_and(|c| is_group_header(&c.text))
    };

    let mut cut = vec![false; lines.len() + 1];
    let mut removed = Vec::new();
    for e in doc.entries.iter().filter(|e| names.contains(&e.key)) {
        let mut taken: Vec<usize> = (e.span.line..=e.span.end_line).collect();
        let above = e.span.line.saturating_sub(1);
        if above >= 1 && comment_lines.contains(&above) {
            let text = &doc.comments.iter().find(|c| c.line == above).unwrap().text;
            // R-9, conservative: one comment line, the shape `add` and `init`
            // write — alone above the key, or under a group header.
            let alone = above == 1 || !comment_lines.contains(&(above - 1)) || is_header(above - 1);
            if alone && !is_group_header(text) && !is_commented_assignment(text) {
                taken.insert(0, above);
            }
        }
        for &n in &taken {
            cut[n] = true;
        }
        removed.push(Removed {
            name: e.key.clone(),
            line: e.span.line,
            lines: taken,
        });
    }

    let mut text: String = bom.to_string();
    for (i, l) in lines.iter().enumerate() {
        if !cut[i + 1] {
            text.push_str(l);
        }
    }
    // R-8: a file that ended without a newline still does, when its last line
    // was the one removed.
    if !body.is_empty() && !body.ends_with('\n') && cut[lines.len()] {
        if let Some(t) = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
        {
            text.truncate(t.len());
        }
    }

    // S-3: the result reads as the old keys minus the removed ones, or nothing
    // is written.
    let after = document(&text);
    let expected: Vec<String> = keys(&doc)
        .into_iter()
        .filter(|k| !names.contains(k))
        .collect();
    if !after.is_clean() || keys(&after) != expected {
        bail!("removing from {label} would change more than the entries asked for, so nothing was written");
    }
    Ok(Cut { text, removed })
}

/// A comment that heads a group rather than describing one variable: never
/// removed, even when the group is left empty (R-9).
fn is_group_header(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("──")
        || t.starts_with("Added by ")
        || t.starts_with("Generated by ")
        || t.starts_with("[ADDED]")
}

/// `# OTHER=1` or `# TODO: OTHER=…`: a variable switched off, not a description.
fn is_commented_assignment(text: &str) -> bool {
    let t = text.trim();
    let t = t.strip_prefix("TODO:").map(str::trim_start).unwrap_or(t);
    let t = t.strip_prefix("export ").unwrap_or(t);
    let name: String = t
        .chars()
        .take_while(|c| c.is_alphanumeric() || "_.-".contains(*c))
        .collect();
    !name.is_empty() && t[name.len()..].trim_start().starts_with('=')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> BTreeSet<String> {
        n.iter().map(|s| s.to_string()).collect()
    }
    fn cut(content: &str, n: &[&str]) -> Cut {
        cut_entries(content, &names(n), "t").unwrap()
    }

    #[test]
    fn a1_a2_a3_only_the_span_goes_and_the_ending_is_kept() {
        assert_eq!(cut("A=1\nB=2\nC=3\n", &["B"]).text, "A=1\nC=3\n");
        assert_eq!(cut("FIRST=1\nB=2\n", &["FIRST"]).text, "B=2\n");
        assert_eq!(cut("A=1\nLAST=2", &["LAST"]).text, "A=1");
        assert_eq!(cut("A=1\nB=2", &["A"]).text, "B=2");
    }

    #[test]
    fn a4_to_a8_forms_the_parser_accepts() {
        assert_eq!(cut("export X=1\nY=2\n", &["X"]).text, "Y=2\n");
        assert_eq!(cut("SPACED = loose\nY=2\n", &["SPACED"]).text, "Y=2\n");
        assert_eq!(
            cut("A=1\nQ=\"has # hash and = sign\"\nB=2\n", &["Q"]).text,
            "A=1\nB=2\n"
        );
        let c = cut("M=\"line one\nline two\"\nNEXT=1\n", &["M"]);
        assert_eq!(c.text, "NEXT=1\n");
        assert_eq!(c.removed[0].lines, vec![1, 2]);
        let c = cut("DUP=first\nA=1\nDUP=second\n", &["DUP"]);
        assert_eq!(c.text, "A=1\n");
        assert_eq!(
            c.removed.iter().map(|r| r.line).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn a9_a11_exact_names_only() {
        let c = cut("PORT=1\nPORT_EXTRA=2\nMY_PORT=3\n", &["PORT"]);
        assert_eq!(c.text, "PORT_EXTRA=2\nMY_PORT=3\n");
        let c = cut("# DROP was removed\nX=\"DROP=1\"\n", &["DROP"]);
        assert!(c.removed.is_empty());
        assert_eq!(c.text, "# DROP was removed\nX=\"DROP=1\"\n");
    }

    #[test]
    fn a12_a13_a14_empty_crlf_bom() {
        assert_eq!(cut("ONLY=1\n", &["ONLY"]).text, "");
        assert_eq!(cut("A=1\r\nB=2\r\nC=3\r\n", &["B"]).text, "A=1\r\nC=3\r\n");
        assert_eq!(cut("A=1\r\nB=2", &["B"]).text, "A=1");
        assert_eq!(cut("\u{feff}B=1\nC=2\n", &["B"]).text, "\u{feff}C=2\n");
    }

    #[test]
    fn a15_a16_attached_comments_as_add_writes_them() {
        // `add custom` with a category: a blank line, the comment, the key.
        let c = cut(
            "\n# ── Custom Variables ──\n\n# [Security] A test variable (required)\nTEST_WEAK=x\nKEEP=1\n\n# Added by evnx add custom on 2026-11-05 10:00\n",
            &["TEST_WEAK"],
        );
        assert_eq!(
            c.text,
            "\n# ── Custom Variables ──\n\nKEEP=1\n\n# Added by evnx add custom on 2026-11-05 10:00\n"
        );
        assert_eq!(c.removed[0].lines, vec![4, 5]);
        // The formatter: a description directly under the group header.
        let c = cut(
            "# ── Database ──\n# Postgres URL (required)\nDB=x\n",
            &["DB"],
        );
        assert_eq!(c.text, "# ── Database ──\n");
        // A header straight above the key stays.
        let c = cut("# ── Custom Variables ──\nA=1\nB=2\n", &["A"]);
        assert_eq!(c.text, "# ── Custom Variables ──\nB=2\n");
        // A person's comment block of two lines stays whole.
        let c = cut("# why we need this:\n# a long story\nA=1\n", &["A"]);
        assert_eq!(c.text, "# why we need this:\n# a long story\n");
        // A commented-out variable is not a description.
        let c = cut("# OLD_A=1\nA=1\n", &["A"]);
        assert_eq!(c.text, "# OLD_A=1\n");
        // A blank line between detaches it.
        let c = cut("# about A\n\nA=1\n", &["A"]);
        assert_eq!(c.text, "# about A\n\n");
    }

    #[test]
    fn a_line_the_parser_refuses_stops_the_edit() {
        let e = cut_entries("A=1\nnot a line\n", &names(&["A"]), ".env").unwrap_err();
        assert!(e.to_string().contains("Line 2"), "{e}");
        assert!(!e.to_string().contains("not a line"), "{e}");
        assert!(cut_entries("A=\"open\nB=2\n", &names(&["A"]), "t").is_err());
    }

    /// F-1, F-2: for generated files, every line outside the removed spans is
    /// byte-identical, and the keys left are the old keys minus the removed.
    #[test]
    fn f1_f2_generated_files_keep_every_other_byte() {
        let pieces = [
            "export EX=1",
            "DUP=a",
            "DUP=b",
            "SP = x",
            "Q=\"a # b = c\"",
            "M=\"l1\nl2\"",
            "# a comment",
            "",
            "EMPTY=",
            "S='single'",
            "Z=1",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let n = 1 + (next() % 12) as usize;
            let crlf = next() % 2 == 0;
            let nl = if crlf { "\r\n" } else { "\n" };
            let mut text: String = if next() % 4 == 0 {
                "\u{feff}".into()
            } else {
                String::new()
            };
            for i in 0..n {
                let p = pieces[(next() % pieces.len() as u64) as usize].replace('\n', nl);
                text.push_str(&p);
                if i + 1 < n || next() % 2 == 0 {
                    text.push_str(nl);
                }
            }
            let doc = document(&text);
            if !doc.is_clean() {
                continue;
            }
            let all = keys(&doc);
            let pick: BTreeSet<String> = all.iter().filter(|_| next() % 2 == 0).cloned().collect();
            let c = cut_entries(&text, &pick, "t").unwrap();
            let gone: BTreeSet<usize> = c.removed.iter().flat_map(|r| r.lines.clone()).collect();
            let body = text.trim_start_matches('\u{feff}');
            let kept: String = body
                .split_inclusive('\n')
                .enumerate()
                .filter(|(i, _)| !gone.contains(&(i + 1)))
                .map(|(_, l)| l)
                .collect();
            let got = c.text.trim_start_matches('\u{feff}');
            // R-8: when the unterminated last line went, one line ending goes too.
            let one_less = kept
                .strip_suffix("\r\n")
                .or_else(|| kept.strip_suffix('\n'))
                .unwrap_or(&kept);
            assert!(
                kept == got || one_less == got,
                "{text:?} minus {pick:?}: {got:?}"
            );
            assert_eq!(text.starts_with('\u{feff}'), c.text.starts_with('\u{feff}'));
            let left: Vec<String> = all.into_iter().filter(|k| !pick.contains(k)).collect();
            assert_eq!(keys(&document(&c.text)), left);
            assert!(!(c.text.contains("\r\n") && c.text.replace("\r\n", "").contains('\n')));
        }
    }

    /// F-4: garbage never panics; it is cut or refused.
    #[test]
    fn f4_garbage_never_panics() {
        let alphabet: Vec<char> = "AB=\"'\n\r# x\\$`{}\u{feff}".chars().collect();
        let mut seed: u64 = 42;
        for _ in 0..2000 {
            let mut s = String::new();
            for _ in 0..(seed % 40) {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            let _ = cut_entries(&s, &names(&["A", "B", "AB"]), "t");
        }
    }

    #[test]
    fn commented_assignments() {
        assert!(is_commented_assignment(" OLD=1"));
        assert!(is_commented_assignment(" TODO: NODE_VERSION=22"));
        assert!(!is_commented_assignment(" [Security] API key (required)"));
    }
}
