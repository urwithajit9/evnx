//! ⚠️ TEST ONLY, and temporary: the parser's entry point exactly as it was
//! before `Document` (v0.12.0 B1), kept as the oracle for the differential test
//! in `tests.rs`. Delete in the first release after 0.12.0.

use indexmap::IndexMap;
use std::collections::HashSet;

use super::expand::strip_markers;
use super::{closes_multiline, opens_multiline, ParseError, ParseResult, Parser};

impl Parser {
    pub(super) fn legacy_parse_content_located(
        &self,
        content: &str,
    ) -> ParseResult<(IndexMap<String, String>, IndexMap<String, usize>)> {
        let mut vars: IndexMap<String, String> = IndexMap::new();
        // Keys written in a form that promises to be literal — single quotes or
        // backticks. Tracked separately because the quote characters are gone by the
        // time `expand_all` runs over the assembled map.
        let mut literal_keys: HashSet<String> = HashSet::new();
        // Where each key was written.
        //
        // ⚠️ Same reason as `literal_keys`: expansion is a separate pass over the
        // assembled map, so it has no line to report unless one is carried to it. It
        // reported **line 0** for every undefined variable — not a line in any file,
        // so it pointed at nothing, in exactly the case where it matters: a large
        // `.env` that suddenly will not parse.
        let mut key_lines: IndexMap<String, usize> = IndexMap::new();

        // Multiline accumulation state.
        let mut ml_key: Option<String> = None;
        let mut ml_value = String::new();
        let mut ml_quote: char = '"';
        let mut ml_start_line: usize = 0;

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1; // 1-indexed for all user-facing messages

            // ── Multiline continuation ────────────────────────────────────────
            if let Some(ref key) = ml_key.clone() {
                let trimmed_end = raw_line.trim_end();

                // ⚠️ `strip_suffix(ml_quote)` here, which closed the value on an
                // escaped `\"` exactly as `is_closed_quote` did on the opening
                // line. The bug had two sites; a value could survive its first
                // line and then be cut short by a continuation line ending in
                // `\"`.
                if let Some(before_close) = closes_multiline(trimmed_end, ml_quote) {
                    // Closing quote found — finalise the value.
                    ml_value.push('\n');
                    ml_value.push_str(before_close);

                    // ⚠️ The accumulated value used to be inserted raw, so a
                    // double-quoted value that spanned lines kept its
                    // backslashes while the identical value on one line had them
                    // resolved: `"x\ty"` gave a tab, `"x\ty` + `z"` gave a
                    // literal backslash and `t`. Same quote character, different
                    // escape rules depending on the line count — and it made
                    // `\"` inexpressible in a multiline value, which is the
                    // other half of B5.
                    //
                    // `unescape_double` only rewrites backslash sequences, so
                    // the real newlines pushed above are untouched and PEM keys
                    // (which contain no backslashes) are unaffected.
                    // ⚠️ The same split as the single-line case, and for the same
                    // reason: a multiline single-quoted value is literal in every
                    // respect, escapes and expansion alike. A PEM key wrapped in
                    // single quotes now means exactly its bytes.
                    let finished = if ml_quote == '"' {
                        self.unescape_double(&ml_value)
                    } else {
                        literal_keys.insert(key.clone());
                        ml_value.clone()
                    };
                    key_lines.insert(key.clone(), ml_start_line);
                    vars.insert(key.clone(), finished);
                    ml_key = None;
                    ml_value.clear();
                } else {
                    // Still inside a multiline value — accumulate.
                    ml_value.push('\n');
                    ml_value.push_str(raw_line);
                }
                continue;
            }

            // ── Skip blank lines and full-line comments ────────────────────────
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // ── Parse KEY=VALUE ───────────────────────────────────────────────
            let (key, raw_value) = self.split_key_value(line, line_num)?;

            // ── Key validation ────────────────────────────────────────────────
            self.validate_key(&key, line_num)?;

            // ── Value parsing ─────────────────────────────────────────────────
            match opens_multiline(&raw_value) {
                // An opening quote with no closing quote on this line.
                Some(q) if self.config.allow_multiline => {
                    // Opening quote but no closing quote on this line.
                    ml_key = Some(key);
                    // Strip the opening quote from the accumulated content.
                    ml_value = raw_value.trim_start_matches(q).to_string();
                    ml_quote = q;
                    ml_start_line = line_num;
                }
                _ => {
                    let (value, is_literal) = self.parse_value(&raw_value, line_num)?;
                    if is_literal {
                        literal_keys.insert(key.clone());
                    }
                    key_lines.insert(key.clone(), line_num);
                    vars.insert(key, value);
                }
            }
        }

        // If we exited the loop still inside a multiline value, the file ended
        // without a closing quote.
        if let Some(_key) = ml_key {
            return Err(ParseError::UnterminatedString {
                line: ml_start_line,
            });
        }

        // ── Variable expansion ────────────────────────────────────────────────
        if self.config.allow_expansion {
            self.legacy_expand_all(&mut vars, &literal_keys, &key_lines)?;
        } else {
            // ⚠️ `expand_all` is what normally consumes the `\$` markers, so with
            // expansion off nothing would — and `"\$B"` would come back holding a
            // U+FDD0 noncharacter. Found by reading this branch rather than by a
            // test, because no CLI flag reaches it today; `allow_expansion` is a
            // library setting, so a consumer of the crate could.
            for value in vars.values_mut() {
                *value = strip_markers(value);
            }
        }

        Ok((vars, key_lines))
    }

    fn legacy_expand_all(
        &self,
        vars: &mut IndexMap<String, String>,
        literal: &HashSet<String>,
        lines: &IndexMap<String, usize>,
    ) -> ParseResult<()> {
        // Snapshot keys to avoid borrow conflicts while mutating the map.
        let keys: Vec<String> = vars.keys().cloned().collect();
        let mut expanded: IndexMap<String, String> = IndexMap::with_capacity(vars.len());

        for key in &keys {
            let value = vars[key].clone();
            if literal.contains(key) {
                // ⚠️ Still stripped. A single-quoted value never goes through
                // `unescape_double`, so it holds no marker — but stripping
                // unconditionally means no path can leak one to a caller.
                expanded.insert(key.clone(), strip_markers(&value));
                continue;
            }
            let mut stack: Vec<String> = Vec::new();
            // ⚠️ The line of the key being expanded, not of the key that defines
            // the missing variable — which may not exist anywhere. "`${NOPE}` is not
            // defined" is only actionable if it names where you wrote it.
            let line = lines.get(key).copied().unwrap_or(0);
            let result = self.expand_value(&value, vars, &mut stack, 0, line)?;
            expanded.insert(key.clone(), strip_markers(&result));
        }

        *vars = expanded;
        Ok(())
    }
}
