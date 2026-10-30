//! Variable expansion: `${VAR}` and `$VAR`, run over the assembled map.

use indexmap::IndexMap;
use std::collections::HashSet;

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

    /// Expand all `${VAR}` and `$VAR` references across the full variable map.
    ///
    /// Each value is expanded independently. Circular references and undefined
    /// variables produce structured errors.
    /// Expand every value that is not written in a literal form.
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
    pub(super) fn expand_all(
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
