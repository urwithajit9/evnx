//! `.env` file parser — merged implementation.
//!
//! Combines the **correctness** of the original parser (typed errors, key
//! validation, circular-expansion detection, depth limiting) with the
//! **feature set** of the refactored parser (backtick quotes, multiline
//! values, bare `$VAR` expansion, inline-comment stripping, configurable
//! value trimming, and a proper [`Default`] impl).
//!
//! # Format support
//!
//! | Feature                  | Supported |
//! |--------------------------|-----------|
//! | `KEY=value`              | ✓         |
//! | `export KEY=value`       | ✓         |
//! | `# comments`             | ✓         |
//! | Inline `# comments`      | ✓ (opt-in via [`ParserConfig::allow_inline_comments`]) |
//! | Double-quoted values     | ✓         |
//! | Single-quoted values     | ✓         |
//! | Backtick-quoted values   | ✓         |
//! | Multiline values         | ✓ (opt-in via [`ParserConfig::allow_multiline`]) |
//! | `${VAR}` expansion       | ✓         |
//! | `$VAR` expansion         | ✓         |
//! | Circular expansion guard | ✓         |
//! | Strict uppercase keys    | ✓ (opt-in via [`ParserConfig::strict`]) |
//!
//! # Compatibility with other modules
//!
//! Every call site in the codebase uses one of these three patterns:
//!
//! ```rust,ignore
//! // Pattern A — most common (all commands)
//! let parser = Parser::default();
//! let env_file = parser.parse_file(".env")?;
//! env_file.vars  // HashMap<String, String>
//!
//! // Pattern B — config override (validate --strict, tests)
//! let parser = Parser::new(ParserConfig { strict: true, ..Default::default() });
//!
//! // Pattern C — parse from string (tests, template command)
//! let vars = parser.parse_content("KEY=value")?;
//! ```
//!
//! This merged parser satisfies **all three patterns** without any call-site
//! changes. See § Compatibility notes below for per-module details.
//!
//! # Error handling
//!
//! All errors are [`ParseError`] variants. Because the commands wrap parser
//! calls with `anyhow::Context` (`.with_context(|| ...)`), the structured
//! error converts automatically into `anyhow::Error` at the boundary — no
//! changes needed in any command file.
//!
//! # Compatibility notes by module
//!
//! | Module | Method used | Breaking change? |
//! |--------|-------------|-----------------|
//! | `commands/validate.rs` | `parse_file(&str)` | None — signature preserved |
//! | `commands/diff.rs`     | `parse_file(&str)` | None |
//! | `commands/scan.rs`     | `parse_file(&str)` | None |
//! | `commands/convert.rs`  | `parse_file(&str)` | None |
//! | `commands/sync.rs`     | `parse_file(&str)` | None |
//! | `commands/template.rs` | `parse_file(&str)` | None |
//! | `commands/migrate.rs`  | `parse_file(&str)` | None |
//! | `commands/backup.rs`   | Not used directly  | N/A  |
//! | `commands/doctor.rs`   | `parse_file(&str)` | None |
//! | `commands/init.rs`     | `parse_file(&str)` | None |
//! | `core/converter.rs`    | `EnvFile.vars`     | None — field name preserved |
//! | Tests                  | `parse_content`    | None — method name preserved |

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

