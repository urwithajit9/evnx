//! Convert command — transform `.env` files to multiple output formats.
//!
//! This module implements the `evnx convert` subcommand, which parses a `.env`
//! file and transforms its variables into 14 target formats including JSON,
//! YAML, cloud provider configs, CI/CD formats, and infrastructure-as-code.
//!
//! # Architecture
//!
//! ```text
//! .env file
//!     │
//!     ▼
//! Parser::parse_file() → IndexMap<String, String>
//!     │
//!     ▼
//! ConvertOptions::prepare() → filter, rename, the format's own names,
//!     │                       collisions, encode — one pipeline (A-12)
//!     ▼
//! Converter::convert() → format-specific layout
//!     │
//!     ▼
//! stdout or output file
//! ```
//!
//! # Key Features
//!
//! - **Filtering**: Include/exclude variables via `core::glob` patterns (`AWS_*`, `DB_*_URL`, `P?RT`)
//! - **Transformations**: Key casing (`camelCase`, `snake_case`), prefixes, base64 encoding
//! - **Interactive mode**: TUI format selector when `--to` is omitted
//! - **Verbose output**: Debug logging with `--verbose` flag
//!
//! # Supported Formats
//!
//! | Category | Formats |
//! |----------|---------|
//! | Generic | `json`, `yaml`, `yml`, `shell`, `bash`, `export` |
//! | Cloud | `aws-secrets`, `gcp-secrets`, `azure-keyvault` |
//! | CI/CD | `github-actions`, `gh-actions` |
//! | Containers | `docker-compose`, `kubernetes`, `k8s` |
//! | IaC | `terraform`, `tfvars`, `tf` |
//! | Secret Managers | `doppler`, `heroku`, `vercel`, `railway` |
//!
//! The list itself is [`crate::formats::FORMATS`].
//!
//! # Usage Examples
//!
//! ```no_run
//! # // These examples demonstrate CLI usage and cannot be run in doc tests
//! # // Basic JSON conversion
//! # // evnx convert --to json --env .env.production
//! #
//! # // With transformations
//! # // evnx convert \
//! # //   --to kubernetes --name myapp-env \
//! # //   --include "PROD_*" \
//! # //   --prefix "MYAPP_" \
//! # //   --transform uppercase \
//! # //   --output k8s-secret.yaml
//! #
//! # // Interactive mode (opens TUI selector)
//! # // evnx convert
//! ```
//!
//! # Error Handling
//!
//! All errors are wrapped with [`anyhow::Context`] to provide actionable messages:
//! - File parsing failures include the source path
//! - Format conversion errors specify the target format
//! - I/O errors include the output destination
//!
//! # See Also
//!
//! - [`crate::core::converter`] — Core conversion traits and options
//! - [`crate::formats`] — Format-specific converter implementations
//! - [`crate::utils::ui`] — Terminal UI utilities for consistent output

use anyhow::{Context, Result};
use colored::*;
use dialoguer::Select;
// use indexmap::IndexMap;

use crate::core::{
    converter::{Collisions, ConvertOptions, KeyTransform},
    Parser,
};
use crate::docs;
use crate::formats::{self, Targets};
use crate::utils::protect::GitignoreMode;
use crate::utils::ui;
// ─────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────

/// Configuration for the convert command.
///
/// This struct consolidates all CLI arguments into a single type,
/// improving testability and reducing parameter passing overhead.
///
/// # Example
///
/// ```
/// # use evnx::commands::convert::ConvertConfig;
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("kubernetes"))
///     .include(vec!["PROD_*".to_string()])
///     .prefix(Some("MYAPP_"))
///     .base64(true)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ConvertConfig {
    /// Path to input .env file
    pub env_path: String,
    /// Target format (None = interactive mode)
    pub target_format: Option<String>,
    /// Output file path (None = stdout)
    pub output_path: Option<String>,
    /// Conversion options (filtering, transformations)
    pub options: ConvertOptions,
    /// Enable verbose progress output
    pub verbose: bool,
    /// Replace an existing output file (`--force`, DEC-G5).
    pub force: bool,
    /// The names a format writes into its output (`--project`, …; I-084).
    pub targets: Targets,
    /// Convert even when no variable is left (`--allow-empty`, I-089).
    pub allow_empty: bool,
    /// `.gitignore` handling for `--output` (`--gitignore`, `--no-gitignore`).
    pub gitignore: GitignoreMode,
}

