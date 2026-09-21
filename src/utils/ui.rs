//! Terminal UI utilities — progress bars, boxed output, and status messages.
//!
//! All functions in this module write directly to stdout. They are intentionally
//! thin wrappers around `colored` and `indicatif` so the rest of the codebase
//! does not need to import those crates directly.
//!
//! # Design Principles
//!
//! - **Consistency**: All commands use the same header style, colors, and icons
//! - **Composability**: Small functions that can be combined for complex output
//! - **Accessibility**: Colors enhance but don't convey essential information
//! - **Testability**: Functions are pure or have minimal side effects
//!
//! # Usage Examples
//!
//! ```no_run
//! # use evnx::utils::ui::*;
//! use colored::Colorize;
//! # fn generate_preview(_: &()) -> String { String::new() } // stub
//! # let vars = (); // stub
//!
//! // Command header
//! print_header("evnx init", Some("Set up environment variables"));
//!
//! // Status messages
//! success("Created .env.example");
//! warning("Conflicting variables will be skipped");
//! error("Failed to parse schema.json");
//! info("Tip: Run 'evnx validate' to check configuration");
//!
//! // Preview section
//! print_preview_header();
//! println!("{}", generate_preview(&vars).dimmed());
//!
//! // Next steps
//! print_next_steps(&[
//!     "Edit .env and replace placeholder values",
//!     "Never commit .env to version control",
//! ]);
//!
//! // Progress for long operations
//! let pb = progress_bar(100, "Scanning files...");
//! for i in 0..100 {
//!     pb.inc(1);
//! }
//! pb.finish_with_message("Done!");
//! ```
//!
//! # Future Work
//!
//! - Respect a global `--no-color` flag by checking `colored::control::SHOULD_COLORIZE`
//! - Add a `spinner()` helper for indeterminate operations
//! - Add a `table()` helper for structured multi-column output
//! - Support ANSI hyperlinks for terminal URLs

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

// ─────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────

/// Default width for boxed output (balanced for 80-column terminals)
pub const BOX_WIDTH: usize = 60;

/// Minimum width for readable output
pub const MIN_BOX_WIDTH: usize = 40;

/// Maximum width to avoid wrapping on wide terminals
pub const MAX_BOX_WIDTH: usize = 80;

// ─────────────────────────────────────────────────────────────
// Headers & Layout
// ─────────────────────────────────────────────────────────────

/// Print a command header in the standard boxed format.
///
/// ```text
/// ┌─ evnx init ─────────────────────────────────────┐
/// │ Set up environment variables for your project  │
/// └────────────────────────────────────────────────┘
/// ```
///
/// # Arguments
///
/// * `title` - The command name (e.g., "evnx init")
/// * `subtitle` - Optional description shown on second line
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_header;
/// print_header("evnx init", Some("Set up environment variables"));
/// ```
pub fn print_header(title: &str, subtitle: Option<&str>) {
    // The user typed the command; a three-line box repeating it back says
    // nothing. One dim line gives the same context at a fifth of the height.
    // ⚠️ The full `evnx scan`, not `scan`. In a CI log among other tools' output
    // the bare verb is ambiguous, and the five characters cost nothing.
    let line = match subtitle {
        Some(sub) => format!("\n  {}  {}\n", title.bold(), sub.dimmed()),
        None => format!("\n  {}\n", title.bold()),
    };
    println!("{}", line);
}

