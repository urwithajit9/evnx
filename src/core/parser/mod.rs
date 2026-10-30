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

    /// The 1-based line each key was written on.
    ///
    /// ⚠️ For a **multiline** value this is the line the value *opened* on, not
    /// the line a given fragment sits on — the same convention parse errors
    /// already use. A consumer reporting a position for a PEM block should say
    /// where the block starts, because that is where someone edits.
    ///
    /// Tracked internally since the expansion-error fix; exposed so `scan` can
    /// report `file:line (KEY)` without re-implementing `.env` parsing. That
    /// second implementation is what let a secret inside a multiline value go
    /// unreported — see `DEFECTS-FOUND.md` D21.
    pub lines: IndexMap<String, usize>,
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

mod expand;
mod lines;
#[cfg(test)]
mod tests;

use expand::strip_markers;
pub use lines::{closes_multiline, line_opens_multiline, opens_multiline};

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
        let (vars, lines) = self.parse_content_located(&content)?;
        Ok(EnvFile {
            vars,
            source: Some(source),
            lines,
        })
    }

    /// The hint for an `--env` file that does not exist, worded once for every
    /// command that reads one (`validate`, `diff`, `convert`; I-091).
    ///
    /// For `.env.<name>` it names `evnx init --env-name <name>` (and `-p` for
    /// another folder): plain `evnx init` writes `.env`, so the hint could not
    /// be followed (C-2, RM-17).
    pub fn missing_env_hint(path: &str) -> String {
        let Some(name) = crate::core::env_name::name_of(path) else {
            return "Create it with `evnx init`, or point at another file with --env.".into();
        };
        let dir = std::path::Path::new(path)
            .parent()
            .filter(|d| !d.as_os_str().is_empty() && *d != std::path::Path::new("."))
            .map(|d| format!(" -p {}", crate::core::shell::word(&d.to_string_lossy())))
            .unwrap_or_default();
        format!(
            "Create it with `evnx init --env-name {}{dir}`, or point at another file with --env.",
            crate::core::shell::word(name)
        )
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
    pub fn parse_content_located(
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
            self.expand_all(&mut vars, &literal_keys, &key_lines)?;
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

    /// Parse `content` into variables, discarding the line map.
    ///
    /// The shape 51 call sites already use. [`Parser::parse_content_located`]
    /// is the same work and also returns where each key was written — needed by
    /// anything that reports a position, which until D21 meant `scan` kept its
    /// own `.env` splitting instead.
    pub fn parse_content(&self, content: &str) -> ParseResult<IndexMap<String, String>> {
        self.parse_content_located(content).map(|(vars, _)| vars)
    }
}
