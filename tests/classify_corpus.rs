//! The classifier corpus (RM-38): every `name=value` evnx has a verdict on,
//! in one file, checked against the four questions every command asks through
//! `core::classify` and `core::mask`.
//!
//! `tests/fixtures/classify-corpus.tsv`, one row per pair:
//!
//! ```text
//! name  value  secret  strict  lenient  masked  note
//! ```
//!
//! * `secret`: `holds_secret`, by name or by value (`y`/`n`).
//! * `strict`, `lenient`: `is_placeholder` at each level (`validate` uses
//!   Strict, `scan` and `diff` Lenient).
//! * `masked`: what `mask` prints.
//!
//! A value may hold `\t`, `\n` and `\\`; an empty field is an empty value.
//!
//! ⚠️ Every disagreeing row is reported, not the first: a change to one list
//! moves many verdicts, and the point is to see all of them at once.
//! `UPDATE_CORPUS=1` rewrites the four expected columns from what the code
//! says now, and the diff is what gets reviewed, as with the goldens.
//!
//! Values are fixtures, never live credentials: documented example keys, or
//! strings shaped like a format.

use evnx::core::classify::{holds_secret, is_placeholder, Strictness};
use evnx::core::mask::mask;
use std::path::Path;

const CORPUS: &str = "tests/fixtures/classify-corpus.tsv";

struct Row {
    line: usize,
    name: String,
    value: String,
    expect: [String; 4],
    note: String,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn yn(b: bool) -> String {
    if b { "y" } else { "n" }.to_string()
}

fn actual(name: &str, value: &str) -> [String; 4] {
    [
        yn(holds_secret(name, value)),
        yn(is_placeholder(name, value, Strictness::Strict)),
        yn(is_placeholder(name, value, Strictness::Lenient)),
        mask(name, value),
    ]
}

fn read() -> (Vec<String>, Vec<Row>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS);
    let text = std::fs::read_to_string(&path).expect("read the corpus");
    let mut header = Vec::new();
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            if rows.is_empty() {
                header.push(line.to_string());
            }
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            f.len(),
            7,
            "{CORPUS}:{}: 7 tab-separated fields, found {}",
            i + 1,
            f.len()
        );
        rows.push(Row {
            line: i + 1,
            name: f[0].to_string(),
            value: unescape(f[1]),
            expect: [f[2], f[3], f[4], f[5]].map(unescape),
            note: f[6].to_string(),
        });
    }
    (header, rows)
}

#[test]
fn rm38_every_row_of_the_corpus_agrees() {
    let (header, rows) = read();
    assert!(rows.len() >= 100, "the corpus has only {} rows", rows.len());

    if std::env::var_os("UPDATE_CORPUS").is_some() {
        let mut out = header.join("\n");
        out.push('\n');
        for r in &rows {
            let a = actual(&r.name, &r.value);
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                r.name,
                escape(&r.value),
                a[0],
                a[1],
                a[2],
                escape(&a[3]),
                r.note
            ));
        }
        std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS), out).unwrap();
        return;
    }

    const COLUMNS: [&str; 4] = ["secret", "strict", "lenient", "masked"];
    let mut wrong = Vec::new();
    for r in &rows {
        let a = actual(&r.name, &r.value);
        for (i, col) in COLUMNS.iter().enumerate() {
            if a[i] != r.expect[i] {
                wrong.push(format!(
                    "  line {}: {}={:?}  {col}: expected {:?}, got {:?}  ({})",
                    r.line, r.name, r.value, r.expect[i], a[i], r.note
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} rows disagree with {CORPUS} \
         (UPDATE_CORPUS=1 rewrites it from the code; review the diff):\n{}",
        wrong.len(),
        rows.len(),
        wrong.join("\n")
    );
}

/// A row may not be there twice: two verdicts for one pair would hide which
/// one the code meets.
#[test]
fn rm38_no_pair_appears_twice() {
    let (_, rows) = read();
    let mut seen = std::collections::HashMap::new();
    let dups: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            seen.insert((r.name.clone(), r.value.clone()), r.line)
                .map(|first| {
                    format!(
                        "line {} repeats line {first}: {}={:?}",
                        r.line, r.name, r.value
                    )
                })
        })
        .collect();
    assert!(dups.is_empty(), "{}", dups.join("\n"));
}

/// DEC-G13 says Strict is the cautious level: what `validate` calls a
/// placeholder, `scan` and `diff` must too. Over the corpus that holds except
/// for RFC 2606 documentation hosts (`https://api.example.com/v1`), which only
/// the Strict path checks (`classify.rs`, `DOC_HOSTS`). Pinned as found; the
/// fix moves `DOC_HOSTS` ahead of the split and empties this list.
#[test]
fn known_strict_placeholders_lenient_does_not_share() {
    let (_, rows) = read();
    let odd: Vec<String> = rows
        .iter()
        .filter(|r| {
            is_placeholder(&r.name, &r.value, Strictness::Strict)
                && !is_placeholder(&r.name, &r.value, Strictness::Lenient)
        })
        .map(|r| r.value.clone())
        .collect();
    assert_eq!(
        odd,
        [
            "postgresql://reader@db.example.com/main",
            "https://api.example.com/?to=a@b.com",
            "https://api.example.com/users/me@example.com",
            "https://api.example.com/v1",
        ]
    );
}