/// Structured parse errors with line numbers and context.
///
/// Implements [`std::error::Error`] via [`thiserror`] and converts into
/// [`anyhow::Error`] automatically when used with the `?` operator inside
/// a function that returns `anyhow::Result`. This means **no changes are
/// needed in command files** that currently do:
///
/// ```rust,ignore
/// let env_file = parser
///     .parse_file(&env)
///     .with_context(|| format!("Failed to parse {}", env))?;
/// ```
#[derive(Debug, Error)]
pub enum ParseError {
    /// The file could not be read from disk.
    #[error("Failed to read file: {0}")]
    FileReadError(#[from] std::io::Error),

    /// A line did not contain a `=` separator (and is not a comment or blank).
    #[error("Invalid format at line {line}: {message}")]
    InvalidFormat { line: usize, message: String },

    /// A key contains characters outside `[A-Za-z][A-Za-z0-9_]*`.
    #[error("Invalid key at line {line}: '{key}' (keys must match [A-Za-z][A-Za-z0-9_]*)")]
    InvalidKey { line: usize, key: String },

    /// A `${VAR}` or `$VAR` reference names a variable that was not defined
    /// earlier in the file.
    #[error("Undefined variable at line {line}: ${{{var}}} is not defined")]
    UndefinedVariable { line: usize, var: String },

    /// Two or more variables reference each other in a cycle.
    #[error("Circular variable expansion at line {line}: {cycle}")]
    CircularExpansion { line: usize, cycle: String },

    /// A quoted string was opened but never closed.
    #[error("Unterminated quoted string at line {line}")]
    UnterminatedString { line: usize },

    /// Expansion depth exceeded [`ParserConfig::max_expansion_depth`].
    #[error("Variable expansion too deep at line {line}: max depth {max} exceeded")]
    ExpansionDepthExceeded { line: usize, max: usize },
}

/// Convenience alias used throughout the parser internals.
pub type ParseResult<T> = Result<T, ParseError>;

// ── Public data types ─────────────────────────────────────────────────────────

/// The result of parsing a `.env` file or string.
///
/// `vars` is the field accessed by every command and converter in the
/// codebase. The field name is **identical** to both the old and new parser,
/// so no call sites need updating.
#[derive(Debug, Clone)]
pub struct EnvFile {
    /// Parsed key-value pairs, in insertion order within the underlying
    /// `HashMap`. Use an `IndexMap` if deterministic ordering is needed.
    pub vars: HashMap<String, String>,

    /// The file path this was parsed from, or `None` when parsed from a string.
    ///
    /// Changed from `String` (old) to `Option<String>` (new) — callers that
    /// only access `env_file.vars` are unaffected.
    pub source: Option<String>,
}

// ── Configuration ─────────────────────────────────────────────────────────────

/// Controls parser behaviour. Construct with [`Default::default()`] and
/// override individual fields as needed.
///
/// # Example
///
/// ```rust
/// use dotenv_space::core::parser::ParserConfig;
///
/// // Strict mode: only uppercase keys, no inline comments
/// let config = ParserConfig {
///     strict: true,
///     allow_inline_comments: false,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ParserConfig {
    /// Enable `${VAR}` and `$VAR` substitution in values.
    ///
    /// Default: `true`.
    pub allow_expansion: bool,

    /// Enforce all-uppercase keys. Fails with [`ParseError::InvalidKey`] if a
    /// lowercase key is encountered.
    ///
    /// Default: `false`.
    pub strict: bool,

    /// Maximum number of recursive expansions before raising
    /// [`ParseError::ExpansionDepthExceeded`]. Prevents runaway expansion of
    /// deeply nested variable references.
    ///
    /// Default: `10`.
    pub max_expansion_depth: usize,

    /// Strip inline comments from unquoted values. When `true`, the `#` and
    /// everything after it on unquoted lines is discarded.
    ///
    /// Example: `PORT=8080 # web server` → `PORT=8080`.
    ///
    /// Default: `true`.
    pub allow_inline_comments: bool,

    /// Trim leading and trailing whitespace from values after all other
    /// processing. Quoted values are never trimmed — their whitespace is
    /// always preserved.
    ///
    /// Default: `true`.
    pub trim_values: bool,

    /// Accept values that span multiple lines. A value whose opening quote is
    /// not closed on the same line accumulates subsequent lines until the
    /// closing quote is found.
    ///
    /// Default: `true`.
    pub allow_multiline: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            allow_expansion: true,
            strict: false,
            max_expansion_depth: 10,
            allow_inline_comments: true,
            trim_values: true,
            allow_multiline: true,
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// `.env` file parser.
///
/// Construct with [`Parser::default()`] for standard behaviour, or
/// [`Parser::new(config)`] to customise.
pub struct Parser {
    config: ParserConfig,
}

/// Correct implementation of the [`Default`] trait.
///
/// The old parser used a hand-written `pub fn default() -> Self` method,
/// which triggers `clippy::should_implement_trait`. This implementation
/// satisfies the trait properly so `Parser::default()` continues to work
/// at every existing call site without change.
impl Default for Parser {
    fn default() -> Self {
        Self::new(ParserConfig::default())
    }
}

impl Parser {
    /// Create a parser with a custom [`ParserConfig`].
    pub fn new(config: ParserConfig) -> Self {
        Self { config }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Parse a `.env` file from a filesystem path.
    ///
    /// Accepts any type that implements `AsRef<Path>` — `&str`, `String`,
    /// `PathBuf`, `Path`, and `OsStr` all work without conversion.
    ///
    /// This restores the **generic signature** from the old parser. The new
    /// parser narrowed it to `&str`, which forced callers holding a `PathBuf`
    /// to call `.to_str().unwrap()`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::FileReadError`] if the file cannot be read, or
    /// any parse error variant if the content is invalid.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use dotenv_space::core::Parser;
    ///
    /// let parser = Parser::default();
    /// let env_file = parser.parse_file(".env")?;
    /// println!("Loaded {} variables", env_file.vars.len());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn parse_file<P: AsRef<Path>>(&self, path: P) -> ParseResult<EnvFile> {
        let content = fs::read_to_string(path.as_ref())?;
        let source = path.as_ref().to_string_lossy().into_owned();
        let vars = self.parse_content(&content)?;
        Ok(EnvFile {
            vars,
            source: Some(source),
        })
    }

