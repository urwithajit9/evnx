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

    /// Expand every entry that is not written in a literal form, top to bottom:
    /// each reference resolves against the values assigned **above** it, as
    /// Docker Compose and python-dotenv read a file (D10, 0.13.0).
    ///
    /// ⚠️ This is a **behaviour change** from 0.12.0, which expanded against each
    /// key's final value:
    /// - `B=1` / `A=${B}` / `B=2` gives `A=1`, not `2`;
    /// - a forward reference (`A=${B}` / `B=1`) is an undefined variable, because
    ///   `${NOPE}` is an error by design, not `""`;
    /// - `A=1` / `A=${A}3` gives `13`, where it was a circular expansion.
    ///
    /// An undefined variable becomes a [`Problem`] on the entry that counts. A
    /// shadowed entry that fails is a problem only when a later entry reads it
    /// before it is assigned again: otherwise nothing reads that value, which is
    /// what the strict parser has always accepted (`A=${NOPE}` / `A=1`).
    ///
    /// ⚠️ `literal` names the keys written in single quotes or backticks, and they
    /// are never expanded: `'cost $B'` keeps `$B`, as a shell, `dotenv`,
    /// `python-dotenv` and `godotenv` do.
    pub(super) fn expand_entries(&self, doc: &mut Document, literal: &HashSet<String>) {
        // Values settled so far, with every `$` marked literal: they are already
        // expanded and must not be expanded again when a later entry reads them.
        let mut above: IndexMap<String, String> = IndexMap::new();
        // The entry each name was last assigned by, so far.
        let mut current: IndexMap<String, usize> = IndexMap::new();
        let mut pending: Vec<Option<ParseError>> = (0..doc.entries.len()).map(|_| None).collect();
        let mut problems: Vec<Problem> = Vec::new();

        for i in 0..doc.entries.len() {
            let e = &doc.entries[i];
            let value = if literal.contains(&e.key) {
                // ⚠️ Still stripped. A single-quoted value never goes through
                // `unescape_double`, so it holds no marker, but stripping
                // unconditionally means no path can leak one to a caller.
                strip_markers(&e.value)
            } else {
                // A failed entry read here, before it is assigned again, is a
                // problem after all: its value is used.
                for name in references(&e.value) {
                    if let Some(&k) = current.get(&name) {
                        if let Some(error) = pending[k].take() {
                            let src = &doc.entries[k];
                            problems.push(Problem {
                                span: src.span,
                                raw: src.raw.clone(),
                                error,
                            });
                        }
                    }
                }
                // ⚠️ The line of the key being expanded, not of the key that
                // defines the missing variable, which may not exist anywhere.
                match self.expand_value(&e.value, &above, &mut Vec::new(), 0, e.span.line) {
                    Ok(v) => strip_markers(&v),
                    Err(error) => {
                        pending[i] = Some(forward(error, &doc.entries[i + 1..]));
                        strip_markers(&e.value)
                    }
                }
            };
            let key = e.key.clone();
            above.insert(
                key.clone(),
                value.replace('$', &format!("{LITERAL_DOLLAR}$")),
            );
            current.insert(key, i);
            doc.entries[i].value = value;
        }

        // The entries that count: a failure there is always a problem.
        for &i in current.values() {
            if let Some(error) = pending[i].take() {
                let e = &doc.entries[i];
                problems.push(Problem {
                    span: e.span,
                    raw: e.raw.clone(),
                    error,
                });
            }
        }
        // Line order, after the line-pass problems: `problems[0]` stays the
        // error the strict parser returns.
        problems.sort_by_key(|p| p.span.line);
        doc.problems.extend(problems);
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
                    let inner: String = chars.by_ref().take_while(|&c| c != '}').collect();
                    let (var_name, op, word) = match split_operator(&inner) {
                        Some((name, op, word)) => (name.to_string(), op, word),
                        None => (inner.clone(), "", ""),
                    };

                    if stack.contains(&var_name) {
                        return Err(ParseError::CircularExpansion {
                            line: line_hint,
                            cycle: format!("{} → {}", stack.join(" → "), var_name),
                        });
                    }

                    let value = match vars.get(&var_name) {
                        Some(val) => {
                            stack.push(var_name.clone());
                            let expanded =
                                self.expand_value(val, vars, stack, depth + 1, line_hint)?;
                            stack.pop();
                            Some(expanded)
                        }
                        None => None,
                    };

                    // The shell's (and Compose's) meanings. A `:` also treats an
                    // empty value as unset. The word is expanded in turn, so
                    // `${A:-$B}` works.
                    let set = |colon: bool| value.as_ref().filter(|v| !colon || !v.is_empty());
                    let expanded = match op {
                        "" => match value {
                            Some(v) => v,
                            None => {
                                return Err(ParseError::UndefinedVariable {
                                    line: line_hint,
                                    var: var_name,
                                });
                            }
                        },
                        ":-" | "-" => match set(op == ":-") {
                            Some(v) => v.clone(),
                            None => self.expand_value(word, vars, stack, depth + 1, line_hint)?,
                        },
                        ":?" | "?" => match set(op == ":?") {
                            Some(v) => v.clone(),
                            None => {
                                let message = if !word.is_empty() {
                                    strip_markers(word)
                                } else if op == ":?" {
                                    "is not set or is empty".to_string()
                                } else {
                                    "is not set".to_string()
                                };
                                return Err(ParseError::RequiredVariable {
                                    line: line_hint,
                                    var: var_name,
                                    message,
                                });
                            }
                        },
                        // ":+" | "+"
                        _ => match set(op == ":+") {
                            Some(_) => {
                                self.expand_value(word, vars, stack, depth + 1, line_hint)?
                            }
                            None => String::new(),
                        },
                    };
                    result.push_str(&expanded);
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

/// An undefined variable that `below` assigns is a forward reference: say where
/// it is assigned, so the reader knows which line to move (D10).
fn forward(error: ParseError, below: &[Entry]) -> ParseError {
    match error {
        ParseError::UndefinedVariable { line, var } => match below.iter().find(|e| e.key == var) {
            Some(e) => ParseError::ForwardReference {
                line,
                var,
                assigned: e.span.line,
            },
            None => ParseError::UndefinedVariable { line, var },
        },
        other => other,
    }
}

/// Split `NAME:-word` (and `-`, `:?`, `?`, `:+`, `+`) into its parts.
///
/// `None` for a plain name, and for a word holding another `${`: the `}` that
/// closed this reference belonged to that one, so it stays the undefined
/// variable it was before these forms were read.
fn split_operator(inner: &str) -> Option<(&str, &str, &str)> {
    let end = inner.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    if end == 0 {
        return None;
    }
    let (name, rest) = inner.split_at(end);
    let op = rest.get(..if rest.starts_with(':') { 2 } else { 1 })?;
    let word = &rest[op.len()..];
    (matches!(op, ":-" | "-" | ":?" | "?" | ":+" | "+") && !word.contains("${"))
        .then_some((name, op, word))
}

/// The variable names `value` refers to, `${NAME…}` or `$NAME`, in order and
/// once each. A `$` written `\$` is not a reference.
pub(super) fn references(value: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == LITERAL_DOLLAR {
            chars.next_if_eq(&'$');
            continue;
        }
        if c != '$' {
            continue;
        }
        let name = if chars.next_if_eq(&'{').is_some() {
            let inner: String = chars.by_ref().take_while(|&c| c != '}').collect();
            split_operator(&inner).map_or(inner.clone(), |(n, _, _)| n.to_string())
        } else {
            std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '_'))
                .collect()
        };
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}