impl ConvertConfig {
    /// Create a new config with default values.
    ///
    /// ```
    /// # use evnx::commands::convert::ConvertConfig;
    /// let config = ConvertConfig::default();
    /// assert_eq!(config.env_path, ".env");
    /// assert!(config.target_format.is_none());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new config using builder pattern.
    ///
    /// ```
    /// # use evnx::commands::convert::ConvertConfig;
    /// let config = ConvertConfig::builder()
    ///     .env(".env.prod")
    ///     .verbose(true)
    ///     .build();
    /// assert_eq!(config.env_path, ".env.prod");
    /// assert!(config.verbose);
    /// ```
    #[must_use]
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Build the inner ConvertOptions from config fields.
    ///
    /// This method returns a clone of the stored options for use by converters.
    #[must_use]
    pub fn build_options(&self) -> ConvertOptions {
        self.options.clone()
    }
}

impl Default for ConvertConfig {
    fn default() -> Self {
        Self {
            env_path: ".env".to_string(),
            target_format: None,
            output_path: None,
            options: ConvertOptions {
                collisions: Collisions::Refuse,
                ..ConvertOptions::default()
            },
            verbose: false,
            force: false,
            targets: Targets::default(),
            allow_empty: false,
            gitignore: GitignoreMode::Default,
        }
    }
}

/// Builder for [`ConvertConfig`].
///
/// # Example
///
/// ```
/// # use evnx::commands::convert::ConvertConfig;
/// # use evnx::core::converter::KeyTransform;
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("json"))           // ✅ Wrap in Some()
///     .output_path(Some("output.json"))      // ✅ Wrap in Some()
///     .include(vec!["AWS_*".to_string()])
///     .exclude(vec!["*_DEBUG".to_string()])
///     .base64(true)
///     .prefix(Some("MYAPP_"))                // ✅ Wrap in Some()
///     .transform(Some(KeyTransform::Uppercase))  // ✅ Wrap in Some()
///     .verbose(true)
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    env_path: Option<String>,
    target_format: Option<String>,
    output_path: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    base64: bool,
    prefix: Option<String>,
    transform: Option<KeyTransform>,
    verbose: bool,
    force: bool,
    targets: Targets,
    allow_empty: bool,
    allow_collisions: bool,
    gitignore: Option<GitignoreMode>,
}

impl ConfigBuilder {
    /// Set the input .env file path.
    #[must_use]
    pub fn env(mut self, path: impl Into<String>) -> Self {
        self.env_path = Some(path.into());
        self
    }

    /// Set the target output format (optional).
    #[must_use]
    pub fn target_format<S: Into<String>>(mut self, format: Option<S>) -> Self {
        self.target_format = format.map(Into::into);
        self
    }

    /// Set the output file path (optional, None = stdout).
    #[must_use]
    pub fn output_path<S: Into<String>>(mut self, path: Option<S>) -> Self {
        self.output_path = path.map(Into::into);
        self
    }

    /// Keep only variables matching one of these globs (empty keeps all).
    #[must_use]
    pub fn include(mut self, patterns: Vec<String>) -> Self {
        self.include = patterns;
        self
    }

    /// Drop variables matching one of these globs.
    #[must_use]
    pub fn exclude(mut self, patterns: Vec<String>) -> Self {
        self.exclude = patterns;
        self
    }

    /// The names a format writes into its output (I-084).
    #[must_use]
    pub fn targets(mut self, targets: Targets) -> Self {
        self.targets = targets;
        self
    }

    /// Convert even when no variable is left (I-089).
    #[must_use]
    pub fn allow_empty(mut self, enabled: bool) -> Self {
        self.allow_empty = enabled;
        self
    }

