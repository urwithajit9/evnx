//! Text snapshots of terminal output and files (T-SHARE-37).
//!
//! A snapshot pins what a person reads. To be stable across machines and days,
//! the parts that legitimately differ between runs are replaced with markers
//! first:
//!
//! | Varies | Becomes |
//! |---|---|
//! | the temp folder a test ran in (raw, canonical, macOS `/private`, Windows `\\?\`) | `<dir>` |
//! | `2026-10-14`, `2026-10-14 09:30`, `2026-10-14T09:30:05` | `<date>` |
//! | this crate's version, e.g. `0.10.1` | `<version>` |
//! | ANSI colour and cursor codes | removed |
//! | CRLF | LF |
//!
//! ⚠️ The version marker replaces the version string *anywhere*. A test whose
//! fixture contains the same digits as a value would see `<version>` there too;
//! use a different value in the fixture.
//!
//! `tests/init_golden.rs` and `tests/golden_output.rs` keep their own
//! normalisers on purpose: they pin bytes that other tools read, and their
//! rules are part of those contracts.

use std::path::{Path, PathBuf};

/// Replace what varies between runs with markers (see the module table).
/// `dirs` are the temp folders whose paths may appear in the text.
pub fn normalise(raw: &str, dirs: &[&Path]) -> String {
    let mut s = super::run::strip_ansi(raw).replace("\r\n", "\n");
    for spelling in dir_spellings(dirs) {
        s = s.replace(&spelling, "<dir>");
    }
    s = replace_dates(&s);
    s.replace(env!("CARGO_PKG_VERSION"), "<version>")
}

/// Compare `actual` (already normalised) with `tests/golden/<rel>`, or write
/// it under `UPDATE_GOLDEN=1`.
///
/// ```bash
/// UPDATE_GOLDEN=1 cargo test --all-features --test add_golden
/// git diff tests/golden/      # read every line before committing
/// ```
#[track_caller]
pub fn check_golden(rel: &str, actual: &str) {
    let path = golden_path(rel);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create golden dir");
        }
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "no golden file {} ({e}); create it with UPDATE_GOLDEN=1",
                path.display()
            )
        })
        // A CRLF checkout on Windows must not fail the comparison.
        .replace("\r\n", "\n");
    if expected != actual {
        panic!(
            "{rel} differs from its golden file\n{}\nIf the change is intended, rerun with UPDATE_GOLDEN=1 and review the diff.",
            line_diff(&expected, actual)
        );
    }
}

pub fn golden_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
}

/// Every way a folder's path may be printed, longest first so a longer
/// spelling is replaced before a shorter one it contains.
fn dir_spellings(dirs: &[&Path]) -> Vec<String> {
    let mut out = Vec::new();
    for d in dirs {
        let mut forms = vec![d.to_path_buf()];
        if let Ok(c) = d.canonicalize() {
            forms.push(c);
        }
        for p in forms {
            let s = p.to_string_lossy().into_owned();
            // Windows: canonicalize() returns a verbatim `\\?\C:\…` path.
            let plain = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
            for v in [s.clone(), plain.clone(), plain.replace('\\', "/")] {
                // macOS: /var/folders/… is a symlink to /private/var/folders/….
                if let Some(rest) = v.strip_prefix("/private") {
                    out.push(rest.to_string());
                }
                out.push(v);
            }
        }
    }
    out.retain(|s| !s.is_empty());
    out.sort_by_key(|s| std::cmp::Reverse(s.len()));
    out.dedup();
    out
}

/// `YYYY-MM-DD`, optionally followed by ` HH:MM`, `THH:MM` and `:SS`.
fn replace_dates(s: &str) -> String {
    let b = s.as_bytes();
    let digits =
        |i: usize, n: usize| i + n <= b.len() && b[i..i + n].iter().all(u8::is_ascii_digit);
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        let at_boundary = i == 0 || !b[i - 1].is_ascii_digit();
        if at_boundary
            && digits(i, 4)
            && b.get(i + 4) == Some(&b'-')
            && digits(i + 5, 2)
            && b.get(i + 7) == Some(&b'-')
            && digits(i + 8, 2)
            && !digits(i + 10, 1)
        {
            let mut end = i + 10;
            if matches!(b.get(end), Some(b' ' | b'T'))
                && digits(end + 1, 2)
                && b.get(end + 3) == Some(&b':')
                && digits(end + 4, 2)
            {
                end += 6;
                if b.get(end) == Some(&b':') && digits(end + 1, 2) {
                    end += 3;
                }
            }
            out.push_str(&s[copied..i]);
            out.push_str("<date>");
            copied = end;
            i = end;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[copied..]);
    out
}

/// A minimal line diff for failure messages: the first differing line, with
/// context. Enough to see what changed without a diff dependency.
fn line_diff(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let first = e
        .iter()
        .zip(&a)
        .position(|(x, y)| x != y)
        .unwrap_or(e.len().min(a.len()));
    let from = first.saturating_sub(3);
    let mut out = format!("first difference at line {}:\n", first + 1);
    for (i, line) in e.iter().enumerate().skip(from).take(8) {
        out.push_str(&format!("- {:>4} {line}\n", i + 1));
    }
    for (i, line) in a.iter().enumerate().skip(from).take(8) {
        out.push_str(&format!("+ {:>4} {line}\n", i + 1));
    }
    out
}
