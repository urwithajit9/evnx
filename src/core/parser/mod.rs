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

    /// `${VAR:?message}` or `${VAR?message}` named a variable that is not set
    /// (or, with the `:`, is empty).
    #[error("Required variable at line {line}: ${{{var}}} {message}")]
    RequiredVariable {
        line: usize,
        var: String,
        message: String,
    },

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

mod document;
mod expand;
#[cfg(test)]
mod legacy;
mod lines;
#[cfg(test)]
mod tests;

pub use document::{AssignedBelow, Comment, Document, Entry, OrderDifference, Problem, Span};
use expand::strip_markers;
pub use lines::{closes_multiline, line_opens_multiline, opens_multiline, strip_export};

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
        let source = path.as_ref().to_string_lossy().into_owned();
        let (vars, lines) = self.read_document(path)?.into_result()?;
        Ok(EnvFile {
            vars,
            source: Some(source),
            lines,
        })
    }

    /// Read a file into a [`Document`]. Fails only when the file cannot be
    /// read; what is wrong *inside* it is in the document's `problems`.
    pub fn read_document<P: AsRef<Path>>(&self, path: P) -> ParseResult<Document> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ParseError::FileNotFound {
                    path: path.as_ref().display().to_string(),
                }
            } else {
                ParseError::FileReadError(e)
            }
        })?;
        Ok(self.parse_document(&content))
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
        self.parse_document(content).into_result()
    }

    /// Read `content` into a [`Document`], recording every problem instead of
    /// stopping at the first one.
    ///
    /// A bad line becomes a [`Problem`] and reading carries on at the next line.
    /// An unterminated quote is the exception: as in the strict parser it takes
    /// the rest of the file, so the problem spans to the end.
    pub fn parse_document(&self, content: &str) -> Document {
        let mut doc = Document::default();
        // Keys written in a form that promises to be literal — single quotes or
        // backticks. Tracked by name because expansion is a later pass.
        //
        // ⚠️ By name, and never removed: `A='x'` then `A=$B` leaves `A`
        // unexpanded. That is what the parser has always done; changing it is a
        // decision of its own, not part of this refactor.
        let mut literal_keys: HashSet<String> = HashSet::new();

        // A value still open across lines.
        struct Open {
            key: String,
            exported: bool,
            quote: char,
            start: usize,
            value: String,
            raw: String,
        }
        let mut open: Option<Open> = None;
        let mut last_line = 0;

        // A UTF-8 byte-order mark is an encoding marker, not text: Windows
        // editors write one, and Docker Compose, Node dotenv and `docker run`
        // skip it. Read as text it became part of the first key, and the file
        // was refused with an invalid key whose fault no one could see.
        let content = match content.strip_prefix('\u{feff}') {
            Some(rest) => {
                doc.bom = true;
                rest
            }
            None => content,
        };

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1; // 1-indexed for all user-facing messages
            last_line = line_num;

            // ── Multiline continuation ────────────────────────────────────────
            if let Some(o) = open.as_mut() {
                o.raw.push('\n');
                o.raw.push_str(raw_line);
                o.value.push('\n');
                // ⚠️ `closes_multiline` honours `\"`; a bare `strip_suffix` closed
                // the value on an escaped quote (B5).
                let Some(before_close) = closes_multiline(raw_line.trim_end(), o.quote) else {
                    o.value.push_str(raw_line);
                    continue;
                };
                o.value.push_str(before_close);
                let o = open.take().expect("matched above");
                // ⚠️ Escapes mean the same on one line or several; a multiline
                // single-quoted value is literal in every respect (B5).
                let value = if o.quote == '"' {
                    self.unescape_double(&o.value)
                } else {
                    literal_keys.insert(o.key.clone());
                    o.value
                };
                doc.entries.push(Entry {
                    key: o.key,
                    value,
                    span: Span {
                        line: o.start,
                        end_line: line_num,
                    },
                    raw: o.raw,
                    exported: o.exported,
                    quote: Some(o.quote),
                });
                continue;
            }

            // ── Blank lines and full-line comments ────────────────────────────
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(text) = line.strip_prefix('#') {
                doc.comments.push(Comment {
                    line: line_num,
                    text: text.to_string(),
                });
                continue;
            }

            // ── KEY=VALUE ─────────────────────────────────────────────────────
            let parsed = self
                .split_key_value(line, line_num)
                .and_then(|(key, raw_value)| {
                    self.validate_key(&key, line_num)?;
                    Ok((key, raw_value))
                });
            let (key, raw_value) = match parsed {
                Ok(kv) => kv,
                Err(error) => {
                    doc.problems.push(Problem {
                        span: Span::line(line_num),
                        raw: raw_line.to_string(),
                        error,
                    });
                    continue;
                }
            };
            // The same test `split_key_value` strips the prefix with.
            let exported = strip_export(line).is_some();

            match opens_multiline(&raw_value) {
                Some(q) if self.config.allow_multiline => {
                    open = Some(Open {
                        key,
                        exported,
                        quote: q,
                        start: line_num,
                        value: raw_value.trim_start_matches(q).to_string(),
                        raw: raw_line.to_string(),
                    });
                }
                _ => match self.parse_value(&raw_value, line_num) {
                    Ok((value, is_literal)) => {
                        if is_literal {
                            literal_keys.insert(key.clone());
                        }
                        let quote = raw_value
                            .trim_start()
                            .chars()
                            .next()
                            .filter(|c| matches!(c, '"' | '\'' | '`'));
                        doc.entries.push(Entry {
                            key,
                            value,
                            span: Span::line(line_num),
                            raw: raw_line.to_string(),
                            exported,
                            quote,
                        });
                    }
                    Err(error) => doc.problems.push(Problem {
                        span: Span::line(line_num),
                        raw: raw_line.to_string(),
                        error,
                    }),
                },
            }
        }

        // The file ended inside a quoted value.
        if let Some(o) = open {
            doc.problems.push(Problem {
                span: Span {
                    line: o.start,
                    end_line: last_line,
                },
                raw: o.raw,
                error: ParseError::UnterminatedString { line: o.start },
            });
        }

        // ── Variable expansion ────────────────────────────────────────────────
        // ⓘ It runs even when the line pass found problems, so a lossy reader
        // sees expanded values. `problems` lists line-pass problems first, so the
        // strict reading still returns the error it always did.
        if self.config.allow_expansion {
            self.expand_entries(&mut doc, &literal_keys);
        } else {
            // ⚠️ Expansion is what normally consumes the `\$` markers, so with it
            // off nothing would — and `"\$B"` would come back holding a U+FDD0
            // noncharacter. `allow_expansion` is a library setting no CLI flag
            // reaches, so only a consumer of the crate could hit this.
            for e in &mut doc.entries {
                e.value = strip_markers(&e.value);
            }
        }

        doc
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