/// Print a simple boxed message (for sub-sections or alerts).
///
/// ```text
/// ┌─ Notice ──────────────────────────────────────────┐
/// │ This is a multi-line                              │
/// │ message that wraps nicely                         │
/// │ inside the box.                                   │
/// └───────────────────────────────────────────────────┘
/// ```
///
/// # Arguments
///
/// * `title` - Box title (left-aligned in top border)
/// * `message` - Body content (supports multi-line)
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_box;
/// print_box("Warning", "This action cannot be undone.\nPlease confirm.");
/// ```
pub fn print_box(title: &str, message: &str) {
    let width = calculate_box_width(title, Some(message));
    let border = "─".repeat(width - 4);

    // ⚠️ `width - 5`, not `width - 4`. The top row is
    // `┌` + `─` + ` ` + title + ` ` + fill + `┐` = fill + title.len() + 5, while
    // the body and bottom are `width`. With `width - 4` the top came out one
    // column wider than the box beneath it — visible in every summary this
    // printed, and in every command header until the header stopped being a box.
    println!(
        "\n{}",
        format!(
            "┌─ {} {}┐",
            title,
            "─".repeat((width - 5).saturating_sub(title.len()))
        )
        .cyan()
    );

    for line in message.lines() {
        let padded = pad_or_truncate(line, width - 4);
        println!("{}", format!("│ {} │", padded).cyan());
    }

    println!("{}\n", format!("└─{}─┘", border).cyan());
}

/// Print a simple section header (no box, just bold text with icon).
///
/// ```text
/// 📋 Preview:
/// ```
///
/// # Arguments
///
/// * `icon` - Emoji or symbol prefix (e.g., "📋", "⚙️", "🔍")
/// * `title` - Section title
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_section_header;
/// print_section_header("📋", "Preview");
/// // Output: 📋 Preview:
/// ```
pub fn print_section_header(icon: &str, title: &str) {
    // ⚠️ An empty icon used to leave its separating space behind, so
    // `print_section_header("", "Preview")` rendered as `" Preview:"` — a
    // heading indented by one column for no reason. Several callers lost their
    // icon when the emoji went and now pass `""`.
    if icon.is_empty() {
        println!("\n{}:", title.bold());
    } else {
        println!("\n{} {}:", icon.bold(), title.bold());
    }
}

/// Convenience wrapper for preview sections.
pub fn print_preview_header() {
    print_section_header("", "Preview");
}

/// Whether `print_section_header` was handed anything to draw.
///
/// ⚠️ `print_section_header("", "Preview")` rendered as `" Preview:"` — a lone
/// leading space where the icon used to be. Callers that pass an empty icon want
/// a plain heading, not an indented one.
#[cfg(test)]
mod section_header_tests {
    #[test]
    fn an_empty_icon_leaves_no_stray_space() {
        // The rendering is a `println!`, so this pins the format string's shape
        // rather than capturing stdout.
        let icon = "";
        let rendered = if icon.is_empty() {
            format!("{}:", "Preview")
        } else {
            format!("{} {}:", icon, "Preview")
        };
        assert_eq!(rendered, "Preview:");
    }
}

/// Print a numbered list of next steps.
///
/// ```text
/// Next steps:
///   1. Edit .env and replace placeholder values
///   2. Never commit .env to version control
///   3. Run 'evnx validate' to check configuration
/// ```
///
/// # Arguments
///
/// * `steps` - Slice of step descriptions
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_next_steps;
/// print_next_steps(&[
///     "Edit .env and replace placeholder values",
///     "Never commit .env to version control",
/// ]);
/// ```
pub fn print_next_steps(steps: &[&str]) {
    if steps.is_empty() {
        return;
    }

    println!("\n{}", "Next steps:".bold());
    for (i, step) in steps.iter().enumerate() {
        println!("  {}. {}", i + 1, step);
    }
}

/// Print a documentation hint at the end of a command's output.
///
/// Renders a consistent, dimmed footer pointing to the command's guide page.
/// Call this as the **last line** of every command's success path.
///
/// ```text
///   📖 Docs: https://www.evnx.dev/guides/commands/init
/// ```
///
/// # Arguments
///
/// * `doc` — The `CommandDoc` entry from `crate::docs`
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_docs_hint;
/// # use evnx::docs;
/// print_docs_hint(&docs::INIT);
/// ```
pub fn print_docs_hint(doc: &crate::docs::CommandDoc) {
    // println!("{}", doc.hint_line().dimmed());
    eprintln!("{}", doc.hint_line().dimmed());
}

