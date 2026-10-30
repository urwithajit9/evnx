//! Variable expansion: `${VAR}` and `$VAR`, run over the assembled map.

use indexmap::IndexMap;
use std::collections::HashSet;

use super::document::{Document, Entry, Problem};
use super::{ParseError, ParseResult, Parser};

/// Marks a `$` that came from `\$` and must not be expanded.
///
/// U+FDD0 is a Unicode **noncharacter**: permanently reserved, never assigned, and
/// not valid in interchange — so it cannot appear in a `.env` anyone wrote. It exists
/// only between `unescape_double` and `expand_value`, and `strip_markers` removes any
/// that survive, so it never reaches a caller.
pub(super) const LITERAL_DOLLAR: char = '\u{FDD0}';

/// Remove any [`LITERAL_DOLLAR`] marker.
///
/// Called on every path out of the parser, including the ones that cannot contain
/// one, so that no combination of settings can hand a caller an internal marker.
/// `expansion_enabled = false` is exactly such a path: it never runs `expand_value`,
/// which is what normally consumes them.
pub(super) fn strip_markers(s: &str) -> String {
    if s.contains(LITERAL_DOLLAR) {
        s.replace(LITERAL_DOLLAR, "")
    } else {
        s.to_string()
    }
}

impl Parser {
    // ── Private: variable expansion ───────────────────────────────────────────

    /// Expand every entry that is not written in a literal form, against each
    /// key's last value. An undefined variable or a cycle becomes a [`Problem`]
    /// on the entry that counts; a shadowed entry that fails keeps its text
    /// silently, because the strict parser never read it.
    ///
    /// ⚠️ `literal` names the keys written in single quotes or backticks, and
    /// skipping them is a **behaviour change**: expansion used to run in every
    /// quoting form, so `'cost $B'` substituted `B`'s value where a shell, `dotenv`,
    /// `python-dotenv` and `godotenv` all leave it alone.
    ///
    /// That was the deeper half of the problem. Those forms are already literal for
    /// backslash escapes, so "single quotes are literal" was true of one thing and
    /// false of another — which is not a rule anyone can hold in their head. It also
    /// left a secret containing `$` with **no** way to be written: `\$` did not work
    /// either, and still does not in these forms, because there is no unescape pass
    /// to interpret it. Now there does not need to be one.
    pub(super) fn expand_entries(&self, doc: &mut Document, literal: &HashSet<String>) {
        // The map every reference resolves against: each key's last value.
        let vars = doc.vars();
        let mut effective: IndexMap<&str, usize> = IndexMap::with_capacity(vars.len());
        for (i, e) in doc.entries.iter().enumerate() {
            effective.insert(e.key.as_str(), i);
        }
        let effective: Vec<usize> = effective.into_values().collect();

        let expand = |e: &Entry| -> ParseResult<String> {
            if literal.contains(&e.key) {
                // ⚠️ Still stripped. A single-quoted value never goes through
                // `unescape_double`, so it holds no marker — but stripping
                // unconditionally means no path can leak one to a caller.
                return Ok(strip_markers(&e.value));
            }
            let mut stack: Vec<String> = Vec::new();
            // ⚠️ The line of the key being expanded, not of the key that defines
            // the missing variable — which may not exist anywhere. "`${NOPE}` is not
            // defined" is only actionable if it names where you wrote it.
            self.expand_value(&e.value, &vars, &mut stack, 0, e.span.line)
                .map(|v| strip_markers(&v))
        };

        // Effective entries first, in key order: that is the order the strict
        // parser met expansion errors in, so `problems` keeps its first error.
        let mut done = vec![false; doc.entries.len()];
        for i in effective {
            done[i] = true;
            match expand(&doc.entries[i]) {
                Ok(v) => doc.entries[i].value = v,
                Err(error) => {
                    let e = &mut doc.entries[i];
                    e.value = strip_markers(&e.value);
                    doc.problems.push(Problem {
                        span: e.span,
                        raw: e.raw.clone(),
                        error,
                    });
                }
            }
        }
        // A shadowed assignment is not a problem: the strict parser never read it.
        for (i, d) in done.into_iter().enumerate() {
            if !d {
                let e = &doc.entries[i];
                let v = expand(e).unwrap_or_else(|_| strip_markers(&e.value));
                doc.entries[i].value = v;
            }
        }
    }

