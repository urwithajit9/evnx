//! Template engine for generating configuration files from `.env` files.
//!
//! This module provides a flexible template substitution system that supports:
//!
//! - Multiple variable syntax styles: `${VAR}`, `{{VAR}}`, `$VAR`
//! - Filter transformations: `|upper`, `|lower`, `|title`, `|capitalize`, `|bool`, `|int`, `|json`,
//!   `|yaml`, `|toml`, `|sh`, `|default:value`
//! - Proper error handling with contextual messages
//! - Detection of undefined variables with warnings
//! - Integration with the project's UI utilities for consistent output
//!
//! # Variable Substitution Syntax
//!
//! The template engine supports three styles of variable references:
//!
//! ```text
//! ${DATABASE_URL}     # Shell-style (bash-like)
//! {{DATABASE_URL}}    # Template-style (Jinja-like)
//! $DATABASE_URL       # Simple prefix style
//! ```
//!
//! # Filter Syntax
//!
//! Filters can be applied using the pipe operator within double braces:
//!
//! ```text
//! {{ENV|upper}}           # Convert to uppercase: "production" → "PRODUCTION"
//! {{NAME|title}}          # Title case: "hello wORLD" → "Hello World"
//! {{NAME|capitalize}}     # First character only: "hello wORLD" → "Hello wORLD"
//! {{DEBUG|bool}}          # true/yes/on/1/y → true, false/no/off/0/n → false; else an error
//! {{PORT|int}}            # Convert to integer: "8000" → 8000
//! {{NAME|json}}           # JSON-escape: hello "world" → "hello \"world\""
//! {{OPTIONAL|default:foo}} # Use default if undefined or empty
//! {{NAME|yaml}}           # A double-quoted YAML scalar
//! {{NAME|toml}}           # A TOML basic string
//! {{NAME|sh}}             # One shell word, single-quoted
//! ```
//!
//! Filters chain left to right: `{{PORT|default:8080|int}}`. An unknown filter
//! is an error naming its line, and nothing is written.
//!
//! # One pass (B3, DEC-T13)
//!
//! The template is read once, left to right. Inserted text is never read
//! again, so a value that looks like `{{B|upper}}` is written as it is, and
//! the output does not depend on the order of the keys (I-133).
//!
//! - `${NAME}` and `{{NAME}}` are always references. A bare `$NAME` is
//!   substituted only when `NAME` is defined, and never reported (DEC-T3).
//! - A backslash makes a reference literal: `\${NAME}`, `\{{NAME}}` and
//!   `\$NAME` render without the backslash, and `\\` before a reference
//!   renders one backslash (DEC-T5). Every other backslash is text.
//!
//! # Usage Examples
//!
//! ## Basic template processing
//!
//! ```no_run
//! # use indexmap::IndexMap;
//! # use evnx::commands::template::process_template;
//! let template = "url={{DATABASE_URL}}";
//! let mut vars = IndexMap::new();
//! vars.insert("DATABASE_URL".to_string(), "postgresql://localhost".to_string());
//!
//! let result = process_template(template, &vars).unwrap();
//! assert_eq!(result, "url=postgresql://localhost");
//! ```
//!
//! ## Using filters
//!
//! ```no_run
//! # use indexmap::IndexMap;
//! # use evnx::commands::template::process_template;
//! let template = "ENV={{ENV|upper}}, debug={{DEBUG|bool}}";
//! let mut vars = IndexMap::new();
//! vars.insert("ENV".to_string(), "production".to_string());
//! vars.insert("DEBUG".to_string(), "yes".to_string());
//!
//! let result = process_template(template, &vars).unwrap();
//! assert_eq!(result, "ENV=PRODUCTION, debug=true");
//! ```
//!
//! ## Command-line usage
//!
//! ```sh
//! # Generate config from template
//! evnx template --input config.toml.template --output config.toml --env .env.production
//!
//! # Check it renders, without writing anything (CI)
//! evnx template --input template.yaml --output output.yaml --dry-run
//!
//! # Into a directory that does not exist yet, and print to stdout
//! evnx template --input template.yaml --output deploy/prod/output.yaml --mkdir
//! evnx template --input template.yaml --output - | kubectl apply -f -
//! ```
//!
//! # Error Handling
//!
//! - Undefined variables are an error (exit 2), before anything is written;
//!   `--allow-unresolved` writes them through literally and exits 0 (DEC-T2)
//! - Invalid filter values (a non-integer for `|int`, a non-boolean for `|bool`)
//!   and unknown filters return an error; the value is masked when its name
//!   looks secret
//! - File I/O errors include contextual path information
//!
//! # Security Considerations
//!
//! - The output holds real values: a new file is 0600, and the `.gitignore`
//!   step (`utils::protect`) offers to keep it out of git
//! - Variable values are inserted as-is; sanitize if generating shell commands
//! - Template paths should be validated by the caller to prevent path traversal
//! - For JSON output, use `|json` filter to properly escape special characters

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::fs;

use colored::Colorize;

use crate::core::Parser;
use crate::docs;
/// How `template` updates `.gitignore`; the flow is shared (`utils::protect`).
pub use crate::utils::protect::GitignoreMode;
use crate::utils::ui;

// ─────────────────────────────────────────────────────────────
// Main Functions
// ─────────────────────────────────────────────────────────────

