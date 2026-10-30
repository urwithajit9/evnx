//! What the `.env` parser reads, pinned file by file (v0.12.0 B2, P0.0).
//!
//! Every file in `tests/fixtures/conformance/` is read four ways — strict and
//! lossy, with expansion on and off — and the reading is compared with
//! `tests/golden/conformance/<file>.txt`. So **any** change to what the parser
//! reads fails here and names the files it changed.
//!
//! The fixtures:
//! - `dd_*` — the 16 cases of github.com/urwithajit9/dotenv-difference, where the
//!   readers disagree;
//! - `edge_*` — the 20 edge files of the 2026-10-30 parser research, checked
//!   against Docker Compose, python-dotenv and Node.
//!
//! # Updating
//!
//! ```bash
//! UPDATE_GOLDEN=1 cargo test --all-features --test parser_conformance
//! git diff tests/golden/conformance/   # every changed line is a changed reading
//! ```
//!
//! ⚠️ A diff here is a change in what evnx reads from someone's file. It is either
//! a bug or a decision (`release/v0.12.0/PARSER-DECISIONS.md`); name which in the
//! commit message.
//!
//! ⛔ The fixtures' bytes are the test — CRLF, a lone CR, a BOM, a NUL, Latin-1 —
//! which is why `.gitattributes` turns every conversion off for them.

use evnx::core::parser::{Document, Parser, ParserConfig};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The four readings of one file, as text. Values are `{:?}`-quoted, so a
/// `\r`, a NUL or a BOM is visible rather than invisible.
fn render(path: &Path) -> String {
    let mut out = String::new();
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        // The commands read through `read_document`, so report what it says.
        Err(_) => {
            let err = Parser::default().read_document(path).unwrap_err();
            let cause = std::error::Error::source(&err)
                .map(|s| format!(": {s}"))
                .unwrap_or_default();
            writeln!(out, "not read: {err}{cause}").unwrap();
            return out;
        }
    };

    let on = Parser::default();
    let off = Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    });
    let with = visible(&reading(&on, &content));
    let without = visible(&reading(&off, &content));
    writeln!(out, "== expansion on").unwrap();
    out.push_str(&with);
    if without == with {
        writeln!(out, "== expansion off: the same").unwrap();
    } else {
        writeln!(out, "== expansion off").unwrap();
        out.push_str(&without);
    }
    out
}

/// Escape what a reviewer cannot see. Values are already `{:?}`-quoted, but an
/// error message quotes a key as written, and a BOM there is invisible.
fn visible(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => c.to_string(),
            c if c.is_control() || c == '\u{feff}' => c.escape_debug().to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn reading(parser: &Parser, content: &str) -> String {
    let mut out = String::new();
    match parser.parse_content_located(content) {
        Ok((vars, lines)) => {
            writeln!(out, "strict: ok").unwrap();
            for (k, v) in &vars {
                writeln!(out, "  L{} {k} = {v:?}", lines[k]).unwrap();
            }
        }
        Err(e) => writeln!(out, "strict: error: {e}").unwrap(),
    }
    lossy(&parser.parse_document(content), &mut out);
    out
}

fn lossy(doc: &Document, out: &mut String) {
    writeln!(out, "lossy:").unwrap();
    for e in &doc.entries {
        let span = if e.span.end_line == e.span.line {
            format!("L{}", e.span.line)
        } else {
            format!("L{}-{}", e.span.line, e.span.end_line)
        };
        let quote = e.quote.map(|q| format!(" quote={q}")).unwrap_or_default();
        let export = if e.exported { " export" } else { "" };
        writeln!(out, "  {span} {} = {:?}{quote}{export}", e.key, e.value).unwrap();
    }
    for c in &doc.comments {
        writeln!(out, "  L{} # {:?}", c.line, c.text).unwrap();
    }
    for p in &doc.problems {
        writeln!(out, "  problem: {}", p.describe()).unwrap();
    }
    // D10: what Compose reads differently top to bottom (B2 P0.7).
    for d in &doc.order_differences {
        let below: Vec<String> = d
            .below
            .iter()
            .map(|b| {
                let above = if b.also_above { " also above" } else { "" };
                format!("{} L{}{above}", b.name, b.line)
            })
            .collect();
        writeln!(
            out,
            "  order: L{} {} below [{}]",
            d.line,
            d.key,
            below.join(", ")
        )
        .unwrap();
    }
}

#[test]
fn the_parser_reads_every_conformance_fixture_as_pinned() {
    let fixtures = root().join("fixtures/conformance");
    let goldens = root().join("golden/conformance");
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    if update {
        fs::create_dir_all(&goldens).unwrap();
    }

    let mut names: Vec<String> = fs::read_dir(&fixtures)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 36, "fixtures missing: {names:?}");

    let mut changed = Vec::new();
    for name in &names {
        let actual = render(&fixtures.join(name));
        let golden = goldens.join(format!("{name}.txt"));
        if update {
            fs::write(&golden, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "no golden for {name}. Create it with:\n  \
                 UPDATE_GOLDEN=1 cargo test --all-features --test parser_conformance"
            )
        });
        if expected != actual {
            changed.push(format!(
                "── {name}\n-- expected\n{expected}-- actual\n{actual}"
            ));
        }
    }

    // A golden whose fixture is gone pins nothing; it must go too.
    let orphans: Vec<String> = fs::read_dir(&goldens)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|g| !names.iter().any(|n| format!("{n}.txt") == *g))
        .collect();
    assert!(orphans.is_empty(), "goldens without a fixture: {orphans:?}");

    assert!(
        changed.is_empty(),
        "\n\nThe parser now reads {} conformance fixture(s) differently. Each is \
         a changed value in someone's file: a bug, or a decision to name in the \
         commit.\n\n{}",
        changed.len(),
        changed.join("\n")
    );
}