    /// Recursively expand variable references within a single `value` string.
    ///
    /// # Arguments
    ///
    /// * `value`     — The string to expand.
    /// * `vars`      — The full variable map (snapshot at expansion start).
    /// * `stack`     — Variables currently being expanded (cycle detection).
    /// * `depth`     — Current recursion depth.
    /// * `line_hint` — Line number for error reporting (0 when unknown).
    pub(super) fn expand_value(
        &self,
        value: &str,
        vars: &IndexMap<String, String>,
        stack: &mut Vec<String>,
        depth: usize,
        line_hint: usize,
    ) -> ParseResult<String> {
        if depth > self.config.max_expansion_depth {
            return Err(ParseError::ExpansionDepthExceeded {
                line: line_hint,
                max: self.config.max_expansion_depth,
            });
        }

        let mut result = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();

        while let Some(ch) = chars.next() {
            // A marked `$` was written `\$`. Emit the dollar, drop the marker, and
            // do not look at what follows — that is the whole point of the escape.
            if ch == LITERAL_DOLLAR {
                if chars.peek() == Some(&'$') {
                    chars.next();
                    result.push('$');
                }
                continue;
            }

            if ch != '$' {
                result.push(ch);
                continue;
            }

            match chars.peek() {
                // ── ${VAR} syntax ─────────────────────────────────────────────
                Some(&'{') => {
                    chars.next(); // consume `{`
                    let var_name: String = chars.by_ref().take_while(|&c| c != '}').collect();

                    if stack.contains(&var_name) {
                        return Err(ParseError::CircularExpansion {
                            line: line_hint,
                            cycle: format!("{} → {}", stack.join(" → "), var_name),
                        });
                    }

                    match vars.get(&var_name) {
                        Some(val) => {
                            stack.push(var_name.clone());
                            let expanded =
                                self.expand_value(val, vars, stack, depth + 1, line_hint)?;
                            stack.pop();
                            result.push_str(&expanded);
                        }
                        None => {
                            return Err(ParseError::UndefinedVariable {
                                line: line_hint,
                                var: var_name,
                            });
                        }
                    }
                }

                // ── $VAR bare syntax ──────────────────────────────────────────
                Some(&c) if c.is_ascii_alphanumeric() || c == '_' => {
                    // ⚠️ Peek, then consume. This was `take_while`, which also
                    // consumes the first character that is *not* part of the
                    // name, so the character ending the reference vanished:
                    // `$BASE/api` became `http://localhostapi`, `$B:8080`
                    // became `app8080` (I-141). Every command that reads
                    // values went through it. `${VAR}` is unaffected: there
                    // the consumed terminator is the `}`, which belongs to it.
                    let mut var_name = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            var_name.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }

                    if stack.contains(&var_name) {
                        return Err(ParseError::CircularExpansion {
                            line: line_hint,
                            cycle: format!("{} → {}", stack.join(" → "), var_name),
                        });
                    }

                    match vars.get(&var_name) {
                        Some(val) => {
                            stack.push(var_name.clone());
                            let expanded =
                                self.expand_value(val, vars, stack, depth + 1, line_hint)?;
                            stack.pop();
                            result.push_str(&expanded);
                        }
                        None => {
                            // Bare $VAR: keep literal if undefined (common in
                            // shell scripts where $PATH etc. are expected to
                            // come from the environment, not the .env file).
                            // This diverges intentionally from ${VAR} which
                            // always errors — bare $ references are far more
                            // likely to be shell variables than typos.
                            result.push('$');
                            result.push_str(&var_name);
                        }
                    }
                }

                // ── Lone $ ────────────────────────────────────────────────────
                _ => {
                    result.push('$');
                }
            }
        }

        Ok(result)
    }
}