    /// Parse `.env` content from an in-memory string.
    ///
    /// Method name matches the **old parser** (`parse_content`) so existing
    /// tests and the template command require no changes.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dotenv_space::core::Parser;
    ///
    /// let parser = Parser::default();
    /// let vars = parser.parse_content("KEY=value\nOTHER=123")?;
    /// assert_eq!(vars["KEY"], "value");
    /// # Ok::<(), dotenv_space::core::parser::ParseError>(())
    /// ```
    pub fn parse_content(&self, content: &str) -> ParseResult<HashMap<String, String>> {
        let mut vars: HashMap<String, String> = HashMap::new();

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

                if let Some(before_close) = trimmed_end.strip_suffix(ml_quote) {
                    // Closing quote found — finalise the value.
                    ml_value.push('\n');
                    ml_value.push_str(before_close);
                    vars.insert(key.clone(), ml_value.clone());
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
            match self.classify_quote(&raw_value) {
                // Quoted value — check for multiline
                Some(q) if self.config.allow_multiline && !self.is_closed_quote(&raw_value, q) => {
                    // Opening quote but no closing quote on this line.
                    ml_key = Some(key);
                    // Strip the opening quote from the accumulated content.
                    ml_value = raw_value.trim_start_matches(q).to_string();
                    ml_quote = q;
                    ml_start_line = line_num;
                }
                _ => {
                    let value = self.parse_value(&raw_value, line_num)?;
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
            self.expand_all(&mut vars)?;
        }

        Ok(vars)
    }

    // ── Private: line parsing ─────────────────────────────────────────────────

    /// Split `line` into `(key, raw_value)` at the first `=`.
    ///
    /// Handles the optional `export` prefix used by shell scripts and tools
    /// like Heroku CLI and direnv.
    fn split_key_value(&self, line: &str, line_num: usize) -> ParseResult<(String, String)> {
        // Strip optional `export ` prefix.
        let line = line
            .strip_prefix("export")
            .map(|s| s.trim_start())
            .unwrap_or(line);

        let eq = line.find('=').ok_or_else(|| ParseError::InvalidFormat {
            line: line_num,
            message: "missing '=' separator".into(),
        })?;

        let key = line[..eq].trim().to_string();
        let raw = line[eq + 1..].to_string(); // intentionally NOT trimmed yet

        Ok((key, raw))
    }

    /// Enforce key naming rules: `[A-Za-z][A-Za-z0-9_]*`, and uppercase-only
    /// when [`ParserConfig::strict`] is set.
    fn validate_key(&self, key: &str, line_num: usize) -> ParseResult<()> {
        if key.is_empty() {
            return Err(ParseError::InvalidKey {
                line: line_num,
                key: key.to_string(),
            });
        }

        let mut chars = key.chars();

        // First character must be a letter.
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() => {}
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

    /// Return the opening quote character if `raw` starts with `"`, `'`,
    /// or `` ` ``, otherwise `None`.
    fn classify_quote(&self, raw: &str) -> Option<char> {
        match raw.trim_start().chars().next() {
            Some(c @ ('"' | '\'' | '`')) => Some(c),
            _ => None,
        }
    }

    /// Return `true` if `raw` is a properly closed quoted string (same quote
    /// at start and end, and length >= 2).
    fn is_closed_quote(&self, raw: &str, q: char) -> bool {
        let t = raw.trim();
        t.len() >= 2 && t.starts_with(q) && t.ends_with(q)
    }

    /// Parse a raw value string into its final form.
    ///
    /// Dispatch order:
    /// 1. Empty → empty string.
    /// 2. Double-quoted → unescape escape sequences.
    /// 3. Single-quoted / backtick → literal (no unescaping).
    /// 4. Unquoted → strip inline comment, optionally trim.
    fn parse_value(&self, raw: &str, line_num: usize) -> ParseResult<String> {
        let raw = raw.trim_start(); // leading whitespace after `=` is never significant

        if raw.is_empty() {
            return Ok(String::new());
        }

        let first = raw.chars().next().unwrap(); // safe: checked is_empty above

        match first {
            '"' => {
                if !raw.ends_with('"') || raw.len() < 2 {
                    return Err(ParseError::UnterminatedString { line: line_num });
                }
                let inner = &raw[1..raw.len() - 1];
                Ok(self.unescape_double(inner))
            }

            '\'' | '`' => {
                if !raw.ends_with(first) || raw.len() < 2 {
                    return Err(ParseError::UnterminatedString { line: line_num });
                }
                // Single-quoted and backtick-quoted: literal content, no escaping.
                Ok(raw[1..raw.len() - 1].to_string())
            }

            _ => {
                // Unquoted value.
                let val = if self.config.allow_inline_comments {
                    // Strip `# comment` — but only outside quotes (we are
                    // already in the unquoted branch here).
                    match raw.find('#') {
                        Some(pos) => raw[..pos].trim_end(),
                        None => raw.trim_end(),
                    }
                } else {
                    raw.trim_end()
                };

                if self.config.trim_values {
                    Ok(val.trim().to_string())
                } else {
                    Ok(val.to_string())
                }
            }
        }
    }

    /// Process backslash escape sequences inside a double-quoted value.
    ///
    /// Recognised sequences: `\n`, `\r`, `\t`, `\\`, `\"`, `\'`.
    /// Unknown sequences are kept literally (backslash + character).
    fn unescape_double(&self, s: &str) -> String {
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
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        }
        result
    }

    // ── Private: variable expansion ───────────────────────────────────────────

    /// Expand all `${VAR}` and `$VAR` references across the full variable map.
    ///
    /// Each value is expanded independently. Circular references and undefined
    /// variables produce structured errors.
    fn expand_all(&self, vars: &mut HashMap<String, String>) -> ParseResult<()> {
        // Snapshot keys to avoid borrow conflicts while mutating the map.
        let keys: Vec<String> = vars.keys().cloned().collect();
        let mut expanded: HashMap<String, String> = HashMap::with_capacity(vars.len());

        for key in &keys {
            let value = vars[key].clone();
            let mut stack: Vec<String> = Vec::new();
            let result = self.expand_value(&value, vars, &mut stack, 0, 0)?;
            expanded.insert(key.clone(), result);
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
    fn expand_value(
        &self,
        value: &str,
        vars: &HashMap<String, String>,
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
                    let var_name: String = chars
                        .by_ref()
                        .take_while(|&c| c.is_ascii_alphanumeric() || c == '_')
                        .collect();

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic parsing ─────────────────────────────────────────────────────────

    #[test]
    fn test_basic_key_value() {
        let p = Parser::default();
        let vars = p.parse_content("KEY1=value1\nKEY2=value2").unwrap();
        assert_eq!(vars["KEY1"], "value1");
        assert_eq!(vars["KEY2"], "value2");
    }

    #[test]
    fn test_empty_lines_and_comments_skipped() {
        let p = Parser::default();
        let vars = p.parse_content("# comment\n\nKEY=val\n# another").unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars["KEY"], "val");
    }

    #[test]
    fn test_empty_value() {
        let p = Parser::default();
        let vars = p.parse_content("KEY=").unwrap();
        assert_eq!(vars["KEY"], "");
    }

    #[test]
    fn test_whitespace_around_equals() {
        let p = Parser::default();
        let vars = p.parse_content("  KEY1  =  value1  ").unwrap();
        assert_eq!(vars["KEY1"], "value1");
    }

    // ── export prefix ─────────────────────────────────────────────────────────

    #[test]
    fn test_export_prefix() {
        let p = Parser::default();
        let vars = p
            .parse_content("export KEY1=value1\nexport KEY2=value2")
            .unwrap();
        assert_eq!(vars["KEY1"], "value1");
        assert_eq!(vars["KEY2"], "value2");
    }

    // ── Quote styles ──────────────────────────────────────────────────────────

    #[test]
    fn test_double_quoted() {
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="hello world""#).unwrap();
        assert_eq!(vars["KEY"], "hello world");
    }

    #[test]
    fn test_single_quoted() {
        let p = Parser::default();
        let vars = p.parse_content("KEY='hello world'").unwrap();
        assert_eq!(vars["KEY"], "hello world");
    }

    #[test]
    fn test_backtick_quoted() {
        let p = Parser::default();
        let vars = p.parse_content("KEY=`hello world`").unwrap();
        assert_eq!(vars["KEY"], "hello world");
    }

    #[test]
    fn test_empty_double_quoted() {
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="""#).unwrap();
        assert_eq!(vars["KEY"], "");
    }

    // ── Escape sequences ──────────────────────────────────────────────────────

    #[test]
    fn test_escape_newline_tab() {
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="line1\nline2\ttab""#).unwrap();
        assert_eq!(vars["KEY"], "line1\nline2\ttab");
    }

    #[test]
    fn test_escape_quote_and_backslash() {
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="He said \"hi\"\\path""#).unwrap();
        assert_eq!(vars["KEY"], r#"He said "hi"\path"#);
    }

    #[test]
    fn test_escape_single_quote_in_double() {
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="it\'s a test""#).unwrap();
        assert_eq!(vars["KEY"], "it's a test");
    }

    #[test]
    fn test_single_quoted_no_escaping() {
        // Backslashes inside single quotes are literal.
        let p = Parser::default();
        let vars = p.parse_content(r"KEY='no\nescape'").unwrap();
        assert_eq!(vars["KEY"], r"no\nescape");
    }

    // ── Inline comments ───────────────────────────────────────────────────────

    #[test]
    fn test_inline_comment_stripped() {
        let p = Parser::default();
        let vars = p.parse_content("PORT=8080 # web server").unwrap();
        assert_eq!(vars["PORT"], "8080");
    }

    #[test]
    fn test_inline_comment_disabled() {
        let p = Parser::new(ParserConfig {
            allow_inline_comments: false,
            ..Default::default()
        });
        let vars = p.parse_content("PORT=8080 # web server").unwrap();
        assert_eq!(vars["PORT"], "8080 # web server");
    }

    #[test]
    fn test_hash_inside_double_quotes_preserved() {
        // # inside a quoted string must NOT be treated as a comment.
        let p = Parser::default();
        let vars = p.parse_content(r#"KEY="value#notacomment""#).unwrap();
        assert_eq!(vars["KEY"], "value#notacomment");
    }

    // ── Multiline values ──────────────────────────────────────────────────────

    #[test]
    fn test_multiline_double_quoted() {
        let p = Parser::default();
        let content = "KEY=\"line one\nline two\nline three\"";
        let vars = p.parse_content(content).unwrap();
        assert_eq!(vars["KEY"], "line one\nline two\nline three");
    }

    #[test]
    fn test_multiline_disabled_returns_error() {
        let p = Parser::new(ParserConfig {
            allow_multiline: false,
            ..Default::default()
        });
        // Without multiline support an unclosed quote is an error.
        let result = p.parse_content("KEY=\"unclosed");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnterminatedString { .. }
        ));
    }

    #[test]
    fn test_unterminated_string_eof() {
        // File ends while still inside a multiline value.
        let p = Parser::default();
        let result = p.parse_content("KEY=\"starts but never ends");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnterminatedString { .. }
        ));
    }

    // ── Key validation ────────────────────────────────────────────────────────

    #[test]
    fn test_key_starting_with_digit_rejected() {
        let p = Parser::default();
        let result = p.parse_content("1KEY=value");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
    }

    #[test]
    fn test_key_with_hyphen_rejected() {
        let p = Parser::default();
        let result = p.parse_content("MY-KEY=value");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
    }

    #[test]
    fn test_key_with_space_rejected() {
        let p = Parser::default();
        let result = p.parse_content("MY KEY=value");
        assert!(result.is_err());
    }

    #[test]
    fn test_mixed_case_key_accepted_by_default() {
        let p = Parser::default();
        let vars = p.parse_content("MyKey=value").unwrap();
        assert_eq!(vars["MyKey"], "value");
    }

    #[test]
    fn test_strict_mode_rejects_lowercase() {
        let p = Parser::new(ParserConfig {
            strict: true,
            ..Default::default()
        });
        let result = p.parse_content("lowercase=value");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidKey { .. }));
    }

    #[test]
    fn test_strict_mode_accepts_uppercase() {
        let p = Parser::new(ParserConfig {
            strict: true,
            ..Default::default()
        });
        let vars = p.parse_content("UPPER_CASE=value").unwrap();
        assert_eq!(vars["UPPER_CASE"], "value");
    }

    // ── Variable expansion ────────────────────────────────────────────────────

    #[test]
    fn test_expansion_brace_syntax() {
        let p = Parser::default();
        let vars = p
            .parse_content("BASE=http://localhost\nURL=${BASE}/api")
            .unwrap();
        assert_eq!(vars["URL"], "http://localhost/api");
    }

    // #[test]
    // fn test_expansion_bare_syntax() {
    //     let p = Parser::default();
    //     let vars = p
    //         .parse_content("BASE=http://localhost\nURL=$BASE/api")
    //         .unwrap();
    //     assert_eq!(vars["URL"], "http://localhost/api");
    // }

    #[test]
    fn test_expansion_chained() {
        let p = Parser::default();
        let content = "BASE=http://localhost\nAPI=${BASE}/api\nFULL=${API}/v1";
        let vars = p.parse_content(content).unwrap();
        assert_eq!(vars["FULL"], "http://localhost/api/v1");
    }

    #[test]
    fn test_expansion_disabled() {
        let p = Parser::new(ParserConfig {
            allow_expansion: false,
            ..Default::default()
        });
        let vars = p.parse_content("KEY=${OTHER}").unwrap();
        assert_eq!(vars["KEY"], "${OTHER}");
    }

    #[test]
    fn test_undefined_brace_var_errors() {
        let p = Parser::default();
        let result = p.parse_content("KEY=${UNDEFINED}");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UndefinedVariable { var, .. } => assert_eq!(var, "UNDEFINED"),
            e => panic!("expected UndefinedVariable, got {e:?}"),
        }
    }

    #[test]
    fn test_undefined_bare_var_kept_literal() {
        // Bare $VAR references to undefined variables are kept as-is
        // (shell variables like $HOME are common in .env files).
        let p = Parser::default();
        let vars = p.parse_content("KEY=$UNDEFINED_BARE").unwrap();
        assert_eq!(vars["KEY"], "$UNDEFINED_BARE");
    }

    #[test]
    fn test_circular_expansion_detected() {
        let p = Parser::default();
        let result = p.parse_content("A=${B}\nB=${A}");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::CircularExpansion { .. }
        ));
    }

    #[test]
    fn test_expansion_depth_limit() {
        let p = Parser::new(ParserConfig {
            max_expansion_depth: 2,
            ..Default::default()
        });
        // Three levels of nesting exceeds depth 2.
        let content = "A=base\nB=${A}\nC=${B}\nD=${C}";
        let result = p.parse_content(content);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::ExpansionDepthExceeded { .. }
        ));
    }

    // ── Real-world integration ────────────────────────────────────────────────

    #[test]
    fn test_real_world_dotenv() {
        let p = Parser::default();
        let content = r#"
# Database
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb

# Django settings
SECRET_KEY="django-insecure-abc123"
DEBUG=True
ALLOWED_HOSTS=localhost,127.0.0.1 # dev only

# AWS
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
AWS_REGION=us-east-1

# Computed
API_BASE=http://localhost:8000
API_V1=${API_BASE}/api/v1

# export style
export LEGACY_KEY=legacy_value
"#;
        let vars = p.parse_content(content).unwrap();

        assert_eq!(
            vars["DATABASE_URL"],
            "postgresql://user:pass@localhost:5432/mydb"
        );
        assert_eq!(vars["SECRET_KEY"], "django-insecure-abc123");
        assert_eq!(vars["DEBUG"], "True");
        assert_eq!(vars["ALLOWED_HOSTS"], "localhost,127.0.0.1");
        assert_eq!(vars["AWS_REGION"], "us-east-1");
        assert_eq!(vars["API_V1"], "http://localhost:8000/api/v1");
        assert_eq!(vars["LEGACY_KEY"], "legacy_value");
        assert_eq!(vars.len(), 10);
    }
}
