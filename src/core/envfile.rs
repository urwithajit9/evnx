//! Writing `.env` lines, and deciding whether text is a `.env` (C-01, U5).
//!
//! One rule for everything evnx writes into a `.env` or `.env.example`: a value
//! is written so that the parser reads back **exactly** that value.
//!
//! | Value | Written as | Why |
//! |---|---|---|
//! | plain (`info`, `8080`, `https://x/y`) | bare | nothing to protect |
//! | empty | bare (`KEY=`) | the template convention |
//! | `$` or `\`, no `'`, no line break | `'single quotes'` | literal in evnx and in every common dotenv reader; double quotes would expand `$NAME` |
//! | anything else that needs quoting | `"double quotes"` with `\\` `\"` `\$` `\n` `\r` | the only form that holds a line break |
//!
//! "Needs quoting": a space or tab, `#`, a quote of any kind, `$`, `\`, or
//! leading/trailing whitespace. (`=` does not: the parser splits on the first
//! one, so `URL=https://x/?a=b` stays bare.) Raw `KEY=value` lines lost everything after
//! ` #` (I-114) and the outer whitespace.
//!
//! [`is_dotenv`] asks the same parser. The line heuristic it replaces rejected
//! `export A=1` and files with a PEM block, so `restore` set valid backups
//! aside as `.restored` (I-145).

use std::borrow::Cow;

use super::parser::{Parser, ParserConfig};

/// `value` as it must appear after `KEY=`.
pub fn quote(value: &str) -> Cow<'_, str> {
    if !needs_quoting(value) {
        return Cow::Borrowed(value);
    }
    let breaks = value.contains('\n') || value.contains('\r');
    if !breaks && !value.contains('\'') && (value.contains('$') || value.contains('\\')) {
        return Cow::Owned(format!("'{value}'"));
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    Cow::Owned(out)
}

/// One `KEY=value` line, without the newline.
pub fn line(key: &str, value: &str) -> String {
    format!("{key}={}", quote(value))
}

fn needs_quoting(value: &str) -> bool {
    !value.is_empty()
        && (value.starts_with(char::is_whitespace)
            || value.ends_with(char::is_whitespace)
            || value.chars().any(|c| {
                matches!(
                    c,
                    ' ' | '\t' | '\n' | '\r' | '#' | '"' | '\'' | '`' | '$' | '\\'
                )
            }))
}

/// The parser `is_dotenv` and `count_vars` use: no expansion, so a reference
/// to an undefined or circular variable does not make a file "not a `.env`".
fn reader() -> Parser {
    Parser::new(ParserConfig {
        allow_expansion: false,
        ..Default::default()
    })
}

/// Whether evnx can read `content` as a `.env`. Empty content is one.
pub fn is_dotenv(content: &str) -> bool {
    reader().parse_content(content).is_ok()
}

/// How many variables `content` holds, or `None` when it is not a `.env`.
pub fn count_vars(content: &str) -> Option<usize> {
    reader().parse_content(content).ok().map(|vars| vars.len())
}

/// The names in `a` that `b` does not have, in `a`'s order.
///
/// One answer for "what is on one side only" (R-sync, 0.10.12). `sync`
/// computed it four times — forward, reverse, the JSON report, and the check
/// with no `.env` — and the JSON copy drifted from the others (I-117); `diff`
/// computed its `missing` and `extra` the same way. Source order, never hash
/// order: `sync` writes this order into a committed file.
pub fn only_in<V, W>(
    a: &indexmap::IndexMap<String, V>,
    b: &indexmap::IndexMap<String, W>,
) -> Vec<String> {
    a.keys().filter(|k| !b.contains_key(*k)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read `KEY=<quoted>` back with the default parser, after a definition of
    /// `A`, so an unescaped `$A` would show as an expansion.
    fn round_trip(value: &str) -> String {
        let content = format!("A=1\nKEY={}\n", quote(value));
        let vars = Parser::default()
            .parse_content(&content)
            .unwrap_or_else(|e| panic!("{value:?} as {content:?}: {e}"));
        vars["KEY"].clone()
    }

    const TRICKY: &[&str] = &[
        "My App # prod",
        "hello # world",
        "a b",
        "  edge  ",
        "\tlead",
        "$A",
        "x$A/y",
        "${A}",
        "$5 off",
        "back\\slash",
        "C:\\path\\$A",
        "it's",
        "it's $A",
        "say \"hi\"",
        "both ' and \"",
        "a`b",
        "k=v",
        "line1\nline2",
        "-----BEGIN KEY-----\nQUJD\n-----END KEY-----",
        "cr\r\nlf",
        "日本語 の 値",
        "\\n literal",
        "#",
        "=",
    ];

    /// T-SHARE (C-01), I-114: every value reads back unchanged.
    #[test]
    fn t_i114_every_value_round_trips() {
        for v in TRICKY {
            assert_eq!(&round_trip(v), v, "{v:?} written as {}", quote(v));
        }
    }

    #[test]
    fn plain_values_stay_bare() {
        for v in [
            "info",
            "8080",
            "https://example.com/a?b=c",
            "",
            "your_value_here",
        ] {
            assert_eq!(quote(v), v);
        }
        assert_eq!(line("PORT", "8080"), "PORT=8080");
        assert_eq!(line("EMPTY", ""), "EMPTY=");
    }

    #[test]
    fn a_dollar_prefers_single_quotes() {
        assert_eq!(quote("$5 off"), "'$5 off'");
        assert_eq!(quote("it's $5"), "\"it's \\$5\"");
    }

    /// I-145: what `restore` used to set aside.
    #[test]
    fn t_i145_export_and_pem_files_are_dotenv() {
        assert!(is_dotenv("export A=1\nexport B=2\n"));
        let pem: String = (0..30).map(|_| "QUJDREVGR0g=\n").collect();
        assert!(is_dotenv(&format!(
            "A=1\nKEY=\"-----BEGIN-----\n{pem}-----END-----\"\n"
        )));
        assert_eq!(count_vars("export A=1\n# c\n\nB=\"x\ny\"\n"), Some(2));
    }

    #[test]
    fn prose_and_binary_are_not() {
        assert!(!is_dotenv("Dear reader,\nthis is a letter.\n"));
        assert!(!is_dotenv("\u{0}\u{1}PK\u{3}\u{4} binary"));
        assert!(is_dotenv(""));
        assert!(is_dotenv("# only a comment\n\n"));
        assert_eq!(count_vars("prose"), None);
    }

    /// An undefined or circular reference is still a `.env`.
    #[test]
    fn references_do_not_decide() {
        assert!(is_dotenv("A=$B\nB=$A\n"));
        assert!(is_dotenv("A=${MISSING}\n"));
    }

    #[test]
    fn only_in_keeps_the_first_maps_order() {
        let a: indexmap::IndexMap<String, &str> = [("Z", "1"), ("A", "2"), ("M", "3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let b: indexmap::IndexMap<String, &str> = [("A", "x")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(only_in(&a, &b), ["Z", "M"]);
        assert!(only_in(&b, &a).is_empty());
    }
}