    /// Keep the last value when two names become one (DEC-PL2).
    #[must_use]
    pub fn allow_collisions(mut self, enabled: bool) -> Self {
        self.allow_collisions = enabled;
        self
    }

    /// `.gitignore` handling for `--output`.
    #[must_use]
    pub fn gitignore(mut self, mode: GitignoreMode) -> Self {
        self.gitignore = Some(mode);
        self
    }

    /// Enable base64 encoding for all values.
    #[must_use]
    pub fn base64(mut self, enabled: bool) -> Self {
        self.base64 = enabled;
        self
    }

    /// Set prefix to prepend to all variable names (optional).
    #[must_use]
    pub fn prefix<S: Into<String>>(mut self, prefix: Option<S>) -> Self {
        self.prefix = prefix.map(Into::into);
        self
    }

    /// Set key casing transformation (optional).
    #[must_use]
    pub fn transform(mut self, transform: Option<KeyTransform>) -> Self {
        self.transform = transform;
        self
    }

    /// Enable verbose progress output.
    #[must_use]
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }

    /// Replace an existing output file (DEC-G5).
    #[must_use]
    pub fn force(mut self, enabled: bool) -> Self {
        self.force = enabled;
        self
    }

    /// Build the final [`ConvertConfig`].
    #[must_use]
    pub fn build(self) -> ConvertConfig {
        ConvertConfig {
            env_path: self.env_path.unwrap_or_else(|| ".env".to_string()),
            target_format: self.target_format,
            output_path: self.output_path,
            options: ConvertOptions {
                include: self.include,
                exclude: self.exclude,
                base64: self.base64,
                strip_prefix: None,
                prefix: self.prefix,
                transform: self.transform,
                collisions: if self.allow_collisions {
                    Collisions::LastWins
                } else {
                    Collisions::Refuse
                },
            },
            verbose: self.verbose,
            force: self.force,
            targets: self.targets,
            allow_empty: self.allow_empty,
            gitignore: self.gitignore.unwrap_or(GitignoreMode::Default),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point
// ─────────────────────────────────────────────────────────────

/// Execute the convert command with the provided configuration.
///
/// This function orchestrates the full conversion pipeline:
/// 1. Validate and parse the input `.env` file
/// 2. Select or interactively prompt for the target format
/// 3. Instantiate the appropriate [`Converter`] implementation
/// 4. Apply filtering, transformations, and serialization
/// 5. Write output to stdout or specified file
///
/// # Arguments
///
/// * `config` - The [`ConvertConfig`] containing all command options
///
/// # Returns
///
/// * `Ok(())` on successful conversion and output
/// * `Err(anyhow::Error)` with context for:
///   - File I/O errors (reading input or writing output)
///   - Parsing errors (invalid `.env` syntax)
///   - Serialization errors (format-specific conversion failures)
///   - User cancellation in interactive mode
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::convert::{ConvertConfig, run};
/// # use anyhow::Result;
/// # fn example() -> Result<()> {
/// let config = ConvertConfig::builder()
///     .env(".env.production")
///     .target_format(Some("json"))
///     .output_path(Some("output.json"))
///     .build();
///
/// run(config)?;
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// This function does not panic. All errors are returned via [`Result`].
pub fn run(config: ConvertConfig) -> Result<()> {
    if config.verbose {
        eprintln!(
            "{}",
            format!(
                "{} Running convert in verbose mode: {}",
                ui::glyph::INFO,
                config.env_path
            )
            .dimmed()
        );
    }

    // Parse .env file. A missing one is the error `validate` and `diff` give,
    // not three nested contexts (I-091).
    let parser = Parser::default();
    let env_file = parser.parse_file_or_hint(
        &config.env_path,
        &Parser::missing_env_hint(&config.env_path),
    )?;

    // ⚠️ Before anything is converted, so a refusal writes nothing (I-081,
    // DEC-G5).
    if let Some(out) = config.output_path.as_deref() {
        check_output(&config.env_path, out, config.force)?;
    }

    if config.verbose {
        eprintln!(
            "{}",
            format!(
                "{} Loaded {} variables from {}",
                ui::glyph::INFO,
                env_file.vars.len(),
                config.env_path
            )
            .dimmed()
        );
    }

    // Determine target format (interactive if not specified)
    let format_name = match &config.target_format {
        Some(f) => f.clone(),
        None => select_format_interactive(config.verbose)?,
    };

    // ⚠️ The exit lives here, not in the dispatch. `formats::get_converter`
    // returns an error so that `cloud export` can reach the same 14 formats
    // without a library function ending the process underneath it — but
    // `convert`'s own contract is unchanged: unknown format, this help, exit 2.
    let Some(format) = formats::find(&format_name) else {
        eprintln!("{} Unknown format: {}", "✗".red(), format_name);
        eprintln!();
        eprintln!("{}", "Supported formats:".bold());
        eprintln!();
        print_format_help();
        // 2, not 1: convert was asked for something it cannot produce, so
        // there is no output — the same "could not run" every other command
        // reports with 2. Nothing here converted anything badly.
        std::process::exit(2);
    };

    // The names the format writes into its output, required where it needs
    // one (I-084, DEC-CV1).
    let converter = formats::converter_for(format, &config.targets, "convert")?;

    if config.verbose {
        eprintln!(
            "{}",
            format!(
                "{} Converting to {} format…",
                ui::glyph::INFO,
                converter.name()
            )
            .dimmed()
        );
    }

    // Before converting: names that become one name — by `--transform` or by
    // the format's own naming — are refused unless `--allow-collisions`
    // (I-088, I-249, DEC-PL2); and nothing left to convert is refused unless
    // `--allow-empty` (I-089, DEC-CV3).
    let prepared = config.options.prepare(&env_file.vars, converter.as_ref())?;
    if prepared.is_empty() && !config.allow_empty {
        anyhow::bail!(nothing_to_convert(&config.env_path, env_file.vars.len()));
    }
    config
        .options
        .announce_kept(&env_file.vars, converter.as_ref());

    let result = converter
        .convert(&env_file.vars, &config.options)
        .with_context(|| format!("Conversion to '{}' failed", format.name))?;

    // DEC-CV4: a shape not yet checked against its target says so, on stderr
    // so a pipe still carries the artefact alone.
    if let Some(note) = format.experimental {
        ui::warning(note);
    }

    // Output result
    write_output(
        &result,
        config.output_path.as_deref(),
        config.verbose,
        &config.gitignore,
    )?;

    ui::print_docs_hint(&docs::CONVERT);

    Ok(())
}

/// Why nothing is left to convert, and the flag that converts it anyway.
///
/// ⚠️ It printed `{}` and exited 0 (I-089), so a pipeline whose `--include`
/// had a typo uploaded an empty secret and reported success. `cloud run`
/// refused the same case already; the filter wording is shared with it.
fn nothing_to_convert(env_path: &str, before: usize) -> String {
    let why = if before == 0 {
        format!("{env_path} holds no variables, so there is nothing to convert.")
    } else {
        crate::core::glob::nothing_left(before, "convert")
    };
    format!("{why}\n\x20 Pass --allow-empty to write the empty output anyway.")
}

// ─────────────────────────────────────────────────────────────
// Helper Functions
// ─────────────────────────────────────────────────────────────

/// Display interactive TUI for format selection.
///
/// # Arguments
///
/// * `verbose` - If true, emit debug messages to stderr
///
/// # Returns
///
/// * `Ok(String)` - The selected format name
/// * `Err(anyhow::Error)` - On user cancellation or dialoguer error
///
/// # Side Effects
///
/// - Prints formatted header to stdout
/// - Blocks waiting for user input via terminal UI
fn select_format_interactive(verbose: bool) -> Result<String> {
    // ⚠️ Before the header: with no terminal dialoguer failed with "IO error:
    // not a terminal", which reached the user as "Format selection cancelled by
    // user: … not a terminal: not a terminal" under a header for a menu that
    // never appeared (I-083).
    if !crate::utils::term::interactive() {
        return Err(crate::utils::prompt::choice_needed(
            "--to",
            "evnx convert --to <FORMAT>",
            &formats::names(),
        ));
    }

    if verbose {
        eprintln!(
            "{}",
            format!("{} Launching interactive format selector", ui::glyph::INFO).dimmed()
        );
    }

    // ⚠️ This was three hand-typed box-drawing literals with the bottom border
    // one column longer than the other two — 56 against 55, the same off-by-one
    // `migrate` had, and for the same reason: typed by hand rather than measured
    // by `print_box`.
    //
    // It also outlived the claim. A comment in `migrate/mod.rs` called that one
    // "the last command still drawing a box at all", and `diff.rs` said the same
    // of itself before that. Both were wrong, because a literal box is invisible
    // to a search for the helper that draws boxes. This one was found by a doc
    // sample that reproduced its output.
    ui::print_header(
        "evnx convert",
        Some("Transform .env into different formats"),
    );

    // From the one table, so a label cannot say "CLI commands" for a format
    // that prints JSON (I-087).
    let items: Vec<String> = formats::FORMATS
        .iter()
        .map(|f| format!("{} - {}", f.name, f.label))
        .collect();

    let selection = Select::new()
        .with_prompt("Select output format")
        .items(&items)
        .default(0)
        .interact()
        .context("choosing an output format")?;

    let format = formats::FORMATS[selection].name.to_string();

    if verbose {
        eprintln!("{}", format!("✓ Selected format: {}", format).dimmed());
    }

    Ok(format)
}

// `get_converter` moved to `crate::formats` so `cloud export` can reach the same
// format dispatch. See `formats::get_converter` — including why it now returns
// an error where this copy called `std::process::exit(2)`.

/// Print formatted help for supported formats.
///
/// Called when an unknown format is specified. Delegates to
/// [`crate::formats::format_help`] so a format added to the dispatch shows up
/// here without a second edit.
fn print_format_help() {
    formats::format_help();
}

/// Refuse an output that would destroy something: the input itself (by any
/// name — path, symlink or hard link), or an existing file without `--force`
/// (DEC-G5).
fn check_output(input: &str, output: &str, force: bool) -> Result<()> {
    // template's wording: one rule, read the same way in both (R-template).
    crate::core::safe_write::refuse_same_file(output, "env file", input)?;
    if !force && std::fs::symlink_metadata(output).is_ok() {
        anyhow::bail!("'{}' already exists. Pass --force to replace it.", output);
    }
    Ok(())
}

/// Write conversion result to stdout or file.
///
/// # Arguments
///
/// * `content` - The formatted output string
/// * `output_path` - Optional file path (None = stdout)
/// * `verbose` - If true, emit success messages to stderr
/// * `gitignore` - What to do about `.gitignore` for a written file
///
/// # Returns
///
/// * `Ok(())` on successful write
/// * `Err(anyhow::Error)` on I/O failure
fn write_output(
    content: &str,
    output_path: Option<&str>,
    verbose: bool,
    gitignore: &GitignoreMode,
) -> Result<()> {
    // ⚠️ Both paths go through the same normalisation, which is the whole fix.
    // They used to disagree: `fs::write` added nothing and `println!` added a
    // newline, so every format produced different bytes depending on whether
    // `--output` was passed (D22), and the formats whose converter already ends
    // in a newline gained a second one on stdout (D23).
    let content = crate::core::converter::newline_terminated(content.to_string());
    let content = content.as_str();

    match output_path {
        Some(path) => {
            // The output holds every value from the input: new files 0600,
            // atomic, symlinks written through (I-081, core::safe_write).
            let written = crate::core::safe_write::write_private(path, content.as_bytes())
                .with_context(|| format!("Failed to write output to '{}'", path))?;
            if verbose {
                eprintln!("{}", "✓ Converted successfully".green());
                eprintln!(
                    "{}",
                    format!("{} Output written to: {}", ui::glyph::OK, path).dimmed()
                );
            } else {
                // Both hidden by `-q`: the file is the result (I-007 family).
                ui::success("Converted successfully");
                ui::say(format!("Output written to: {path}"));
            }
            if written.replaced {
                ui::say(format!("Replaced {path}"));
            }
            crate::core::safe_write::report(&written);
            // Every value is in that file: keep it out of git the way
            // `template --output` does (R-template, `utils::protect`).
            crate::utils::protect::protect_output(path, gitignore)?;
        }
        None => {
            // `print!`, not `println!` — `content` is already newline-terminated
            // above, and this is the line that produced D23's blank line.
            print!("{content}");
            if verbose {
                eprintln!("{}", "✓ Output written to stdout".dimmed());
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::converter::KeyTransform;
    use indexmap::IndexMap;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_convert_config_default() {
        let config = ConvertConfig::default();
        assert_eq!(config.env_path, ".env");
        assert!(config.target_format.is_none());
        assert!(config.output_path.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_convert_config_builder() {
        let config = ConvertConfig::builder()
            .env(".env.prod")
            .target_format(Some("json")) // ✅ Wrap in Some()
            .output_path(Some("out.json")) // ✅ Wrap in Some()
            .include(vec!["AWS_*".to_string()])
            .exclude(vec!["*_DEBUG".to_string()])
            .base64(true)
            .prefix(Some("APP_")) // ✅ Wrap in Some()
            .transform(Some(KeyTransform::Uppercase)) // ✅ Wrap in Some()
            .verbose(true)
            .build();

        assert_eq!(config.env_path, ".env.prod");
        assert_eq!(config.target_format, Some("json".to_string()));
        assert_eq!(config.output_path, Some("out.json".to_string()));
        assert_eq!(config.options.include, ["AWS_*"]);
        assert_eq!(config.options.exclude, ["*_DEBUG"]);
        assert!(config.options.base64);
        assert_eq!(config.options.prefix, Some("APP_".to_string()));
        assert!(matches!(
            config.options.transform,
            Some(KeyTransform::Uppercase)
        ));
        assert!(config.verbose);
    }

    #[test]
    fn test_build_options_clone() {
        let opts = ConvertOptions {
            prefix: Some("TEST_".to_string()),
            base64: true,
            ..Default::default()
        };

        let config = ConvertConfig {
            options: opts,
            ..Default::default()
        };

        let built = config.build_options();
        assert_eq!(built.prefix, Some("TEST_".to_string()));
        assert!(built.base64);
        // Verify it's a clone, not a move
        assert_eq!(config.options.prefix, Some("TEST_".to_string()));
    }

    // The dispatch tests moved with the dispatch, to `formats::registry_tests`.
    // They grew in the move: every alias is now asserted, and so is the unknown
    // arm, which could not be tested here because it exited the process.

    #[test]
    fn test_write_output_to_stdout() -> Result<()> {
        let content = "KEY=value\n";
        // Just verify no error when writing to stdout (can't easily capture)
        assert!(write_output(content, None, false, &GitignoreMode::Skip).is_ok());
        Ok(())
    }

    #[test]
    fn test_write_output_to_file() -> Result<()> {
        let tmp = TempDir::new()?;
        let output_path = tmp.path().join("output.txt");
        let content = "KEY=value\n";

        write_output(
            content,
            Some(output_path.to_str().unwrap()),
            false,
            &GitignoreMode::Skip,
        )?;

        let written = fs::read_to_string(&output_path)?;
        assert_eq!(written, content);
        Ok(())
    }

    /// D22: a file must end with a newline even when the converter did not.
    ///
    /// The JSON-shaped converters return a String with no trailing newline, and
    /// `fs::write` added none — so `convert --to json -o f.json` wrote a file
    /// that no diff tool, and some parsers, would accept as whole.
    #[test]
    fn a_file_always_ends_with_a_newline() -> Result<()> {
        let tmp = TempDir::new()?;
        let path = tmp.path().join("out.json");

        write_output(
            "{\"A\":\"1\"}",
            Some(path.to_str().unwrap()),
            false,
            &GitignoreMode::Skip,
        )?;

        let written = fs::read_to_string(&path)?;
        assert_eq!(written, "{\"A\":\"1\"}\n");
        Ok(())
    }

    /// D23: and it must not gain a second one when the converter supplied it.
    #[test]
    fn a_file_does_not_gain_a_blank_line() -> Result<()> {
        let tmp = TempDir::new()?;
        let path = tmp.path().join("out.yaml");

        write_output(
            "A: '1'\n",
            Some(path.to_str().unwrap()),
            false,
            &GitignoreMode::Skip,
        )?;

        let written = fs::read_to_string(&path)?;
        assert_eq!(written, "A: '1'\n", "a trailing blank line came back");
        Ok(())
    }

    /// The property the two defects were really about: **one** rule, applied to
    /// every format, whichever converter produced it.
    ///
    /// ⚠️ This asserts on the normalisation rather than on `write_output`'s
    /// stdout, which cannot be captured here. The stdout path calls `print!` on
    /// the same normalised string, so the two agree by construction — a reader
    /// changing one must change the other.
    #[test]
    fn every_format_normalises_to_exactly_one_trailing_newline() {
        use crate::core::converter::newline_terminated;
        let vars: IndexMap<String, String> =
            [("A".to_string(), "1".to_string())].into_iter().collect();

        for name in [
            "json",
            "yaml",
            "shell",
            "aws-secrets",
            "gcp-secrets",
            "azure-keyvault",
            "github-actions",
            "docker-compose",
            "kubernetes",
            "terraform",
            "doppler",
            "heroku",
            "vercel",
            "railway",
        ] {
            let raw = formats::get_converter(name)
                .unwrap()
                .convert(&vars, &ConvertOptions::default())
                .unwrap();
            let out = newline_terminated(raw);
            assert!(out.ends_with('\n'), "{name}: no trailing newline");
            assert!(!out.ends_with("\n\n"), "{name}: trailing blank line");
        }
    }

    #[test]
    fn test_key_transform_integration() {
        // Integration test: verify options transform keys correctly
        let options = ConvertOptions {
            prefix: Some("APP_".to_string()),
            transform: Some(KeyTransform::Uppercase),
            ..Default::default()
        };

        let original = "database_url";
        let transformed = options.transform_key(original);
        assert_eq!(transformed, "APP_DATABASE_URL");
    }

    #[test]
    fn test_glob_filter_integration() {
        let mut vars = IndexMap::new(); // ✅ IndexMap now imported
        vars.insert("AWS_KEY".to_string(), "aws_val".to_string());
        vars.insert("DB_KEY".to_string(), "db_val".to_string());
        vars.insert("APP_URL".to_string(), "app_val".to_string());

        let options = ConvertOptions {
            include: vec!["AWS_*".to_string()],
            ..Default::default()
        };

        let filtered = options.filter_vars(&vars);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("AWS_KEY"));
        assert!(!filtered.contains_key("DB_KEY"));
    }

    #[test]
    fn test_builder_with_none_values() {
        // Test that builder handles None values correctly
        let config = ConvertConfig::builder()
            .env(".env")
            .target_format(Option::<String>::None)
            .output_path(Option::<String>::None)
            .include(Vec::new())
            .exclude(Vec::new())
            .base64(false)
            .prefix(Option::<String>::None)
            .transform(None)
            .verbose(false)
            .build();

        assert_eq!(config.env_path, ".env");
        assert!(config.target_format.is_none());
        assert!(config.output_path.is_none());
        assert!(config.options.include.is_empty());
        assert!(config.options.exclude.is_empty());
        assert!(!config.options.base64);
        assert!(config.options.prefix.is_none());
        assert!(config.options.transform.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_builder_chaining_order_independence() {
        // Builder methods should work in any order
        let config1 = ConvertConfig::builder()
            .env(".env")
            .target_format(Some("json"))
            .verbose(true)
            .build();

        let config2 = ConvertConfig::builder()
            .verbose(true)
            .target_format(Some("json"))
            .env(".env")
            .build();

        assert_eq!(config1.env_path, config2.env_path);
        assert_eq!(config1.target_format, config2.target_format);
        assert_eq!(config1.verbose, config2.verbose);
    }
}