/// Execute the template command: generate config files from templates.
///
/// This function orchestrates the template processing pipeline:
/// 1. Parse the `.env` file into a variable map
/// 2. Read the template file content
/// 3. Process variable substitution and filters
/// 4. Write the generated output file
///
/// # Arguments
///
/// * `input` - Path to the template file (e.g., `config.toml.template`)
/// * `output` - Path where the generated config will be written
/// * `env` - Path to the `.env` file containing variable definitions
/// * `verbose` - Enable detailed progress output to stderr
/// * `strict` - Refuse to write when a placeholder has no value
///
/// # Returns
///
/// * `Ok(())` on success
/// * `Err(anyhow::Error)` with contextual message on failure
///
/// ⚠️ Under `strict`, the refusal happens **before** the output file is written.
/// An unresolved `{{DATABASE_URL}}` does not stay a template once written — it
/// becomes a config file some other program reads as a literal host name.
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::template::{run, GitignoreMode};
/// run(
///     "config.yaml.template".to_string(),
///     "config.yaml".to_string(),
///     ".env.production".to_string(),
///     true, // verbose
///     GitignoreMode::Default,
///     true, // strict
/// )
/// .expect("Template generation failed");
/// ```
pub fn run(
    input: String,
    output: String,
    env: String,
    verbose: bool,
    gitignore_mode: GitignoreMode,
    strict: bool,
) -> Result<()> {
    run_with(input, output, env, verbose, gitignore_mode, strict, None)
}

/// [`run`] with a file mode for the output (`--mode`, DEC-T6). `None`: a new
/// output is 0600 and an existing one keeps its mode (DEC-G6).
///
/// `output` of `-` prints the rendered text on stdout and writes no file
/// (DEC-T8); status lines then go to stderr.
pub fn run_with(
    input: String,
    output: String,
    env: String,
    verbose: bool,
    gitignore_mode: GitignoreMode,
    strict: bool,
    file_mode: Option<u32>,
) -> Result<()> {
    run_options(
        input,
        output,
        env,
        Options {
            verbose,
            gitignore: gitignore_mode,
            strict,
            file_mode,
            mkdir: false,
            dry_run: false,
        },
    )
}

/// Everything `template` takes besides its three paths.
pub struct Options {
    /// `-v`.
    pub verbose: bool,
    /// `--gitignore` / `--no-gitignore` / neither.
    pub gitignore: GitignoreMode,
    /// Refuse to write when a placeholder has no value: the CLI's default since
    /// 0.11.0 (`--allow-unresolved` turns it off). `false` here for library callers.
    pub strict: bool,
    /// `--mode`: `None` makes a new output 0600 and keeps an existing one's mode.
    pub file_mode: Option<u32>,
    /// `--mkdir`: create the output's missing directories, owner-only (I-144).
    pub mkdir: bool,
    /// `--dry-run`: render and report, write nothing (DEC-T9).
    pub dry_run: bool,
}