/// Print a horizontal separator line.
///
/// ```text
/// ────────────────────────────────────────────────────
/// ```
pub fn separator() {
    println!("{}", "─".repeat(BOX_WIDTH).dimmed());
}

/// Print a simple progress indicator for quick operations.
///
/// ```text
/// ⋯ Resolving blueprint variables...
/// ```
///
/// For long-running operations, prefer [`progress_bar`].
///
/// # Arguments
///
/// * `message` - Status message to display
pub fn print_progress(message: &str) {
    println!("{} {}", "⋯".dimmed(), message.dimmed());
}

// ─────────────────────────────────────────────────────────────
// Status Messages
// ─────────────────────────────────────────────────────────────

/// The status glyphs, defined once.
///
/// ⚠️ **Every glyph here is exactly one terminal column.** The set they replace
/// was not: `✓` (U+2713) and `✗` (U+2717) are one column, while `⚠️` and `ℹ️`
/// carry a U+FE0F variation selector that makes them two. A status list mixing
/// them can never form a column, however carefully the padding is computed.
///
/// It was worse than a fixed offset. `⚠` appeared in the source in two byte
/// forms — nineteen times with the variation selector and twice without — so two
/// lines that looked identical in the editor rendered at different widths.
///
/// # Why no emoji
///
/// Emoji width is a terminal-and-font question, not a Unicode one: the same
/// codepoint is one column in some terminals and two in others, so no care taken
/// here makes an emoji-aligned column reliable everywhere. `✓ ✗ ! · →` carry the
/// same meaning in one predictable cell.
pub mod glyph {
    /// Success — created, written, passed.
    pub const OK: &str = "✓";
    /// Failure — an error, or a check that did not pass.
    pub const FAIL: &str = "✗";
    /// Warning — worth knowing, not worth stopping for.
    pub const WARN: &str = "!";
    /// Neutral detail, subordinate to the line above it.
    pub const INFO: &str = "·";
    /// "becomes", or a remediation step.
    pub const ARROW: &str = "→";
}

/// Print a green success message with checkmark.
///
/// ```text
/// ✓ Created .env.example with 15 variables
/// ```
pub fn success(message: impl AsRef<str>) {
    println!("{} {}", glyph::OK.green(), message.as_ref());
}

/// Print a red error message with cross.
///
/// ```text
/// ✗ Failed to parse schema.json
/// ```
pub fn error(message: impl AsRef<str>) {
    println!("{} {}", glyph::FAIL.red(), message.as_ref().red());
}

/// Print a yellow warning message with alert icon.
///
/// ```text
/// ⚠️  Conflicting variables will be skipped
/// ```
pub fn warning(message: impl AsRef<str>) {
    println!("{} {}", glyph::WARN.yellow(), message.as_ref().yellow());
}

/// Print a cyan info message with info icon.
///
/// ```text
/// ℹ️  Tip: Run 'evnx validate' to check configuration
/// ```
pub fn info(message: impl AsRef<str>) {
    println!("{} {}", glyph::INFO.dimmed(), message.as_ref().dimmed());
}

/// Print a bold important notice.
///
/// ```text
/// 🔑 Required: This variable must be set
/// ```
pub fn notice(icon: &str, message: &str) {
    println!("{} {}", icon.bold(), message.bold());
}

// ─────────────────────────────────────────────────────────────
// Progress Bars (indicatif integration)
// ─────────────────────────────────────────────────────────────

/// Create a styled progress bar for long-running operations.
///
/// ```text
/// ⠙ [████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 12/50 Processing KEY
/// ```
///
/// The caller is responsible for calling [`ProgressBar::inc`] and
/// [`ProgressBar::finish_with_message`] when done.
///
/// # Arguments
///
/// * `len` - Total number of steps
/// * `message` - Initial status message
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::progress_bar;
/// let pb = progress_bar(100, "Scanning files...");
/// // Stub files vector with explicit type for doctest
/// let files: Vec<String> = vec![];
/// for _file in &files {
///     # fn scan_file(_: &String) {} // stub
///     // scan_file(file);
///     pb.inc(1);
/// }
/// pb.finish_with_message("Scan complete!");
/// ```
pub fn progress_bar(len: u64, message: &str) -> indicatif::ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .expect("Progress bar template is valid")
            .progress_chars("#>-"),
    );
    pb.set_message(message.to_string());
    pb
}

