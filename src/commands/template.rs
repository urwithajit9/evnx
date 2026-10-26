//! Template engine for generating configuration files from `.env` files.
//!
//! This module provides a flexible template substitution system that supports:
//!
//! - Multiple variable syntax styles: `${VAR}`, `{{VAR}}`, `$VAR`
//! - Filter transformations: `|upper`, `|lower`, `|title`, `|capitalize`, `|bool`, `|int`, `|json`, `|default:value`
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
//! ```
//!
//! Filters are processed **before** simple substitution to avoid pattern conflicts.
//! One filter per placeholder: an unknown filter, or two chained
//! (`{{X|upper|json}}`), is an error naming its line, and nothing is written.
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
//! evnx template --input template.yaml --output output.yaml --dry-run --strict
//!
//! # Into a directory that does not exist yet, and print to stdout
//! evnx template --input template.yaml --output deploy/prod/output.yaml --mkdir
//! evnx template --input template.yaml --output - | kubectl apply -f -
//! ```
//!
//! # Error Handling
//!
//! - Undefined variables trigger a warning and exit 0; `--strict` makes them an
//!   error, before anything is written
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
use regex::Regex;
use std::collections::HashSet;
use std::fs;

use colored::Colorize;

use crate::core::Parser;
use crate::docs;
/// How `template` updates `.gitignore`; the flow is shared (`utils::protect`).
pub use crate::utils::protect::GitignoreMode;
use crate::utils::ui;

// ─────────────────────────────────────────────────────────────
// Regex Pattern Helpers (avoid format! brace escaping hell)
// ─────────────────────────────────────────────────────────────

/// Build regex pattern to match `{{VAR|filter}}` style references.
///
/// The `filter` argument **must include the leading pipe**, e.g. `"|upper"`, `"|bool"`.
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - used internally by process_filters
/// let pattern = filter_var_pattern("ENV", "|upper");
/// // pattern == r"\{\{ENV\|upper\}\}" as a regex string
/// ```
fn filter_var_pattern(key: &str, filter: &str) -> String {
    // `filter` arrives with its leading pipe (`"|upper"`). The pipe is split out
    // so whitespace can be allowed around it: `{{ ENV | upper }}` is the form
    // people write, and it previously matched nothing.
    let name = filter.strip_prefix('|').unwrap_or(filter);
    format!(
        "\\{{\\{{\\s*{}\\s*\\|\\s*{}\\s*\\}}\\}}",
        regex::escape(key),
        regex::escape(name)
    )
}

/// Build regex pattern to match `{{VAR|default:VALUE}}` with capture group.
///
/// The capture group extracts the default value for substitution.
fn default_var_pattern(key: &str) -> String {
    format!(
        "\\{{\\{{\\s*{}\\s*\\|\\s*default:\\s*([^}}]*?)\\s*\\}}\\}}",
        regex::escape(key)
    )
}

/// Match **any** `{{ NAME|default:VALUE }}`, whatever `NAME` is.
///
/// ⚠️ Every other pattern here is built from a key that is already in the
/// environment, which made `|default:` exactly backwards: the fallback fired
/// only when the variable existed — the one case it is not needed — and a
/// genuinely missing variable rendered `{{NOPE|default:1234}}` into the output
/// file. This pattern is keyed on nothing, so it can see the absent case.
fn any_default_pattern() -> &'static str {
    r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\|\s*default:\s*([^}]*?)\s*\}\}"
}

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
    /// `--strict`: refuse to write when a placeholder has no value.
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
            "{} placeholder{} left unresolved and written through literally. \
             Use --strict to make this an error.",
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
];