/// [`run`] with every option (`--mkdir`, `--dry-run` included).
pub fn run_options(input: String, output: String, env: String, opts: Options) -> Result<()> {
    let to_stdout = crate::core::safe_write::is_stdout(&output);
    // With `--output -` stdout is the rendered file, so a status line there
    // would end up inside it (DEC-T8).
    let status = |msg: String| {
        if to_stdout {
            if !ui::is_quiet() {
                eprintln!("{} {}", ui::glyph::OK.green(), msg);
            }
        } else {
            ui::success(msg);
        }
    };

    // ⛔ Before anything is read: the template or the env file named as the
    // output, by any path, symlink or hard link, would be replaced by the
    // rendered result — real values in, the template gone (I-131, DEC-T4).
    if !to_stdout {
        for (what, path) in [("template", &input), ("env file", &env)] {
            crate::core::safe_write::refuse_same_file(&output, what, path)?;
        }
    }

    if opts.verbose {
        ui::verbose_stderr("Running template in verbose mode");
    }

    ui::print_header("Generate config from template", None);

    // Parse .env file
    let parser = Parser::default();
    let env_file = parser
        .parse_file(&env)
        .with_context(|| format!("Failed to parse environment file: {}", env))?;

    status(format!(
        "Loaded {} variables from {}",
        env_file.vars.len(),
        env
    ));

    // Read template
    let template_content = fs::read_to_string(&input)
        .with_context(|| format!("Failed to read template file: {}", input))?;

    status(format!("Read template from {}", input));

    // A filter evnx does not know is a typo shipping into the config: refuse
    // before anything else is reported (DEC-T7).
    check_filters(&template_content, &env_file.vars, &input, &output)?;

    // Not evnx syntax: written as it is, but said (I-136).
    for (line, raw) in shell_expansions(&template_content, &env_file.vars) {
        ui::warning(format!(
            "{input} line {line}: {raw} is written through as it is. \
             evnx's fallback is {{{{NAME|default:value}}}}"
        ));
    }

    // Detect undefined variables and warn
    let undefined = detect_undefined_vars(&template_content, &env_file.vars);
    for var in &undefined {
        ui::warning(format!(
            "Variable '{}' referenced in template but not defined in {}",
            var, env
        ));
    }

    // ⚠️ Refuse *before* writing. An unresolved `{{DATABASE_URL}}` does not stay
    // a template once it is written — it becomes a config file that some other
    // program reads as a literal host name. Writing the file and then reporting
    // the problem leaves the broken artefact on disk for whatever runs next.
    if opts.strict && !undefined.is_empty() {
        anyhow::bail!(
            "{} variable{} referenced in {input} {} no value in {env}: {}\n\n\
             Define {} there, or give {} a fallback with {{{{NAME|default:value}}}}.\n\
             To write unresolved placeholders through as they are: --allow-unresolved.\n\
             {}",
            undefined.len(),
            if undefined.len() == 1 { "" } else { "s" },
            if undefined.len() == 1 { "has" } else { "have" },
            undefined.join(", "),
            if undefined.len() == 1 { "it" } else { "them" },
            if undefined.len() == 1 { "it" } else { "each" },
            nothing_written(&output),
        );
    }

    // Process template
    let result = process_template(&template_content, &env_file.vars)
        .context("Failed to process template substitutions")?;

    if to_stdout {
        if opts.dry_run {
            // ⚠️ stderr: a dry run piped into `kubectl apply -f -` must not
            // feed the report to it.
            eprintln!("Dry run: nothing was printed.");
            eprintln!(
                "  would print {} to stdout",
                unresolved_note(result.lines().count(), "line", &undefined)
            );
            return Ok(());
        }
        print!("{result}");
        // No file, so nothing to protect in .gitignore (DEC-T8).
        ui::print_docs_hint(&docs::TEMPLATE);
        return Ok(());
    }

    // What the write would do, worked out the way the write works it out
    // (core::safe_write::preview): the dry run reports it, the real run checks
    // the directory before writing.
    let plan = crate::core::safe_write::preview(&output)
        .with_context(|| format!("Failed to write generated config to: {}", output))?;
    if let Some(dir) = &plan.missing_dir {
        if !opts.mkdir {
            return Err(anyhow::anyhow!(
                "the directory {} does not exist. Add --mkdir to create it.",
                dir.display()
            ))
            .with_context(|| format!("Failed to write generated config to: {}", output));
        }
    }

    if opts.dry_run {
        print_dry_run(&plan, &output, &undefined);
        return Ok(());
    }

    if let Some(dir) = &plan.missing_dir {
        let created = crate::core::safe_write::create_dirs_private(dir)?;
        if !created.is_empty() {
            ui::say(format!(
                "Created {} {}",
                if created.len() == 1 {
                    "directory"
                } else {
                    "directories"
                },
                created
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // Write output: new 0600, existing keeps its mode, atomic, symlinks written
    // through (I-130, I-140, core::safe_write).
    let written = match opts.file_mode {
        Some(m) => crate::core::safe_write::write_with_mode(&output, result.as_bytes(), m),
        None => crate::core::safe_write::write_private(&output, result.as_bytes()),
    }
    .with_context(|| format!("Failed to write generated config to: {}", output))?;

    ui::success(format!("Generated config at {}", output));
    if written.replaced {
        ui::say(format!("Replaced {output}"));
    }
    crate::core::safe_write::report(&written);

    if !undefined.is_empty() {
        ui::info(format!(
            "{} placeholder{} left unresolved and written through literally \
             (--allow-unresolved).",
            undefined.len(),
            if undefined.len() == 1 {
                " was"
            } else {
                "s were"
            }
        ));
    }
    crate::utils::protect::protect_output(&output, &opts.gitignore)?;
    ui::print_docs_hint(&docs::TEMPLATE);
    Ok(())
}

/// `--dry-run` with a file output: what the write would do (DEC-T9). Printed
/// whatever `-q` says, as `add --dry-run` does: it is the output that was
/// asked for. Never the rendered text — it holds the values.
fn print_dry_run(plan: &crate::core::safe_write::Preview, output: &str, undefined: &[String]) {
    println!("Dry run: nothing was written.");
    if let Some(dir) = &plan.missing_dir {
        println!("  would create the directory {} (--mkdir)", dir.display());
    }
    let verb = if plan.replaces { "replace" } else { "create" };
    if plan.through_link {
        println!(
            "  would {verb} {} through the link {output}",
            plan.target.display()
        );
    } else {
        println!("  would {verb} {output}");
    }
    if !undefined.is_empty() {
        println!(
            "  {}",
            unresolved_note(undefined.len(), "placeholder", &[]) + " would be left unresolved"
        );
    }
}

/// `3 lines` / `1 placeholder`, plus how many references stay unresolved.
fn unresolved_note(n: usize, what: &str, undefined: &[String]) -> String {
    let mut s = format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    if !undefined.is_empty() {
        s.push_str(&format!(
            ", {} placeholder{} unresolved",
            undefined.len(),
            if undefined.len() == 1 { "" } else { "s" }
        ));
    }
    s
}

/// The filters evnx knows, for the error that lists them.
const FILTERS: &[&str] = &[
    "upper",
    "lower",
    "title",
    "capitalize",
    "bool",
    "int",
    "json",
    "yaml",
    "toml",
    "sh",
];

/// Refuse a filter evnx does not know, naming each one with its line
/// (DEC-T7). They used to be written into the output unchanged, so
/// `{{PORT|innt}}` shipped as literal text with exit 0.
///
/// Every `{{ NAME | … }}` counts when NAME is an identifier or a key in the
/// env file, whether it has a value or not (owner, 2026-10-17). Text that is
/// not an evnx reference — Helm's `{{ .Values.x | quote }}` — is left alone.
fn check_filters(
    template: &str,
    vars: &IndexMap<String, String>,
    input: &str,
    output: &str,
) -> Result<()> {
    let mut problems = Vec::new();
    for token in tokens(template, vars) {
        let Token::Ref(r) = token else { continue };
        for f in &r.filters {
            let Filter::Named(name) = f else { continue };
            if FILTERS.contains(name) {
                continue;
            }
            let mut msg = format!("line {}: {}: unknown filter `{name}`", r.line, r.raw);
            let near = crate::utils::string::closest(
                &name.to_lowercase(),
                FILTERS.iter().map(|f| (*f, *f)),
            );
            if let Some(best) = near.first() {
                msg.push_str(&format!(" — did you mean `{best}`?"));
            }
            problems.push(msg);
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "{input} has {} evnx cannot apply:\n  {}\n\n\
         Filters: {}, default:VALUE. They chain left to right.\n\
         {}",
        if problems.len() == 1 {
            "a filter".to_string()
        } else {
            format!("{} filters", problems.len())
        },
        problems.join("\n  "),
        FILTERS.join(", "),
        nothing_written(output),
    )
}

/// The last line of a refusal. `-` is stdout, so "written to -" named no file
/// (RM-13); the wording matches `--dry-run`'s "nothing was printed".
fn nothing_written(output: &str) -> String {
    if crate::core::safe_write::is_stdout(output) {
        "Nothing was printed.".to_string()
    } else {
        format!("Nothing was written to {output}.")
    }
}

/// A name the reference patterns treat as a variable: a letter or `_`, then
/// letters, digits and `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One piece of a template, as the scanner reads it.
enum Token<'a> {
    /// Written as it is. An escape's backslash is already left out.
    Text(&'a str),
    /// `${NAME}`, `{{NAME|filters…}}`, or a bare `$NAME`.
    Ref(Ref<'a>),
    /// `${NAME:-x}` and the other shell expansions: not evnx syntax, so
    /// written as it is, with a warning (I-136).
    Shell { raw: &'a str, line: usize },
}

struct Ref<'a> {
    /// The placeholder as written: what an unresolved reference renders as.
    raw: &'a str,
    name: &'a str,
    filters: Vec<Filter<'a>>,
    /// A bare `$NAME`: substituted only when defined, never reported (DEC-T3).
    bare: bool,
    line: usize,
}

enum Filter<'a> {
    /// `default:TEXT`, trimmed. Its text may hold a pipe, unless a filter
    /// name follows the pipe: `{{X|default:a|b}}` is `a|b`, and
    /// `{{X|default:8080|int}}` is a default, then `int`.
    Default(&'a str),
    Named(&'a str),
}

/// Whether `s` starts a reference: `${`, `{{`, or `$` and a name.
fn opens_ref(s: &str) -> bool {
    s.starts_with("${")
        || s.starts_with("{{")
        || s.strip_prefix('$')
            .and_then(|r| r.chars().next())
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

/// `${NAME:-x}`, `${NAME-x}`, `${NAME:?x}` and the like: a name, then a
/// shell operator.
fn is_shell_expansion(inner: &str) -> bool {
    let end = inner
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(inner.len());
    is_identifier(&inner[..end])
        && [":-", "-", ":=", "=", ":?", "?", ":+", "+"]
            .iter()
            .any(|op| inner[end..].starts_with(op))
}

fn parse_filters(tail: &str) -> Vec<Filter<'_>> {
    let mut out = Vec::new();
    // Where the open default's text starts in `tail`.
    let mut default_from = None;
    let mut pos = 0;
    for seg in tail.split('|') {
        let end = pos + seg.len();
        let s = seg.trim();
        let new_filter = FILTERS.contains(&s) || s.starts_with("default:");
        match default_from {
            Some(from) if !new_filter => {
                if let Some(last) = out.last_mut() {
                    *last = Filter::Default(tail[from..end].trim());
                }
            }
            _ => {
                default_from = None;
                if let Some(at) = seg.find("default:").filter(|_| s.starts_with("default:")) {
                    let from = pos + at + "default:".len();
                    default_from = Some(from);
                    out.push(Filter::Default(tail[from..end].trim()));
                } else {
                    out.push(Filter::Named(s));
                }
            }
        }
        pos = end + 1;
    }
    out
}

/// Read `template` once, left to right (B3). `vars` decides whether a
/// non-identifier name such as `VAR.WITH.DOTS` is a key or someone else's
/// syntax.
fn tokens<'a>(template: &'a str, vars: &IndexMap<String, String>) -> Vec<Token<'a>> {
    let is_name = |n: &str| is_identifier(n) || vars.contains_key(n);
    let mut out = Vec::new();
    let (mut i, mut text, mut line) = (0, 0, 1);
    while i < template.len() {
        let rest = &template[i..];
        let b = rest.as_bytes()[0];
        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b == b'\\' {
            let after = &rest[1..];
            if after.starts_with('\\') && opens_ref(&after[1..]) {
                // `\\` before a reference: one backslash, then the reference.
                out.push(Token::Text(&template[text..=i]));
                i += 2;
                text = i;
            } else if opens_ref(after) {
                // The reference's opener is text; the rest reads as text too.
                out.push(Token::Text(&template[text..i]));
                text = i + 1;
                i += if after.starts_with('$') && !after.starts_with("${") {
                    2
                } else {
                    3
                };
            } else {
                i += 1;
            }
            continue;
        }
        // A reference never spans a line.
        let closing = |open: usize, close: &str| {
            let body = &rest[open..];
            body.find(close).filter(|&j| !body[..j].contains('\n'))
        };
        let mut found = None;
        if rest.starts_with("${") {
            if let Some(j) = closing(2, "}") {
                let inner = &rest[2..2 + j];
                let raw = &rest[..2 + j + 1];
                let name = inner.trim();
                if is_name(name) {
                    found = Some(Token::Ref(Ref {
                        raw,
                        name,
                        filters: Vec::new(),
                        bare: false,
                        line,
                    }));
                } else if is_shell_expansion(name) {
                    found = Some(Token::Shell { raw, line });
                }
            }
        } else if rest.starts_with("{{") {
            if let Some(j) = closing(2, "}}") {
                let inner = &rest[2..2 + j];
                let raw = &rest[..2 + j + 2];
                let (name, tail) = match inner.split_once('|') {
                    Some((n, t)) => (n.trim(), Some(t)),
                    None => (inner.trim(), None),
                };
                if is_name(name) {
                    let filters = tail.map(parse_filters).unwrap_or_default();
                    found = Some(Token::Ref(Ref {
                        raw,
                        name,
                        filters,
                        bare: false,
                        line,
                    }));
                }
            }
        } else if opens_ref(rest) {
            let end = 1 + rest[1..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len() - 1);
            found = Some(Token::Ref(Ref {
                raw: &rest[..end],
                name: &rest[1..end],
                filters: Vec::new(),
                bare: true,
                line,
            }));
        }
        match found {
            Some(token) => {
                let len = match &token {
                    Token::Ref(r) => r.raw.len(),
                    Token::Shell { raw, .. } => raw.len(),
                    Token::Text(_) => unreachable!(),
                };
                out.push(Token::Text(&template[text..i]));
                out.push(token);
                i += len;
                text = i;
            }
            // Not a reference: one byte on. Only ASCII is matched, so a byte
            // inside a multi-byte character is never a slice boundary here.
            None => i += 1,
        }
    }
    out.push(Token::Text(&template[text..]));
    out
}

/// What `r` renders as, or `None` when it stays unresolved: no value, and no
/// `default:` in its chain.
fn resolve(r: &Ref, vars: &IndexMap<String, String>) -> Result<Option<String>> {
    let mut value = vars.get(r.name).cloned();
    for f in &r.filters {
        value = match (f, value) {
            (Filter::Default(text), v) => match v {
                Some(v) if !v.trim().is_empty() => Some(v),
                _ => Some((*text).to_string()),
            },
            (Filter::Named(_), None) => None,
            (Filter::Named(name), Some(v)) => Some(apply_filter(name, r.name, &v)?),
        };
    }
    Ok(value)
}

fn apply_filter(filter: &str, key: &str, value: &str) -> Result<String> {
    Ok(match filter {
        "upper" => value.to_uppercase(),
        "lower" => value.to_lowercase(),
        "title" => title_case(value),
        // The old `|title`: the first character only, the rest as written.
        "capitalize" => {
            let mut c = value.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }
        // One word list for evnx (core::value); anything else is an error, as
        // `|int` is (I-138, DEC-T7).
        "bool" => crate::core::value::parse_bool(value)
            .with_context(|| {
                format!(
                    "Variable '{}' value '{}' is not a boolean for |bool: use {} or {}",
                    key,
                    shown(key, value),
                    crate::core::value::TRUE_WORDS.join("/"),
                    crate::core::value::FALSE_WORDS.join("/"),
                )
            })?
            .to_string(),
        "int" => value
            .parse::<i64>()
            .with_context(|| {
                format!(
                    "Variable '{}' value '{}' is not a valid integer for |int filter",
                    key,
                    shown(key, value)
                )
            })?
            .to_string(),
        "json" => serde_json::to_string(value)
            .with_context(|| format!("Failed to JSON-encode variable '{}'", key))?,
        "yaml" => quoted(value, yaml_escape),
        "toml" => quoted(value, toml_escape),
        "sh" => crate::core::shell::quote(value),
        other => anyhow::bail!("unknown filter `{other}` on {key}"),
    })
}

/// `value` in double quotes, each character through `escape`.
fn quoted(value: &str, escape: fn(char, &mut String)) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        escape(c, &mut out);
    }
    out.push('"');
    out
}

/// A YAML double-quoted scalar (DEC-T10): `\` and `"` escaped, and every
/// control or line-separator character as an escape, so the value stays on
/// its line and nothing in it is YAML syntax.
fn yaml_escape(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\\\"),
        '"' => out.push_str("\\\""),
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        c if c.is_control() || c == '\u{2028}' || c == '\u{2029}' => {
            out.push_str(&format!("\\u{:04x}", c as u32))
        }
        c => out.push(c),
    }
}

/// A TOML basic string (DEC-T10): the escapes the TOML spec requires.
fn toml_escape(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\\\"),
        '"' => out.push_str("\\\""),
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
        c => out.push(c),
    }
}

/// Process a template string with variable substitution and filter transformations.
///
/// One pass, left to right: a substituted value is never read again, so the
/// result does not depend on key order or on what the values look like
/// (I-133). An unresolved reference is written as it is.
///
/// # Supported Patterns
///
/// | Pattern | Description | Example |
/// |---------|-------------|---------|
/// | `${VAR}` | Shell-style substitution | `${PORT}` → `8000` |
/// | `{{VAR}}` | Template-style substitution | `{{PORT}}` → `8000` |
/// | `$VAR` | Only when `VAR` is defined | `$PORT` → `8000` |
/// | `{{VAR|upper}}` | Uppercase filter | `{{env|upper}}` → `PRODUCTION` |
/// | `{{VAR|bool}}` | Boolean conversion | `{{debug|bool}}` → `true` |
/// | `{{VAR|int}}` | Integer parsing | `{{port|int}}` → `8000` |
/// | `{{VAR|json}}` | JSON string escaping | `{{name|json}}` → `"hello \"world\""` |
/// | `{{VAR|default:foo}}` | Default value if undefined or empty | `{{opt|default:bar}}` → `bar` |
/// | `{{VAR|default:8080|int}}` | Filters chain left to right | `8080` |
///
/// # Example
///
/// ```
/// # use indexmap::IndexMap;
/// # use evnx::commands::template::process_template;
/// let template = "host={{HOST|upper}}, port=${PORT}";
/// let mut vars = IndexMap::new();
/// vars.insert("HOST".to_string(), "localhost".to_string());
/// vars.insert("PORT".to_string(), "3000".to_string());
///
/// let result = process_template(template, &vars).unwrap();
/// assert_eq!(result, "host=LOCALHOST, port=3000");
/// ```
pub fn process_template(template: &str, vars: &IndexMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    for token in tokens(template, vars) {
        match token {
            Token::Text(s) | Token::Shell { raw: s, .. } => out.push_str(s),
            Token::Ref(r) => match resolve(&r, vars)? {
                Some(v) => out.push_str(&v),
                None => out.push_str(r.raw),
            },
        }
    }
    Ok(out)
}

/// The shell expansions in `template` (`${X:-d}` and the like), with their
/// lines: evnx writes them through as they are (I-136).
fn shell_expansions<'a>(
    template: &'a str,
    vars: &IndexMap<String, String>,
) -> Vec<(usize, &'a str)> {
    tokens(template, vars)
        .into_iter()
        .filter_map(|t| match t {
            Token::Shell { raw, line } => Some((line, raw)),
            _ => None,
        })
        .collect()
}

