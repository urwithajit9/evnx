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

use indexmap::IndexMap;
use std::collections::HashSet;
// use std::collections::HashMap;
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
/// Is `key` a legal environment-variable name — `[A-Za-z_][A-Za-z0-9_]*`?
///
/// ⚠️ Exists so commands that **write** `.env` files can apply the same rule the
/// parser applies when reading them. `evnx add custom` took a name straight from
/// the prompt with no check, so typing `NODE_VERSION=22` produced the line
/// `# TODO: NODE_VERSION=22=22`, which this parser then refuses. A tool must not
/// be able to write a file it cannot read.
pub fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Error)]
pub enum ParseError {
    /// The file is not there at all.
    ///
    /// ⚠️ Separate from [`ParseError::FileReadError`] on purpose. A missing file
    /// used to arrive as "Failed to parse X: Failed to read file: No such file
    /// or directory", which sends the reader looking for a syntax error in a
    /// file that does not exist (issue #12).
    #[error("{path} does not exist")]
    FileNotFound { path: String },

    /// The file exists but could not be read — permissions, a directory, I/O.
    ///
    /// ⚠️ The message does **not** interpolate the source. thiserror's `#[from]`
    /// already sets `source()`, and anyhow prints the chain, so `{0}` here
    /// printed the OS error twice in a row.
    #[error("could not be read")]
    FileReadError(#[from] std::io::Error),

    /// A line did not contain a `=` separator (and is not a comment or blank).
    #[error("Invalid format at line {line}: {message}")]
    InvalidFormat { line: usize, message: String },

    /// A key contains characters outside `[A-Za-z_][A-Za-z0-9_]*`.
    #[error("Invalid key at line {line}: '{key}' (keys must match [A-Za-z_][A-Za-z0-9_]*)")]
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
    /// Changed from `HashMap` to `IndexMap` to preserve key order for
    /// predictable diff output and consistent user experience.
    pub vars: IndexMap<String, String>,

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
/// use evnx::core::parser::ParserConfig;
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

/// Marks a `$` that came from `\$` and must not be expanded.
///
/// U+FDD0 is a Unicode **noncharacter**: permanently reserved, never assigned, and
/// not valid in interchange — so it cannot appear in a `.env` anyone wrote. It exists
/// only between `unescape_double` and `expand_value`, and `strip_markers` removes any
/// that survive, so it never reaches a caller.
const LITERAL_DOLLAR: char = '\u{FDD0}';

/// Remove any [`LITERAL_DOLLAR`] marker.
///
/// Called on every path out of the parser, including the ones that cannot contain
/// one, so that no combination of settings can hand a caller an internal marker.
/// `expansion_enabled = false` is exactly such a path: it never runs `expand_value`,
/// which is what normally consumes them.
fn strip_markers(s: &str) -> String {
    if s.contains(LITERAL_DOLLAR) {
        s.replace(LITERAL_DOLLAR, "")
    } else {
        s.to_string()
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
    /// use evnx::core::Parser;
    ///
    /// let parser = Parser::default();
    /// let env_file = parser.parse_file(".env")?;
    /// println!("Loaded {} variables", env_file.vars.len());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn parse_file<P: AsRef<Path>>(&self, path: P) -> ParseResult<EnvFile> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ParseError::FileNotFound {
                    path: path.as_ref().display().to_string(),
                }
            } else {
                ParseError::FileReadError(e)
            }
        })?;
        let source = path.as_ref().to_string_lossy().into_owned();
        let vars = self.parse_content(&content)?;
        Ok(EnvFile {
            vars,
            source: Some(source),
        })
    }

    /// Parse a file, and say the right thing when it is not there.
    ///
    /// [`Parser::parse_file`] returns a structured error; every caller then
    /// wrapped it in `"Failed to parse {path}"`, which is a lie for a file that
    /// does not exist. This keeps that context for real parse failures and
    /// replaces it with `hint` — what to run to create the file — otherwise.
    pub fn parse_file_or_hint(&self, path: &str, hint: &str) -> anyhow::Result<EnvFile> {
        self.parse_file(path).map_err(|e| match e {
            ParseError::FileNotFound { .. } => anyhow::anyhow!("{e}\n\n{hint}"),
            other => anyhow::Error::from(other).context(format!("Failed to parse {path}")),
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
    /// use evnx::core::Parser;
    ///
    /// let parser = Parser::default();
    /// let vars = parser.parse_content("KEY=value\nOTHER=123")?;
    /// assert_eq!(vars["KEY"], "value");
    /// # Ok::<(), evnx::core::parser::ParseError>(())
    /// ```
    pub fn parse_content(&self, content: &str) -> ParseResult<IndexMap<String, String>> {
        let mut vars: IndexMap<String, String> = IndexMap::new();
        // Keys written in a form that promises to be literal — single quotes or
        // backticks. Tracked separately because the quote characters are gone by the
        // time `expand_all` runs over the assembled map.
        let mut literal_keys: HashSet<String> = HashSet::new();

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
            self.expand_all(&mut vars, &literal_keys)?;
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
    fn validate_key(&self, key: &str, line_num: usize) -> ParseResult<()> {
        if !is_valid_key(key) {
            return Err(ParseError::InvalidKey {
                line: line_num,
                key: key.to_string(),
            });
        }
        self.validate_key_strict(key, line_num)
    }

    /// Strict mode only: the rest of the original check.
    fn validate_key_strict(&self, key: &str, line_num: usize) -> ParseResult<()> {
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
    fn parse_value(&self, raw: &str, line_num: usize) -> ParseResult<(String, bool)> {
        let raw = raw.trim_start(); // leading whitespace after `=` is never significant

        if raw.is_empty() {
            return Ok((String::new(), false));
        }

        let first = raw.chars().next().unwrap(); // safe: checked is_empty above

        match first {
            '"' => {
                if !raw.ends_with('"') || raw.len() < 2 {
                    return Err(ParseError::UnterminatedString { line: line_num });
                }
                let inner = &raw[1..raw.len() - 1];
                Ok((self.unescape_double(inner), false))
            }

            '\'' | '`' => {
                if !raw.ends_with(first) || raw.len() < 2 {
                    return Err(ParseError::UnterminatedString { line: line_num });
                }
                // Single-quoted and backtick-quoted: literal content — no escaping,
                // and since v0.7.0 no expansion either. One rule instead of two.
                Ok((raw[1..raw.len() - 1].to_string(), true))
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
    fn expand_all(
        &self,
        vars: &mut IndexMap<String, String>,
        literal: &HashSet<String>,
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
            let result = self.expand_value(&value, vars, &mut stack, 0, 0)?;
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
    fn expand_value(
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
fn ends_with_closing_quote(s: &str, q: char) -> bool {
    let Some(body) = s.strip_suffix(q) else {
        return false;
    };
    if q != '"' {
        return true;
    }
    body.chars().rev().take_while(|&c| c == '\\').count() % 2 == 0
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
    // A lone quote character opens a value with nothing after it, so it cannot
    // also be the closing quote.
    if t.len() > q.len_utf8() && ends_with_closing_quote(t, q) {
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
    let line = line
        .strip_prefix("export")
        .map(|s| s.trim_start())
        .unwrap_or(line);
    let eq = line.find('=')?;
    opens_multiline(&line[eq + 1..])
}

/// The content of `line` before the quote that closes a multiline value opened
/// with `q`, or `None` if this line does not close it.
pub fn closes_multiline(line: &str, q: char) -> Option<&str> {
    let trimmed = line.trim_end();
    ends_with_closing_quote(trimmed, q).then(|| &trimmed[..trimmed.len() - q.len_utf8()])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── D16: a value containing `$` can be written ──────────────────────────
    //
    // Expansion used to run in every quoting form and there was no escape, so a
    // generated secret containing `${...}` made the file fail to parse and one
    // containing `$NAME`, where NAME was also a key, was silently replaced — with
    // another of your secrets, and with no error.

    fn parse_one(content: &str) -> IndexMap<String, String> {
        Parser::default().parse_content(content).expect("parses")
    }

    /// The headline: single quotes are now literal for `$`, as they already were
    /// for backslashes. One rule instead of two.
    #[test]
    fn single_quotes_do_not_expand() {
        let vars = parse_one("B=world\nA='cost $B'\nC=`cost $B`\n");
        assert_eq!(vars["A"], "cost $B");
        assert_eq!(vars["C"], "cost $B", "backticks too");
    }

    /// ⚠️ The failure that had no workaround. A token containing `${...}` used to
    /// make the **whole file** fail to parse, with no way to write it.
    #[test]
    fn a_braced_reference_in_single_quotes_is_not_an_undefined_variable() {
        let vars = parse_one("TOKEN='pa${SOMETHING}ss'\n");
        assert_eq!(vars["TOKEN"], "pa${SOMETHING}ss");
    }

    /// ⚠️ The quieter failure, and the worse one: no error, and the value that
    /// replaced it is another of your secrets.
    #[test]
    fn a_name_that_collides_with_a_key_is_no_longer_substituted() {
        let vars = parse_one("DB_PASS=hunter2\nAPI_KEY='prefix$DB_PASS'\n");
        assert_eq!(vars["API_KEY"], "prefix$DB_PASS");
        assert!(
            !vars["API_KEY"].contains("hunter2"),
            "leaked another secret"
        );
    }

    #[test]
    fn double_quoted_and_unquoted_still_expand() {
        let vars = parse_one("B=world\nA=\"cost $B\"\nC=cost $B\n");
        assert_eq!(vars["A"], "cost world");
        assert_eq!(vars["C"], "cost world");
    }

    /// `\$` is a literal dollar in a double-quoted value, so expansion can be kept
    /// for the rest of the string.
    #[test]
    fn an_escaped_dollar_is_literal() {
        let vars = parse_one("B=world\nA=\"\\$B costs $B\"\n");
        assert_eq!(vars["A"], "$B costs world");
    }

    /// ⚠️ The case the marker exists for, and the reason `\$` could not simply emit
    /// a `$`.
    ///
    /// `unescape_double` turns `\\` into `\` and would have turned `\$` into `$`,
    /// making "literal dollar" and "literal backslash before a real expansion"
    /// indistinguishable by the time expansion ran. This asserts the second still
    /// works, which a naive fix would have broken.
    #[test]
    fn a_backslash_before_a_real_expansion_still_expands() {
        let vars = parse_one("B=world\nA=\"path\\\\$B\"\n");
        assert_eq!(vars["A"], "path\\world");
    }

    /// The internal marker must never reach a caller, by any path.
    #[test]
    fn the_literal_dollar_marker_never_escapes() {
        for content in [
            "B=w\nA=\"\\$B\"\n",
            "A='\\$B'\n",
            "A=\"\\$\"\n",
            "B=w\nA=\"\\$B and $B and \\$B\"\n",
        ] {
            for (k, v) in parse_one(content) {
                assert!(
                    !v.contains(LITERAL_DOLLAR),
                    "{k} leaked the marker from {content:?}: {v:?}"
                );
            }
        }
    }

    /// ⚠️ The path `expand_all` does not cover. With expansion disabled nothing
    /// consumes the markers, so the strip has to be unconditional.
    #[test]
    fn the_marker_is_stripped_even_with_expansion_disabled() {
        let parser = Parser::new(ParserConfig {
            allow_expansion: false,
            ..Default::default()
        });
        let vars = parser
            .parse_content("B=world\nA=\"\\$B\"\n")
            .expect("parses");
        assert!(!vars["A"].contains(LITERAL_DOLLAR), "{:?}", vars["A"]);
        assert_eq!(vars["A"], "$B");
    }

    /// A multiline single-quoted value is literal in every respect — escapes and
    /// expansion alike.
    #[test]
    fn a_multiline_single_quoted_value_is_fully_literal() {
        let vars = parse_one("B=world\nA='line $B\nsecond \\t line'\n");
        assert_eq!(vars["A"], "line $B\nsecond \\t line");
    }

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

    /// B5. An escaped `\"` inside a value that spans lines used to end the value,
    /// because the closing-quote check was a bare `ends_with('"')` and the final
    /// character of `"line1 \"q\"` *is* a quote. The next line then became a
    /// stray entry: `Invalid format at line 2: missing '=' separator`.
    #[test]
    fn escaped_quote_does_not_end_a_multiline_value() {
        let p = Parser::default();
        let vars = p
            .parse_content("B=\"line1 \\\"q\\\"\nline2\"\nAFTER=ok\n")
            .unwrap();
        assert_eq!(vars["B"], "line1 \"q\"\nline2");
        // Parsing must continue past it — the original symptom was the *next*
        // line being misread, not the value itself.
        assert_eq!(vars["AFTER"], "ok");
    }

    /// The same value, on one line and across two, must mean the same thing.
    ///
    /// ⚠️ It did not. The multiline branch inserted its accumulated text directly
    /// and never reached `parse_value`, so `"x\ty"` produced a tab on one line and
    /// a literal backslash-`t` across two — and `\"` was inexpressible in a
    /// multiline value at all, which is the other half of B5.
    #[test]
    fn escape_sequences_mean_the_same_across_lines() {
        let p = Parser::default();

        let one = p.parse_content("V=\"x\\ty and \\\"q\\\"\"\n").unwrap();
        let two = p.parse_content("V=\"x\\ty and \\\"q\\\"\nz\"\n").unwrap();

        assert_eq!(one["V"], "x\ty and \"q\"");
        assert_eq!(two["V"], "x\ty and \"q\"\nz");
        // The spanning value is the single-line one plus a newline and `z`.
        assert_eq!(two["V"], format!("{}\nz", one["V"]));
    }

    /// Single quotes are literal by definition, so a backslash inside them
    /// survives verbatim whether or not the value spans lines. This is the escape
    /// hatch the README points at for content that must not be unescaped.
    #[test]
    fn single_quoted_multiline_stays_literal() {
        let p = Parser::default();
        let vars = p.parse_content("S='a\\nb\nsecond'\n").unwrap();
        // A literal backslash and `n`, then a real newline from the line break.
        assert_eq!(vars["S"], "a\\nb\nsecond");
    }

    /// A PEM key is the common case and contains no backslashes, so the escape
    /// change above must leave it byte-identical.
    #[test]
    fn pem_key_is_unaffected_by_escape_handling() {
        let p = Parser::default();
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADAN\n-----END PRIVATE KEY-----";
        let vars = p.parse_content(&format!("P=\"{pem}\"\nNEXT=1\n")).unwrap();
        assert_eq!(vars["P"], pem);
        assert_eq!(vars["NEXT"], "1");
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
    // New test demonstrating order preservation:
    #[test]
    fn test_key_order_preserved() {
        let p = Parser::default();
        let content = "Z=last\nA=first\nM=middle";
        let vars = p.parse_content(content).unwrap();

        let keys: Vec<_> = vars.keys().collect();
        assert_eq!(keys, vec!["Z", "A", "M"]); // Insertion order preserved
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
