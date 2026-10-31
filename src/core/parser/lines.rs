//! The line pass: splitting `KEY=value`, key rules, quoting, inline comments,
//! escapes, and where a multiline value ends.

use super::expand::LITERAL_DOLLAR;
use super::{is_valid_key, ParseError, ParseResult, Parser};

/// Where an unquoted value ends, given that `#` may start a comment.
///
/// ⛔ **A `#` is a comment only when whitespace precedes it.** This was
/// `raw.find('#')`, which took the first one anywhere — so every `#` inside a
/// value truncated it, silently:
///
/// ```text
/// COLOR=#ff0000           →  ""
/// PW=Tr0ub4dor#3          →  "Tr0ub4dor"
/// URL=https://x/#/route   →  "https://x/"
/// TAG=v1.0#rc1            →  "v1.0"
/// ```
///
/// ⚠️ **The ecosystem is split on this and the choice is deliberate.** Measured,
/// not assumed:
///
/// | | evnx (before) | Node dotenv 17 | Docker Compose |
/// |---|---|---|---|
/// | `PW=Tr0ub4dor#3` | `Tr0ub4dor` | `Tr0ub4dor` | `Tr0ub4dor#3` |
/// | `COLOR=#ff0000` | `` | `` | `#ff0000` |
/// | `A=v # comment` | `v` | `v` | `v` |
///
/// Node dotenv truncates at any `#`; Compose requires the whitespace. They
/// agree only on the case that is actually a comment. evnx follows **Compose**,
/// for two reasons: the failure mode of the other rule is silent destruction of
/// a password or a URL, and `evnx convert --to docker` would otherwise emit a
/// file Compose reads differently from the `.env` it came from.
///
/// ⓘ A `#` at the very start of a value is literal, which is also what Compose
/// does — so `KEY= # c` is the value `# c`, and there is no way to write an
/// empty value followed by a comment. That is Compose's behaviour verbatim,
/// checked with `docker compose config`.
pub(super) fn strip_inline_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(offset) = value[from..].find('#') {
        let at = from + offset;
        // ⓘ Byte-indexing is safe for this test: every ASCII whitespace byte is
        // < 0x80, and no continuation byte of a multi-byte character is.
        if at > 0 && bytes[at - 1].is_ascii_whitespace() {
            return value[..at].trim_end();
        }
        from = at + 1;
    }
    value.trim_end()
}

impl Parser {
    // ── Private: line parsing ─────────────────────────────────────────────────

    /// Split `line` into `(key, raw_value)` at the first `=`.
    ///
    /// Handles the optional `export` prefix used by shell scripts and tools
    /// like Heroku CLI and direnv.
    pub(super) fn split_key_value(
        &self,
        line: &str,
        line_num: usize,
    ) -> ParseResult<(String, String)> {
        let (line, exported) = match strip_export(line) {
            Some(rest) => (rest, true),
            None => (line, false),
        };
        if exported && (line.is_empty() || line.starts_with('=')) {
            return Err(ParseError::InvalidFormat {
                line: line_num,
                message: "`export` is followed by no key".into(),
            });
        }

        let eq = line.find('=').ok_or_else(|| ParseError::InvalidFormat {
            line: line_num,
            message: "missing '=' separator".into(),
        })?;

        let key = line[..eq].trim().to_string();
        let raw = line[eq + 1..].to_string(); // intentionally NOT trimmed yet

        Ok((key, raw))
    }

    /// Enforce key naming rules: `[A-Za-z_][A-Za-z0-9_]*`, and uppercase-only
    /// when [`ParserConfig::strict`] is set.
    ///
    /// ⚠️ **A leading underscore is allowed**, and was not until 2026-09-24.
    /// `_INTERNAL=1` is a valid environment variable name in POSIX shells and in
    /// every mainstream dotenv library, and `evnx doctor` already accepted it —
    /// so the parser was the stricter of two disagreeing definitions inside one
    /// binary. Found by `doctor_agrees_with_the_parser_about_what_is_valid` on
    /// its first run.
    ///
    /// A digit first is still refused: `1FOO` is not a name a shell can export.
    pub(super) fn validate_key(&self, key: &str, line_num: usize) -> ParseResult<()> {
        if !is_valid_key(key) {
            return Err(ParseError::InvalidKey {
                line: line_num,
                key: key.to_string(),
            });
        }
        self.validate_key_strict(key, line_num)
    }

    /// Strict mode only: the rest of the original check.
    pub(super) fn validate_key_strict(&self, key: &str, line_num: usize) -> ParseResult<()> {
        if key.is_empty() {
            return Err(ParseError::InvalidKey {
                line: line_num,
                key: key.to_string(),
            });
        }

        let mut chars = key.chars();

        // First character: a letter or an underscore, never a digit.
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => {
                return Err(ParseError::InvalidKey {
                    line: line_num,
                    key: key.to_string(),
                })
            }
        }