/// Title case, Jinja's rule (I-137, owner 2026-10-17): each word's first
/// letter upper and the rest lower; a word starts after whitespace or one of
/// `-([{<`. `hello wORLD` → `Hello World`, `my API` → `My Api`. The old
/// behaviour, first character only, is `|capitalize`.
fn title_case(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut word_start = true;
    for c in v.chars() {
        if c.is_whitespace() || matches!(c, '-' | '(' | '{' | '[' | '<') {
            out.push(c);
            word_start = true;
        } else if word_start {
            out.extend(c.to_uppercase());
            word_start = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// A value for an error message: masked when its name or shape says secret
/// (`core::mask`), so a bad `|int` on `DB_PASSWORD` does not print it.
fn shown(key: &str, value: &str) -> String {
    crate::core::mask::mask(key, value)
}

/// Detect variable references in template that are not defined in the variable map.
///
/// `${NAME}` and `{{NAME}}` count, unless a `default:` in the chain supplies a
/// value. ⛔ Not a bare `$NAME` (DEC-T3, I-135): in nginx, shell or Makefile
/// text (`$host`, `$document_root`, `$1`) it is that program's syntax, not a
/// missing variable.
///
/// # Example
///
/// ```
/// # use indexmap::IndexMap;
/// # use evnx::commands::template::detect_undefined_vars;
/// let template = "url=${DATABASE_URL}, host={{HOST}}";
/// let mut vars = IndexMap::new();
/// vars.insert("DATABASE_URL".to_string(), "postgresql://localhost".to_string());
/// // HOST is not defined
///
/// let undefined = detect_undefined_vars(template, &vars);
/// assert_eq!(undefined, vec!["HOST"]);
/// ```
pub fn detect_undefined_vars(template: &str, vars: &IndexMap<String, String>) -> Vec<String> {
    let mut undefined: Vec<String> = Vec::new();
    for token in tokens(template, vars) {
        let Token::Ref(r) = token else { continue };
        let defaulted = r.filters.iter().any(|f| matches!(f, Filter::Default(_)));
        if r.bare || defaulted || vars.contains_key(r.name) || !is_identifier(r.name) {
            continue;
        }
        if !undefined.iter().any(|u| u == r.name) {
            undefined.push(r.name.to_string());
        }
    }
    undefined
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Render through the real engine. Tests read the output, never a
    /// pattern: a pattern test cannot see that a value gets re-scanned.
    fn render(template: &str, pairs: &[(&str, &str)]) -> String {
        let vars: IndexMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        process_template(template, &vars).unwrap()
    }

    #[test]
    fn both_braced_styles_substitute() {
        assert_eq!(render("${MY_VAR}", &[("MY_VAR", "v")]), "v");
        assert_eq!(render("{{MY_VAR}}", &[("MY_VAR", "v")]), "v");
        assert_eq!(render("{{KEY|bool}}", &[("KEY", "yes")]), "true");
        assert_eq!(render("{{OPT|default:x}}", &[]), "x");
    }

    /// Whitespace inside a placeholder is how most people write one, and every
    /// one of these forms rendered literally into the output file before v0.5.0.
    #[test]
    fn whitespace_inside_a_placeholder_is_tolerated() {
        assert_eq!(render("${ V }", &[("V", "x")]), "x");
        assert_eq!(render("{{ V }}", &[("V", "x")]), "x");
        assert_eq!(render("{{ V | upper }}", &[("V", "x")]), "X");
        assert_eq!(render("{{ V | default:x }}", &[("V", "")]), "x");
    }

    #[test]
    fn test_process_template_simple_substitution() {
        let template = "DATABASE_URL=${DATABASE_URL}";
        let mut vars = IndexMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "DATABASE_URL=postgresql://localhost");
    }

    #[test]
    fn test_process_template_double_braces() {
        let template = "url: {{DATABASE_URL}}";
        let mut vars = IndexMap::new();
        vars.insert(
            "DATABASE_URL".to_string(),
            "postgresql://localhost".to_string(),
        );

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "url: postgresql://localhost");
    }

    #[test]
    fn test_process_template_simple_prefix() {
        let template = "host: $HOST port: $PORT";
        let mut vars = IndexMap::new();
        vars.insert("HOST".to_string(), "localhost".to_string());
        vars.insert("PORT".to_string(), "3000".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "host: localhost port: 3000");
    }

    #[test]
    fn test_filter_order_prevents_conflicts() {
        // Ensure {{VAR|upper}} is not partially replaced by {{VAR}} substitution
        let template = "env={{ENV|upper}} raw={{ENV}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "production".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "env=PRODUCTION raw=production");
    }

    #[test]
    fn test_filter_upper() {
        let template = "ENV={{ENV|upper}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "production".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "ENV=PRODUCTION");
    }

    #[test]
    fn test_filter_lower() {
        let template = "ENV={{ENV|lower}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "PRODUCTION".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "ENV=production");
    }

    #[test]
    fn test_filter_title() {
        let template = "name={{NAME|title}}";
        let mut vars = IndexMap::new();
        vars.insert("NAME".to_string(), "hello world".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "name=Hello World");
    }

    #[test]
    fn test_filter_capitalize_keeps_the_rest() {
        let mut vars = IndexMap::new();
        vars.insert("NAME".to_string(), "hello wORLD".to_string());
        let result = process_template("name={{NAME|capitalize}}", &vars).unwrap();
        assert_eq!(result, "name=Hello wORLD");
    }

    #[test]
    fn title_case_follows_jinja() {
        assert_eq!(title_case("hello wORLD"), "Hello World");
        assert_eq!(title_case("my API key"), "My Api Key");
        assert_eq!(title_case("a-b (c) [d] {e} <f>"), "A-B (C) [D] {E} <F>");
        assert_eq!(title_case("o'neil x_y"), "O'neil X_y");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn test_filter_bool_truthy_values() {
        let test_cases = vec!["true", "True", "TRUE", "yes", "Yes", "1", "on", "Y"];
        for truthy in test_cases {
            let template = "debug={{DEBUG|bool}}";
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), truthy.to_string());

            let result = process_template(template, &vars).unwrap();
            assert_eq!(result, "debug=true", "Failed for truthy value: {}", truthy);
        }
    }

    #[test]
    fn test_filter_bool_unknown_words_are_an_error() {
        for word in ["maybe", "", "enabled"] {
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), word.to_string());
            let err = process_template("debug={{DEBUG|bool}}", &vars).unwrap_err();
            assert!(err.to_string().contains("not a boolean"), "{word:?}: {err}");
        }
    }

    #[test]
    fn test_filter_bool_falsy_values() {
        let test_cases = vec!["false", "no", "0", "off", "N"];
        for falsy in test_cases {
            let template = "debug={{DEBUG|bool}}";
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), falsy.to_string());

            let result = process_template(template, &vars).unwrap();
            assert_eq!(result, "debug=false", "Failed for falsy value: {}", falsy);
        }
    }

    #[test]
    fn test_filter_int_valid() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "8000".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "port=8000");
    }

    #[test]
    fn test_filter_int_invalid_error() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "not-a-number".to_string());

        let result = process_template(template, &vars);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a valid integer"));
    }

    #[test]
    fn test_filter_json_proper_escaping() {
        let template = r#"value={{VALUE|json}}"#;
        let mut vars = IndexMap::new();
        // Value containing quotes and special chars
        vars.insert("VALUE".to_string(), r#"hello "world" & <test>"#.to_string());

        let result = process_template(template, &vars).unwrap();
        // Should be properly JSON-escaped WITH quotes
        assert_eq!(result, r#"value="hello \"world\" & <test>""#);
    }

    #[test]
    fn test_filter_default_when_empty() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "".to_string()); // empty value

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "opt=fallback");
    }

    #[test]
    fn test_filter_default_when_defined() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "actual_value".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "opt=actual_value");
    }

    #[test]
    fn test_detect_undefined_vars_all_styles() {
        let template = "${UNDEF1} {{UNDEF2}} $UNDEF3 {{DEFINED|upper}}";
        let mut vars = IndexMap::new();
        vars.insert("DEFINED".to_string(), "value".to_string());

        // I-135, DEC-T3: bare `$UNDEF3` is another program's syntax, not a
        // missing variable.
        let undefined = detect_undefined_vars(template, &vars);
        assert_eq!(undefined, vec!["UNDEF1", "UNDEF2"]);
    }

    #[test]
    fn test_detect_undefined_vars_no_duplicates() {
        let template = "${VAR} {{VAR}} ${ VAR }";
        let vars = IndexMap::new(); // VAR not defined

        let undefined = detect_undefined_vars(template, &vars);
        // Should report VAR only once despite 3 references
        assert_eq!(undefined, vec!["VAR"]);
    }

    #[test]
    fn test_regex_escaping_for_special_var_names() {
        // Test that variable names with regex special chars are handled safely
        let template = "${VAR_NAME}";
        let mut vars = IndexMap::new();
        vars.insert("VAR_NAME".to_string(), "value".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "value");
    }

    #[test]
    fn test_mixed_substitution_styles() {
        let template = "${A} {{B}} $C";
        let mut vars = IndexMap::new();
        vars.insert("A".to_string(), "1".to_string());
        vars.insert("B".to_string(), "2".to_string());
        vars.insert("C".to_string(), "3".to_string());

        let result = process_template(template, &vars).unwrap();
        assert_eq!(result, "1 2 3");
    }

    /// ⚠️ A dot is a regex metacharacter, so a key containing one must stay
    /// escaped. The single-pass rewrite builds its alternation with
    /// `regex::escape` over the real keys for exactly this reason — a generic
    /// `\w+` class would have dropped dotted keys silently.
    #[test]
    fn a_dotted_key_is_matched_literally() {
        let vars = &[("VAR.WITH.DOTS", "v")];
        assert_eq!(render("${VAR.WITH.DOTS}", vars), "v");
        assert_eq!(render("{{VAR.WITH.DOTS}}", vars), "v");
        // The dot must not act as "any character".
        assert_eq!(render("${VARxWITHyDOTS}", vars), "${VARxWITHyDOTS}");
        assert_eq!(render("{{VARxWITHyDOTS}}", vars), "{{VARxWITHyDOTS}}");
    }

    /// A key must not swallow a longer neighbouring name.
    #[test]
    fn a_key_does_not_shadow_a_longer_one() {
        assert_eq!(render("{{PORTAL}}", &[("PORT", "8080")]), "{{PORTAL}}");
        assert_eq!(
            render("{{PORTAL}}", &[("PORT", "8080"), ("PORTAL", "gate")]),
            "gate"
        );
        assert_eq!(render("$PORTAL", &[("PORT", "8080")]), "$PORTAL");
    }

    /// ⛔ The two defects this rewrite closed, as behaviour.
    #[test]
    fn a_value_is_substituted_literally_and_only_once() {
        // `$` in a value is not a capture reference.
        assert_eq!(
            render("p={{DB_PASSWORD}}", &[("DB_PASSWORD", "pa$sw0rd")]),
            "p=pa$sw0rd"
        );
        // A substituted value is never re-scanned.
        assert_eq!(
            render("b={{BRACE}}", &[("BRACE", "{{OTHER}}"), ("OTHER", "plain")]),
            "b={{OTHER}}"
        );
        // An unknown reference is left exactly as written.
        assert_eq!(render("{{NOPE}}", &[("A", "1")]), "{{NOPE}}");
    }

    #[test]
    fn whitespace_around_a_filter_is_tolerated() {
        for form in [
            "{{MY_VAR|upper}}",
            "{{ MY_VAR | upper }}",
            "{{MY_VAR |upper}}",
        ] {
            assert_eq!(render(form, &[("MY_VAR", "v")]), "V", "{form}");
        }
    }

    /// I-133: a value that looks like a template is written as it is, in
    /// either key order. The per-key filter loop evaluated it in one order.
    #[test]
    fn b3_i133_a_value_is_never_read_as_a_template() {
        for pairs in [
            [("A", "{{B|upper}}"), ("B", "bee")],
            [("B", "bee"), ("A", "{{B|upper}}")],
        ] {
            assert_eq!(
                render("{{A|json}} {{A}}", &pairs),
                r#""{{B|upper}}" {{B|upper}}"#
            );
        }
    }

    /// The same template renders the same under every order of the keys,
    /// with values that hold every reference style (T-TPL-12).
    #[test]
    fn b3_i133_key_order_never_changes_the_output() {
        let pairs = [
            ("A", "${B}"),
            ("B", "{{C|upper}}"),
            ("C", "$A"),
            ("D", "\\${A}"),
        ];
        let template = "${A} {{B}} $C {{D|json}} {{C|upper}} {{NOPE|default:$A}}\n";
        let orders = [
            [0, 1, 2, 3],
            [3, 2, 1, 0],
            [1, 3, 0, 2],
            [2, 0, 3, 1],
            [0, 2, 1, 3],
            [3, 1, 2, 0],
        ];
        let rendered: Vec<String> = orders
            .iter()
            .map(|o| render(template, &o.map(|i| pairs[i])))
            .collect();
        assert!(rendered.windows(2).all(|w| w[0] == w[1]), "{rendered:?}");
        assert_eq!(rendered[0], "${B} {{C|upper}} $A \"\\\\${A}\" $A $A\n");
    }

    /// I-134: each `default:` uses its own text.
    #[test]
    fn b3_i134_each_default_has_its_own_text() {
        assert_eq!(
            render("{{E|default:one}} {{E|default:two}}", &[("E", "")]),
            "one two"
        );
        assert_eq!(
            render("{{E|default:one}} {{E|default:two}}", &[]),
            "one two"
        );
        // Spaces, `:` and a lone `}` are the default's text.
        assert_eq!(render("{{E|default: a:b }c }}", &[]), "a:b }c");
    }

    /// Filters chain left to right; a default then a filter is a chain, and a
    /// pipe followed by anything else is the default's text.
    #[test]
    fn b3_filters_chain_left_to_right() {
        assert_eq!(render("{{X|upper|json}}", &[("X", "a\"b")]), "\"A\\\"B\"");
        assert_eq!(render("{{PORT|default:8080|int}}", &[("PORT", "")]), "8080");
        assert_eq!(
            render("{{PORT|default:8080|int}}", &[("PORT", "9000")]),
            "9000"
        );
        assert_eq!(render("{{X|default:a|b}}", &[]), "a|b");
        assert_eq!(render("{{X|default:x | upper}}", &[]), "X");
        // No value and no default: written as it is.
        assert_eq!(render("{{NOPE|upper}}", &[]), "{{NOPE|upper}}");
        assert!(
            detect_undefined_vars("{{NOPE|upper}} {{N2|upper|default:x}}", &IndexMap::new())
                == ["NOPE"]
        );
    }

    /// DEC-T3 (I-135): a bare `$NAME` is substituted only when defined, and
    /// never reported; `${NAME}` and `{{NAME}}` always are.
    #[test]
    fn b3_i135_a_bare_name_is_only_ever_a_defined_variable() {
        let t = "root $document_root; $1 $HOME ${PORT} $PORT";
        assert_eq!(
            render(t, &[("PORT", "80")]),
            "root $document_root; $1 $HOME 80 80"
        );
        assert!(detect_undefined_vars("$HOME $host", &IndexMap::new()).is_empty());
        assert_eq!(
            detect_undefined_vars("${HOME} {{host}}", &IndexMap::new()),
            ["HOME", "host"]
        );
    }

    /// DEC-T5: a backslash makes a reference literal; `\\` before one is one
    /// backslash. Every other backslash is text, so `C:\\x` and `\n` stay.
    #[test]
    fn b3_dec_t5_a_backslash_escapes_a_reference() {
        let v = &[("X", "v")];
        assert_eq!(render(r"\${X} \{{X}} \$X", v), "${X} {{X}} $X");
        assert_eq!(render(r"\\${X} \\{{X}}", v), r"\v \v");
        assert_eq!(render(r#""C:\\dir\n" \x"#, v), r#""C:\\dir\n" \x"#);
        assert!(detect_undefined_vars(r"\${NOPE} \{{NOPE}}", &IndexMap::new()).is_empty());
    }

    /// DEC-T10: values quoted for the syntax they land in.
    #[test]
    fn b3_dec_t10_yaml_toml_and_sh_quote_safely() {
        let v = &[("V", "a\"b\\c\nd'e\u{7}")];
        assert_eq!(render("{{V|yaml}}", v), r#""a\"b\\c\nd'e\u0007""#);
        assert_eq!(render("{{V|toml}}", v), r#""a\"b\\c\nd'e\u0007""#);
        assert_eq!(render("{{V|sh}}", v), "'a\"b\\c\nd'\\''e\u{7}'");
        // What the YAML and TOML readers make of it is the value again.
        let yaml: String = serde_yaml::from_str(&render("{{V|yaml}}", v)).unwrap();
        assert_eq!(yaml, v[0].1);
        let toml: toml::Table = render("k = {{V|toml}}", v).parse().unwrap();
        assert_eq!(toml["k"].as_str(), Some(v[0].1));
    }

    /// I-136: `${X:-d}` is not evnx syntax. It is written as it is, and
    /// reported with its line so it is never silent.
    #[test]
    fn b3_i136_shell_expansions_are_written_through_and_found() {
        let t = "a\nb=${X:-d} ${Y?e} ${1}";
        assert_eq!(render(t, &[("X", "x")]), t);
        let found = shell_expansions(t, &IndexMap::new());
        assert_eq!(found, [(2, "${X:-d}"), (2, "${Y?e}")]);
    }

    /// I-143: 2,000 keys over 3,000 lines took 4.2 s; the budget is 400 ms
    /// (debug builds included).
    #[test]
    fn b3_i143_two_thousand_keys_render_within_budget() {
        let vars: IndexMap<String, String> = (0..2000)
            .map(|i| (format!("KEY_{i}"), format!("value-{i}")))
            .collect();
        let template: String = (0..3000)
            .map(|i| {
                format!(
                    "a{i}: ${{KEY_{}}} {{{{KEY_{}|upper}}}} $KEY_{}\n",
                    i % 2000,
                    (i + 1) % 2000,
                    (i + 2) % 2000
                )
            })
            .collect();
        let start = std::time::Instant::now();
        let out = process_template(&template, &vars).unwrap();
        let took = start.elapsed();
        assert!(out.starts_with("a0: value-0 VALUE-1 value-2\n"));
        assert!(took < std::time::Duration::from_millis(400), "{took:?}");
    }
}