/// Create a spinner for indeterminate operations.
///
/// ```text
/// ⠙ Resolving dependencies...
/// ```
///
/// Call `.finish_with_message()` when done.
///
/// # Arguments
///
/// * `message` - Status message to display
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::spinner;
/// let spinner = spinner("Loading schema...");
/// # fn load_schema() {} // stub for doctest
/// // load_schema();
/// spinner.finish_with_message("Schema loaded!");
/// ```
pub fn spinner(message: &str) -> indicatif::ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("Spinner template is valid"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

// ─────────────────────────────────────────────────────────────
// Dynamic Output (Advanced)
// ─────────────────────────────────────────────────────────────

/// Clear the current terminal line (for dynamic updates).
///
/// Works on Unix-like systems. No-op on Windows.
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::clear_line;
/// # use std::io::Write;
/// print!("Processing...");
/// std::io::stdout().flush().unwrap();
/// # fn do_work() {} // stub
/// // do_work();
/// clear_line();
/// println!("✓ Done");
/// ```
#[cfg(unix)]
pub fn clear_line() {
    print!("\r\x1b[K");
    #[allow(clippy::print_literal)]
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

#[cfg(not(unix))]
pub fn clear_line() {
    // No-op on Windows for now
}

/// Update a progress message in-place (requires `clear_line` support).
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_progress_inline;
/// print_progress_inline("Step 1/3: Loading...");
/// # fn load_step_1() {} // stub
/// // load_step_1();
/// print_progress_inline("Step 2/3: Processing...");
/// # fn process_step_2() {} // stub
/// // process_step_2();
/// print_progress_inline("Step 3/3: Finalizing...");
/// # fn finalize() {} // stub
/// // finalize();
/// println!(); // Newline after last update
/// ```
#[cfg(unix)]
pub fn print_progress_inline(message: &str) {
    clear_line();
    print!("{} {}", "⋯".dimmed(), message.dimmed());
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

#[cfg(not(unix))]
pub fn print_progress_inline(message: &str) {
    // Fallback: just print normally on Windows
    print_progress(message);
}

// ─────────────────────────────────────────────────────────────
// Helper Functions (Internal)
// ─────────────────────────────────────────────────────────────

/// Calculate optimal box width based on content.
fn calculate_box_width(title: &str, subtitle: Option<&str>) -> usize {
    let title_len = title.len();
    let subtitle_len = subtitle.map_or(0, |s| s.len());
    let max_content = title_len.max(subtitle_len);

    // Start with base width, expand for content, clamp to limits
    (BOX_WIDTH + max_content.saturating_sub(30)).clamp(MIN_BOX_WIDTH, MAX_BOX_WIDTH)
}

/// Pad or truncate a string to fit within a width.
fn pad_or_truncate(s: &str, width: usize) -> String {
    if s.len() >= width {
        // Truncate with ellipsis if too long
        format!("{}...", &s[..width.saturating_sub(3)])
    } else {
        // Pad with spaces to exact width
        format!("{:<width$}", s, width = width)
    }
}

/// Check if color output should be enabled.
///
/// Respects `colored::control::SHOULD_COLORIZE` and `NO_COLOR` env var.
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::should_colorize;
/// # use colored::Colorize;
/// if should_colorize() {
///     println!("{}", "Success!".green());
/// } else {
///     println!("Success!");
/// }
/// ```
pub fn should_colorize() -> bool {
    // Check NO_COLOR env var first (https://no-color.org/)
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }

    // Fall back to colored crate's logic
    colored::control::SHOULD_COLORIZE.should_colorize()
}

/// Apply color conditionally based on `should_colorize()`.
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::color_if;
/// # use colored::Colorize;
/// let msg = color_if("Success!", |s: colored::ColoredString| s.green());
/// println!("{}", msg);
/// ```
pub fn color_if<F, S>(text: S, f: F) -> String
where
    F: FnOnce(colored::ColoredString) -> colored::ColoredString,
    S: Into<colored::ColoredString>,
{
    if should_colorize() {
        f(text.into()).to_string()
    } else {
        text.into().clear().to_string()
    }
}

// ─────────────────────────────────────────────────────────────
// Stderr Variants (for machine-readable output commands)
// ─────────────────────────────────────────────────────────────

/// Print a command header to stderr in the standard boxed format.
///
/// Use this for commands that output structured data to stdout
/// (e.g., JSON, SARIF) but still want to show progress to users.
///
/// ```text
/// ┌─ evnx scan ─────────────────────────────────────┐
/// │ Checking for exposed secrets                   │
/// └────────────────────────────────────────────────┘
/// ```
///
/// # Arguments
///
/// * `title` - The command name (e.g., "evnx scan")
/// * `subtitle` - Optional description shown on second line
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::print_header_stderr;
/// print_header_stderr("evnx scan", Some("Checking for exposed secrets"));
/// ```
pub fn print_header_stderr(title: &str, subtitle: Option<&str>) {
    // The user typed the command; a three-line box repeating it back says
    // nothing. One dim line gives the same context at a fifth of the height.
    // ⚠️ The full `evnx scan`, not `scan`. In a CI log among other tools' output
    // the bare verb is ambiguous, and the five characters cost nothing.
    let line = match subtitle {
        Some(sub) => format!("\n  {}  {}\n", title.bold(), sub.dimmed()),
        None => format!("\n  {}\n", title.bold()),
    };
    eprintln!("{}", line);
}

/// Print verbose progress message to stderr.
///
/// Useful for `--verbose` mode when stdout is reserved for data output.
///
/// # Arguments
///
/// * `message` - Status message to display
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::verbose_stderr;
/// verbose_stderr("Scanning 42 files...");
/// ```
pub fn verbose_stderr(message: impl AsRef<str>) {
    eprintln!("{} {}", "⋯".dimmed(), message.as_ref().dimmed());
}

/// Print file scanning progress to stderr.
///
/// # Arguments
///
/// * `file` - Path being scanned
///
/// # Example
///
/// ```no_run
/// # use evnx::utils::ui::scanning_file_stderr;
/// # use std::path::Path;
/// scanning_file_stderr(Path::new(".env"));
/// ```
pub fn scanning_file_stderr(file: &std::path::Path) {
    eprintln!(
        "{} {}",
        "⋯".dimmed(),
        format!("Scanning: {}", file.display()).dimmed()
    );
}

/// Print `message` followed by ` [Y/n] `, read a line from stdin, and return
/// `true` if the user typed `y`, `Y`, or pressed Enter (empty = yes).
/// Returns `false` for `n`, `N`, or any other input.
pub fn prompt_yes_no(message: impl std::fmt::Display) -> bool {
    use std::io::{self, Write};
    eprint!("{message} [Y/n] ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "" | "y" | "yes")
}

pub fn print_key_value(items: &[(&str, &str)]) {
    let label_width = items.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in items {
        println!("  {:<width$}: {}", key, value, width = label_width);
    }
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_bar_length() {
        let pb = progress_bar(100, "Testing");
        assert_eq!(pb.length(), Some(100));
    }

    #[test]
    fn test_progress_bar_zero() {
        // Edge case — a bar with zero steps should not panic.
        let pb = progress_bar(0, "Empty");
        assert_eq!(pb.length(), Some(0));
    }

    #[test]
    fn test_calculate_box_width_defaults() {
        // Short content should use default width
        assert_eq!(calculate_box_width("init", None), BOX_WIDTH);
        assert_eq!(calculate_box_width("init", Some("short")), BOX_WIDTH);
    }

    #[test]
    fn test_calculate_box_width_expands() {
        // Long content should expand width (within limits)
        let long_title = "evnx migrate-from-aws-secrets-manager";
        let width = calculate_box_width(long_title, None);
        assert!(width > BOX_WIDTH);
        assert!(width <= MAX_BOX_WIDTH);
    }

    #[test]
    fn test_pad_or_truncate() {
        assert_eq!(pad_or_truncate("short", 20), "short               ");
        assert_eq!(
            pad_or_truncate("this is a very long string that exceeds width", 20),
            "this is a very lo..."
        );
    }

    #[test]
    fn test_should_colorize_respects_no_color() {
        // Save original
        let original = std::env::var_os("NO_COLOR");

        // Test with NO_COLOR set
        std::env::set_var("NO_COLOR", "1");
        assert!(!should_colorize());

        // Restore
        match original {
            Some(v) => std::env::set_var("NO_COLOR", v),
            None => std::env::remove_var("NO_COLOR"),
        }
    }
}

/// Announce the `.evnx.toml` a run is operating under.
///
/// ⚠️ Why this is not optional. `[scan] severity` and `[scan] exclude` can
/// *weaken* what the scanner reports, and the file is committed — so one commit
/// can quietly narrow scanning for everyone who clones the repository, with
/// nothing on the command line to show it. That is the same fail-open shape this
/// CLI has spent several releases removing.
///
/// Configurability is not the problem; silence is. A team genuinely needs to
/// exclude its fixtures. So the file is allowed to change the default and
/// **obliged to say so**.
///
/// Always stderr, never stdout, so `evnx convert --to json > out.json` stays
/// machine-readable. Suppressed by `--quiet`.
pub fn config_banner(path: &std::path::Path, overrides: &[String], quiet: bool) {
    if quiet {
        return;
    }
    let detail = if overrides.is_empty() {
        String::new()
    } else {
        format!(" ({})", overrides.join(", "))
    };
    eprintln!(
        "{}",
        format!("  config    {}{}", short_path(path), detail).dimmed()
    );
}

/// A committed config's path, shortened for a line printed on every run.
///
/// The loader resolves an absolute path, which is unambiguous but too long to
/// put in front of every command. Relative to the working directory it reads as
/// `.evnx.toml` at the project root and `../../.evnx.toml` from a package — and
/// the second of those says something worth knowing, that the policy came from
/// above rather than from here.
///
/// Falls back to the absolute path when the two share no prefix.
fn short_path(path: &std::path::Path) -> String {
    // `current_dir` is already absolute and already symlink-resolved by `getcwd`,
    // and the path being shortened came from a walk rooted at it — so the two
    // share a prefix without any further resolution. Canonicalizing again would
    // only reintroduce the macOS `/var` vs `/private/var` mismatch.
    let Ok(cwd) = std::env::current_dir() else {
        return path.display().to_string();
    };

    if let Ok(rest) = path.strip_prefix(&cwd) {
        return rest.display().to_string();
    }

    // Above the working directory: count the levels up to the shared ancestor.
    let mut ancestor = cwd.as_path();
    let mut ups = 0;
    while let Some(parent) = ancestor.parent() {
        ancestor = parent;
        ups += 1;
        if let Ok(rest) = path.strip_prefix(ancestor) {
            let mut out = String::new();
            for _ in 0..ups {
                out.push_str("../");
            }
            out.push_str(&rest.display().to_string());
            return out;
        }
    }
    path.display().to_string()
}

/// Report a key the config file carries that this version does not understand.
///
/// A warning rather than an error: the file is committed and teams run mixed
/// versions, so a config written for a later evnx must not break an earlier one.
/// Naming the key is what keeps a typo from being silent.
pub fn config_warning(message: &str, quiet: bool) {
    if quiet {
        return;
    }
    eprintln!("{} {}", glyph::WARN.yellow(), message.dimmed());
}