        // Remaining characters: alphanumeric or underscore.
        for c in chars {
            if !c.is_ascii_alphanumeric() && c != '_' {
                return Err(ParseError::InvalidKey {
                    line: line_num,
                    key: key.to_string(),
                });
            }
        }

        // Strict mode: all-uppercase required.
        if self.config.strict && key != key.to_uppercase() {
            return Err(ParseError::InvalidKey {
                line: line_num,
                key: key.to_string(),
            });
        }

        Ok(())
    }

    // ── Private: value parsing ────────────────────────────────────────────────

    /// Parse a raw value string into its final form.
    ///
    /// Dispatch order:
    /// 1. Empty → empty string.
    /// 2. Double-quoted → unescape escape sequences.
    /// 3. Single-quoted / backtick → literal: no unescaping, and since v0.7.0 no
    ///    expansion either.
    /// 4. Unquoted → strip inline comment, optionally trim.
    ///
    /// Returns the value **and whether the form was literal**. That second half is
    /// what lets `expand_all` leave single quotes alone: expansion runs over the
    /// assembled map, long after the quote characters are gone, so the form has to be
    /// carried forward or it is lost.
    pub(super) fn parse_value(&self, raw: &str, line_num: usize) -> ParseResult<(String, bool)> {
        let raw = raw.trim_start(); // leading whitespace after `=` is never significant

        if raw.is_empty() {
            return Ok((String::new(), false));
        }

        let first = raw.chars().next().unwrap(); // safe: checked is_empty above

        match first {
            // ⚠️ **The closing quote, wherever it is — not the end of the line.**
            //
            // These two branches tested `raw.ends_with(q)`, so a quoted value
            // with anything after it failed:
            //
            // ```text
            // KEY="value" # a trailing comment
            //   → Unterminated quoted string at line 1
            // ```
            //
            // and because the error aborts the whole parse, one such line made
            // `validate`, `diff` and every other command exit 2 on the entire
            // file. The syntax is valid in every dotenv implementation there is;
            // the comment two branches down even said inline comments are
            // handled "only outside quotes", which was true and was the bug —
            // a `#` *after a closing quote* is already outside them.
            '"' | '\'' | '`' => {
                let Some(close) = closing_quote_index(raw, first) else {
                    return Err(ParseError::UnterminatedString { line: line_num });
                };
                let inner = &raw[first.len_utf8()..close];
                let after = raw[close + first.len_utf8()..].trim();

                // Whatever follows the closing quote is not part of the value.
                // Empty or a comment is fine; anything else is a mistake worth
                // naming, because silently discarding it would hide a missing
                // quote or a stray paste.
                // ⚠️ Counted, never quoted: the stray text is often the rest of
                // a secret, and this message reaches terminals and CI logs.
                if !after.is_empty() && !after.starts_with('#') {
                    let n = after.chars().count();
                    let s = if n == 1 { "" } else { "s" };
                    return Err(ParseError::InvalidFormat {
                        line: line_num,
                        message: format!(
                            "unexpected text after the closing {first} ({n} character{s}) \
                             — quote the whole value, or start a comment with #"
                        ),
                    });
                }

                // Double quotes interpret escapes; `'` and backtick are literal,
                // which is the promise their form makes.
                if first == '"' {
                    Ok((self.unescape_double(inner), false))
                } else {
                    Ok((inner.to_string(), true))
                }
            }

            _ => {
                // Unquoted value.
                let val = if self.config.allow_inline_comments {
                    // Only outside quotes — we are already in the unquoted branch.
                    strip_inline_comment(raw)
                } else {
                    raw.trim_end()
                };

                // Unquoted still expands. It has no escape mechanism either, but
                // it also makes no literal promise — single quotes are the form that
                // does, and now keeps it.
                if self.config.trim_values {
                    Ok((val.trim().to_string(), false))
                } else {
                    Ok((val.to_string(), false))
                }
            }
        }
    }

    /// Process backslash escape sequences inside a double-quoted value.
    ///
    /// Recognised sequences: `\n`, `\r`, `\t`, `\\`, `\"`, `\'`.
    /// Unknown sequences are kept literally (backslash + character).
    pub(super) fn unescape_double(&self, s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars();

        while let Some(ch) = chars.next() {
            if ch != '\\' {
                result.push(ch);
                continue;
            }
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                // ⚠️ `\$` means a literal dollar, and it has to be *marked* rather
                // than simply emitted.
                //
                // Expansion runs later, over the assembled map, so by then a `$`
                // that came from `\$` and one that was always bare look identical —
                // and worse, `\\$` (a literal backslash before a real expansion)
                // unescapes to `\$` too. Emitting a plain `$` here would make those
                // three cases indistinguishable and would regress `"\\$B"`, which
                // correctly yields a backslash followed by B's value today.
                //
                // So the marker travels with it and `expand_value` consumes it. It
                // is a Unicode noncharacter, permanently reserved and never valid in
                // interchange, so it cannot collide with real data.
                Some('$') => {
                    result.push(LITERAL_DOLLAR);
                    result.push('$');
                }
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        }
        result
    }
}