/// Refuse a filter evnx does not know, and chained filters, naming each one
/// with its line (DEC-T7). They used to be written into the output unchanged,
/// so `{{PORT|innt}}` shipped as literal text with exit 0.
///
/// Every `{{ NAME | … }}` counts when NAME is an identifier or a key in the
/// env file, whether it has a value or not (owner, 2026-10-17). Text that is
/// not an evnx reference — Helm's `{{ .Values.x | quote }}` — is left alone.
///
/// `|default:` takes the rest of the braces as its text, pipes included,
/// unless a pipe is followed by a filter name: that is a chain.
///
/// ⚠️ Chained filters (`{{X|upper|json}}`) are refused until the one
/// substitution engine (T-WP3, 0.11.0) can apply them left to right; going
/// from an error to working later breaks nobody.
fn check_filters(
    template: &str,
    vars: &IndexMap<String, String>,
    input: &str,
    output: &str,
) -> Result<()> {
    let re = Regex::new(r"\{\{\s*([^\s{}|]+)\s*\|([^}]*)\}\}").expect("valid filter pattern");
    let mut problems = Vec::new();
    for caps in re.captures_iter(template) {
        let (Some(whole), Some(name), Some(chunk)) = (caps.get(0), caps.get(1), caps.get(2)) else {
            continue;
        };
        if !is_identifier(name.as_str()) && !vars.contains_key(name.as_str()) {
            continue;
        }
        let chunk = chunk.as_str().trim();
        let line = template[..whole.start()].matches('\n').count() + 1;
        let placeholder = whole.as_str();
        if let Some(text) = chunk.strip_prefix("default:") {
            // The default's text runs to the braces, pipes included — unless a
            // pipe is followed by a filter: `{{PORT|default:8080|int}}` is a
            // chain, and rendered `8080|int` with exit 0 (found after #190).
            if text.split('|').skip(1).any(|seg| {
                let seg = seg.trim();
                FILTERS.contains(&seg) || seg.starts_with("default:")
            }) {
                problems.push(format!(
                    "line {line}: {placeholder}: chained filters are not supported; \
                     use one filter per placeholder"
                ));
            }
            continue;
        }
        if chunk.contains('|') {
            problems.push(format!(
                "line {line}: {placeholder}: chained filters are not supported; \
                 use one filter per placeholder"
            ));
        } else if !FILTERS.contains(&chunk) {
            let mut msg = format!("line {line}: {placeholder}: unknown filter `{chunk}`");
            let near = crate::utils::string::closest(
                &chunk.to_lowercase(),
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
         Filters: {}, default:VALUE.\n\
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

/// Process a template string with variable substitution and filter transformations.
///
/// This function applies transformations in a specific order to avoid conflicts:
/// 1. **Filters first**: Process `{{VAR|filter}}` patterns with transformations
/// 2. **Simple substitution**: Replace remaining `${VAR}`, `{{VAR}}`, `$VAR` patterns
///
/// This ordering ensures that filtered variables are not accidentally replaced
/// by the simple substitution pass.
///
/// # Arguments
///
/// * `template` - The template string containing variable references
/// * `vars` - Map of variable names to their string values
///
/// # Returns
///
/// * `Ok(String)` - The processed template with all substitutions applied
/// * `Err(anyhow::Error)` - If a filter transformation fails (e.g., invalid integer)
///
/// # Supported Patterns
///
/// | Pattern | Description | Example |
/// |---------|-------------|---------|
/// | `${VAR}` | Shell-style substitution | `${PORT}` → `8000` |
/// | `{{VAR}}` | Template-style substitution | `{{PORT}}` → `8000` |
/// | `$VAR` | Simple prefix substitution | `$PORT` → `8000` |
/// | `{{VAR|upper}}` | Uppercase filter | `{{env|upper}}` → `PRODUCTION` |
/// | `{{VAR|bool}}` | Boolean conversion | `{{debug|bool}}` → `true` |
/// | `{{VAR|int}}` | Integer parsing | `{{port|int}}` → `8000` |
/// | `{{VAR|json}}` | JSON string escaping | `{{name|json}}` → `"hello \"world\""` |
/// | `{{VAR|default:foo}}` | Default value if undefined | `{{opt|default:bar}}` → `bar` |
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
    let mut result = template.to_string();

    // Step 1: Process filters FIRST (before simple substitution)
    // This prevents {{VAR|filter}} from being partially replaced by $VAR or {{VAR}} patterns
    result = process_filters(&result, vars)?;

    // Step 2: Simple variable substitution for remaining unfiltered references
    result = process_simple_substitution(&result, vars);

    // Step 3: Any `|default:` still standing names a variable that is not in the
    // environment at all, so steps 1 and 2 — which iterate over the variables
    // that *are* — could not have seen it. That is the whole purpose of a
    // default, and until this step it was the one case that did not work.
    result = apply_absent_defaults(&result);

    Ok(result)
}

/// Resolve `{{ NAME|default:VALUE }}` for names with no value in the environment.
///
/// Runs last, so a variable that *does* have a value has already won — a default
/// is a fallback, never an override.
fn apply_absent_defaults(template: &str) -> String {
    let Ok(re) = Regex::new(any_default_pattern()) else {
        return template.to_string();
    };
    re.replace_all(template, "$2").to_string()
}

/// Apply simple variable substitution patterns: `${VAR}`, `{{VAR}}`, `$VAR`.
///
/// This function handles basic substitution without filters. It should be called
/// **after** `process_filters` to avoid interfering with filtered patterns.
///
/// # Arguments
///
/// * `template` - Template string after filter processing
/// * `vars` - Variable map for substitution values
///
/// # Returns
///
/// String with all simple variable references replaced
fn process_simple_substitution(template: &str, vars: &IndexMap<String, String>) -> String {
    // ⛔ **One pass over the template, not one pass per variable.**
    //
    // This used to loop over `vars`, replacing each key across the whole
    // accumulating result. So a value substituted early was **re-scanned** by
    // every later iteration:
    //
    // ```text
    // BRACE='{{OTHER}}'   OTHER=plain
    //   brace={{BRACE}}   →   brace=plain
    // ```
    //
    // The secret's own text was treated as a template, and whether that happened
    // depended on key order — so the same file could render differently after an
    // unrelated edit.
    //
    // ⚠️ The alternation is built from the **actual keys**, not from a generic
    // `\w+`. `regex::escape` keeps keys containing dots working, which a
    // character class would have silently dropped.
    if vars.is_empty() {
        return template.to_string();
    }

    let alt = vars
        .keys()
        .map(|k| regex::escape(k))
        .collect::<Vec<_>>()
        .join("|");

    // ⚠️ The `\s*` around the key is load-bearing and predates this rewrite:
    // without it `${ PORT }` matched nothing and was written to the output file
    // verbatim with a zero exit code. Whitespace inside a placeholder is how
    // most people write one, so the failure was common and invisible.
    let pattern = format!(r"\$\{{\s*({alt})\s*\}}|\{{\{{\s*({alt})\s*\}}\}}|\$({alt})\b");
    let Ok(re) = Regex::new(&pattern) else {
        return template.to_string();
    };

    re.replace_all(template, |caps: &regex::Captures| {
        let key = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
            .unwrap_or_default();

        match vars.get(key) {
            // ⚠️ Returning the value from a closure puts it in **verbatim**.
            // A `&str` replacement is a template, in which `$` names a capture
            // group — which is what turned `DB_PASSWORD=pa$sw0rd` into `pa`,
            // silently and with exit 0.
            Some(value) => value.clone(),
            None => caps
                .get(0)
                .map(|m| m.as_str())
                .unwrap_or_default()
                .to_string(),
        }
    })
    .to_string()
}

/// Type alias for filter transformation functions.
///
/// Each filter takes a string value and returns a transformed string.
type FilterFn = fn(&str) -> String;

/// Registry of built-in filter patterns and their transformation functions.
///
/// Returns a vector of (pattern_suffix, transformation_function) pairs.
/// The pattern suffix **includes the leading pipe** and is appended to the variable
/// name within `{{VAR|suffix}}` syntax, e.g. `"|upper"` matches `{{VAR|upper}}`.
///
/// # Supported Filters
///
/// | Filter | Description | Example Input → Output |
/// |--------|-------------|------------------------|
/// | `upper` | Convert to uppercase | `"hello"` → `"HELLO"` |
/// | `lower` | Convert to lowercase | `"HELLO"` → `"hello"` |
/// | `title` | Title case | `"hello wORLD"` → `"Hello World"` |
/// | `capitalize` | First character upper | `"hello wORLD"` → `"Hello wORLD"` |
///
/// # Example (internal use only)
///
/// ```ignore
/// // This function is private - used internally by process_simple_substitution
/// let pattern = shell_var_pattern("MY_VAR");
/// // pattern == r"\$\{MY_VAR\}" as a regex string
/// ```
fn filter_patterns() -> Vec<(&'static str, FilterFn)> {
    vec![
        ("|upper", |v| v.to_uppercase()),
        ("|lower", |v| v.to_lowercase()),
        ("|title", title_case),
        // The old `|title`: the first character only, the rest as written.
        ("|capitalize", |v| {
            let mut c = v.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }),
    ]
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

/// Process template filters: transformations applied via `{{VAR|filter}}` syntax.
///
/// This function handles all filter-based substitutions using regex for reliable
/// pattern matching. Filters are processed before simple substitution to avoid
/// conflicts with base variable patterns.
///
/// # Supported Filters
///
/// - **String transforms**: `|upper`, `|lower`, `|title`
/// - **Type conversions**: `|bool`, `|int`
/// - **Encoding**: `|json` (properly escaped JSON string)
/// - **Defaults**: `|default:value` (fallback if variable is empty/undefined)
///
/// # Arguments
///
/// * `template` - Template string containing filter patterns
/// * `vars` - Variable map for lookup and transformation
///
/// # Returns
///
/// * `Ok(String)` - Template with all filters applied
/// * `Err(anyhow::Error)` - If a filter transformation fails
fn process_filters(template: &str, vars: &IndexMap<String, String>) -> Result<String> {
    let mut result = template.to_string();

    for (key, value) in vars {
        // Apply string transformation filters (upper/lower/title)
        // filter_suffix already contains the leading pipe, e.g. "|upper"
        for (filter_suffix, transform) in &filter_patterns() {
            let pattern = filter_var_pattern(key, filter_suffix);
            if let Ok(re) = Regex::new(&pattern) {
                if re.is_match(&result) {
                    let transformed = transform(value);
                    result = re
                        .replace_all(&result, regex::NoExpand(transformed.as_str()))
                        .to_string();
                }
            }
        }

        // Boolean filter: {{VAR|bool}}. One word list for evnx (core::value);
        // anything else is an error, as `|int` is (I-138, DEC-T7). It was
        // `false`, silently, for `on`, `enabled` and every typo.
        let bool_pattern = filter_var_pattern(key, "|bool");
        if let Ok(re) = Regex::new(&bool_pattern) {
            if re.is_match(&result) {
                let bool_val = crate::core::value::parse_bool(value).with_context(|| {
                    format!(
                        "Variable '{}' value '{}' is not a boolean for |bool: use {} or {}",
                        key,
                        shown(key, value),
                        crate::core::value::TRUE_WORDS.join("/"),
                        crate::core::value::FALSE_WORDS.join("/"),
                    )
                })?;
                result = re
                    .replace_all(&result, regex::NoExpand(bool_val.to_string().as_str()))
                    .to_string();
            }
        }

        // Integer filter: {{VAR|int}} - parses string to i64, errors on invalid input
        let int_pattern = filter_var_pattern(key, "|int");
        if let Ok(re) = Regex::new(&int_pattern) {
            if re.is_match(&result) {
                let int_val = value.parse::<i64>().with_context(|| {
                    format!(
                        "Variable '{}' value '{}' is not a valid integer for |int filter",
                        key,
                        shown(key, value)
                    )
                })?;
                result = re
                    .replace_all(&result, regex::NoExpand(int_val.to_string().as_str()))
                    .to_string();
            }
        }

        // JSON filter: {{VAR|json}} - properly escape value for JSON context
        let json_pattern = filter_var_pattern(key, "|json");
        if let Ok(re) = Regex::new(&json_pattern) {
            if re.is_match(&result) {
                let json_val = serde_json::to_string(value)
                    .with_context(|| format!("Failed to JSON-encode variable '{}'", key))?;
                // Keep quotes - they're part of valid JSON string representation
                result = re
                    .replace_all(&result, regex::NoExpand(json_val.as_str()))
                    .to_string();
            }
        }

        // Default value filter: {{VAR|default:fallback}}
        let default_pattern = default_var_pattern(key);
        if let Ok(re) = Regex::new(&default_pattern) {
            if re.is_match(&result) {
                let replacement = if value.trim().is_empty() {
                    // Extract default value from capture group 1
                    re.captures(&result)
                        .and_then(|caps| caps.get(1))
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default()
                } else {
                    value.clone()
                };
                result = re
                    .replace_all(&result, regex::NoExpand(replacement.as_str()))
                    .to_string();
            }
        }
    }

    Ok(result)
}

/// Detect variable references in template that are not defined in the variable map.
///
/// Scans the template for all supported variable patterns and returns a list
/// of undefined variable names. This helps catch configuration errors early.
///
/// # Arguments
///
/// * `template` - Template string to scan for variable references
/// * `vars` - Map of defined variable names
///
/// # Returns
///
/// Vec of undefined variable names (deduplicated)
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
    // Regex pattern matching all supported variable reference styles:
    // - ${VAR}: shell-style with braces
    // - {{VAR}} or {{VAR|filter}}: template-style with optional filter
    // - $VAR: simple prefix style with word boundary
    // ⚠️ `\s*` throughout. Without it this agreed with the substituter's blind
    // spot — `{{ PORT }}` was neither substituted nor reported, so the one signal
    // that something was wrong was silent for exactly the forms that failed.
    //
    // Group 3 captures the filter chunk so a `|default:` can be recognised: a
    // placeholder with a fallback is resolved, not undefined.
    let re = Regex::new(
        r"(?:\$\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\})|(?:\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*(\|[^}]*)?\}\})|(?:\$([A-Za-z_][A-Za-z0-9_]*))\b"
    ).expect("Variable detection regex is valid");

    let mut undefined = Vec::new();
    let mut seen = HashSet::new();

    for caps in re.captures_iter(template) {
        // A `|default:` supplies a value, so the variable is not undefined.
        if caps.get(3).is_some_and(|f| f.as_str().contains("default:")) {
            continue;
        }

        // Variable name could be in capture group 1, 2, or 4 depending on style
        let var_name = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(4))
            .map(|m| m.as_str().to_string());

        if let Some(name) = var_name {
            // Only report each undefined variable once, and skip if defined
            if !vars.contains_key(&name) && !seen.contains(&name) {
                undefined.push(name.clone());
                seen.insert(name);
            }
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

    /// Assert what a pattern *matches*, never its source text.
    ///
    /// ⚠️ These tests used to compare the generated regex string literally, which
    /// pinned the implementation rather than the behaviour — and pinned it to a
    /// version that could not match `{{ VAR }}`. A test that fails when the bug is
    /// fixed is worse than no test.
    fn matches(pattern: &str, haystack: &str) -> bool {
        Regex::new(pattern)
            .expect("valid pattern")
            .is_match(haystack)
    }

    /// Render through the real substitution path, which is what the removed
    /// `shell_var_pattern` / `template_var_pattern` helpers used to feed.
    ///
    /// ⚠️ These assertions used to check the *pattern string* those helpers
    /// built. Testing the rendered output instead is why the single-pass
    /// rewrite could be verified at all: a pattern test cannot see that a
    /// substituted value gets re-scanned.
    fn render(template: &str, pairs: &[(&str, &str)]) -> String {
        let vars: IndexMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        process_simple_substitution(template, &vars)
    }

    #[test]
    fn test_pattern_helpers() {
        assert_eq!(render("${MY_VAR}", &[("MY_VAR", "v")]), "v");
        assert_eq!(render("{{MY_VAR}}", &[("MY_VAR", "v")]), "v");
        assert!(matches(&filter_var_pattern("KEY", "|bool"), "{{KEY|bool}}"));
        assert!(matches(&default_var_pattern("OPT"), "{{OPT|default:x}}"));
    }

    /// Whitespace inside a placeholder is how most people write one, and every
    /// one of these forms rendered literally into the output file before v0.5.0.
    #[test]
    fn whitespace_inside_a_placeholder_is_tolerated() {
        assert_eq!(render("${ V }", &[("V", "x")]), "x");
        assert_eq!(render("{{ V }}", &[("V", "x")]), "x");
        assert!(matches(
            &filter_var_pattern("V", "|upper"),
            "{{ V | upper }}"
        ));
        assert!(matches(&default_var_pattern("V"), "{{ V | default:x }}"));
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

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "ENV=PRODUCTION");
    }

    #[test]
    fn test_filter_lower() {
        let template = "ENV={{ENV|lower}}";
        let mut vars = IndexMap::new();
        vars.insert("ENV".to_string(), "PRODUCTION".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "ENV=production");
    }

    #[test]
    fn test_filter_title() {
        let template = "name={{NAME|title}}";
        let mut vars = IndexMap::new();
        vars.insert("NAME".to_string(), "hello world".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "name=Hello World");
    }

    #[test]
    fn test_filter_capitalize_keeps_the_rest() {
        let mut vars = IndexMap::new();
        vars.insert("NAME".to_string(), "hello wORLD".to_string());
        let result = process_filters("name={{NAME|capitalize}}", &vars).unwrap();
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

            let result = process_filters(template, &vars).unwrap();
            assert_eq!(result, "debug=true", "Failed for truthy value: {}", truthy);
        }
    }

    #[test]
    fn test_filter_bool_unknown_words_are_an_error() {
        for word in ["maybe", "", "enabled"] {
            let mut vars = IndexMap::new();
            vars.insert("DEBUG".to_string(), word.to_string());
            let err = process_filters("debug={{DEBUG|bool}}", &vars).unwrap_err();
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

            let result = process_filters(template, &vars).unwrap();
            assert_eq!(result, "debug=false", "Failed for falsy value: {}", falsy);
        }
    }

    #[test]
    fn test_filter_int_valid() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "8000".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "port=8000");
    }

    #[test]
    fn test_filter_int_invalid_error() {
        let template = "port={{PORT|int}}";
        let mut vars = IndexMap::new();
        vars.insert("PORT".to_string(), "not-a-number".to_string());

        let result = process_filters(template, &vars);
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

        let result = process_filters(template, &vars).unwrap();
        // Should be properly JSON-escaped WITH quotes
        assert_eq!(result, r#"value="hello \"world\" & <test>""#);
    }

    #[test]
    fn test_filter_default_when_empty() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "".to_string()); // empty value

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "opt=fallback");
    }

    #[test]
    fn test_filter_default_when_defined() {
        let template = "opt={{OPTIONAL|default:fallback}}";
        let mut vars = IndexMap::new();
        vars.insert("OPTIONAL".to_string(), "actual_value".to_string());

        let result = process_filters(template, &vars).unwrap();
        assert_eq!(result, "opt=actual_value");
    }

    #[test]
    fn test_detect_undefined_vars_all_styles() {
        let template = "${UNDEF1} {{UNDEF2}} $UNDEF3 {{DEFINED|upper}}";
        let mut vars = IndexMap::new();
        vars.insert("DEFINED".to_string(), "value".to_string());
        // UNDEF1, UNDEF2, UNDEF3 are not defined

        let undefined = detect_undefined_vars(template, &vars);
        assert_eq!(undefined.len(), 3);
        assert!(undefined.contains(&"UNDEF1".to_string()));
        assert!(undefined.contains(&"UNDEF2".to_string()));
        assert!(undefined.contains(&"UNDEF3".to_string()));
    }

    #[test]
    fn test_detect_undefined_vars_no_duplicates() {
        let template = "${VAR} {{VAR}} $VAR";
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
    fn test_filter_var_pattern() {
        for form in [
            "{{MY_VAR|upper}}",
            "{{ MY_VAR | upper }}",
            "{{MY_VAR |upper}}",
        ] {
            assert!(
                matches(&filter_var_pattern("MY_VAR", "|upper"), form),
                "{form} should match"
            );
        }
        assert!(matches(&filter_var_pattern("KEY", "|bool"), "{{KEY|bool}}"));
        // A different filter must not match.
        assert!(!matches(&filter_var_pattern("KEY", "|bool"), "{{KEY|int}}"));
    }
}