// ── Multiline quote tracking ──────────────────────────────────────────────────
//
// These are free functions rather than `Parser` methods because `evnx doctor`
// needs them too, and duplicating them is what produced the bug they fix.
//
// ⚠️ `doctor` checks `.env` syntax line by line so it can report every bad line
// at once, which the parser cannot do — it stops at its first error. That
// duplication is deliberate, but it must not extend to *deciding where a value
// ends*. It did, and `doctor` reported every continuation line of a valid
// multiline value as `invalid syntax` while `validate`, `convert` and `scan`
// accepted the same file.

/// Does `s` end with a `q` that actually closes a quoted value?
///
/// ⚠️ The check used to be a bare `ends_with(q)`, and that is the whole of bug
/// B5. For the value `"line1 \"q\"` the final character *is* `"` — the trailing
/// quote of an escaped `\"` — so the parser concluded the value had closed and
/// treated the next line as a stray entry:
///
/// ```text
/// B="line1 \"q\"
/// line2"
/// → Invalid format at line 2: missing '=' separator
/// ```
///
/// Only double-quoted values interpret backslash escapes; `'` and backtick
/// values are literal, so for those a trailing quote always closes. For `"` the
/// quote closes when the run of backslashes immediately before it has even
/// length: `\"` is an escaped quote, `\\"` is an escaped backslash followed by a
/// real closing quote.
pub(super) fn ends_with_closing_quote(s: &str, q: char) -> bool {
    let Some(body) = s.strip_suffix(q) else {
        return false;
    };
    if q != '"' {
        return true;
    }
    body.chars().rev().take_while(|&c| c == '\\').count() % 2 == 0
}

/// The byte index of the quote that **closes** a value opened with `q`, if the
/// value closes at all on this string.
///
/// ⚠️ This exists because asking whether the *line* ends with a quote is a
/// different question, and the two came apart on valid syntax:
///
/// ```text
/// KEY="value" # a trailing comment
/// ```
///
/// The quote closes at index 6. The line ends with `t`. So
/// `ends_with_closing_quote` said "not closed", `opens_multiline` reported a
/// multiline value, the parser scanned to the end of the file looking for a
/// quote that had already been found, and the whole file failed with
/// `Unterminated quoted string at line 1` — on a line that is well-formed in
/// every dotenv implementation there is.
///
/// Escapes are honoured for `"` only, matching [`ends_with_closing_quote`] and
/// the value parser: `'` and backtick values are literal, so the first quote
/// they meet closes them.
pub(super) fn closing_quote_index(s: &str, q: char) -> Option<usize> {
    let mut it = s.char_indices();
    // Skip the opening quote. A string with nothing after it cannot close.
    it.next()?;
    let mut escaped = false;
    for (i, c) in it {
        if escaped {
            escaped = false;
            continue;
        }
        if q == '"' && c == '\\' {
            escaped = true;
            continue;
        }
        if c == q {
            return Some(i);
        }
    }
    None
}

/// The quote character a multiline value is opened with, if `raw_value` opens one.
///
/// `None` means the value is complete on its line — either unquoted, or quoted
/// and closed.
pub fn opens_multiline(raw_value: &str) -> Option<char> {
    let q = match raw_value.trim_start().chars().next() {
        Some(c @ ('"' | '\'' | '`')) => c,
        _ => return None,
    };
    let t = raw_value.trim();
    // ⚠️ "does the quote close anywhere" — not "does the line end with it".
    // A closing quote followed by a comment still closes the value, and a lone
    // quote character has nothing after it to close with.
    if closing_quote_index(t, q).is_some() {
        None
    } else {
        Some(q)
    }
}

/// As [`opens_multiline`], but taking a whole `KEY=VALUE` line.
///
/// Exists so `doctor` does not have to re-implement splitting on `=` and
/// stripping `export`.
pub fn line_opens_multiline(line: &str) -> Option<char> {
    let line = line.trim();
    let line = strip_export(line).unwrap_or(line);
    let eq = line.find('=')?;
    opens_multiline(&line[eq + 1..])
}

/// The content of `line` before the quote that closes a multiline value opened
/// with `q`, or `None` if this line does not close it.
pub fn closes_multiline(line: &str, q: char) -> Option<&str> {
    let trimmed = line.trim_end();
    ends_with_closing_quote(trimmed, q).then(|| &trimmed[..trimmed.len() - q.len_utf8()])
}

/// What follows a leading `export` keyword, or `None` when there is none.
///
/// ⚠️ The keyword needs whitespace after it, or must be the whole line. This was
/// `strip_prefix("export")`, so `exporter=1` was read as the key `er` and
/// `exports_dir` as `s_dir`, silently; Docker Compose and python-dotenv keep
/// both names whole (B2 P0.4).
pub fn strip_export(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("export")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}
