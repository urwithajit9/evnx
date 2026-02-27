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
    let width = calculate_box_width(title, subtitle);
    let border = "─".repeat(width - 4);

    // Top border with title
    println!(
        "\n{}",
        format!("┌─ {} {}┐", title, "─".repeat(width - 4 - title.len())).cyan()
    );

    // Subtitle line (if provided)
    if let Some(sub) = subtitle {
        let padded = pad_or_truncate(sub, width - 4);
        println!("{}", format!("│ {} │", padded).cyan());
    }

    // Bottom border
    println!("{}\n", format!("└─{}─┘", border).cyan());
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

    println!(
        "\n{}",
        format!("┌─ {} {}┐", title, "─".repeat(width - 4 - title.len())).cyan()
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
    println!("\n{} {}:", icon.bold(), title.bold());
}

/// Convenience wrapper for preview sections.
pub fn print_preview_header() {
    print_section_header("📋", "Preview");
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

/// Print a green success message with checkmark.
///
/// ```text
/// ✓ Created .env.example with 15 variables
/// ```
pub fn success(message: &str) {
    println!("{} {}", "✓".green(), message);
}

/// Print a red error message with cross.
///
/// ```text
/// ✗ Failed to parse schema.json
/// ```
pub fn error(message: &str) {
    println!("{} {}", "✗".red(), message.red());
}

/// Print a yellow warning message with alert icon.
///
/// ```text
/// ⚠️  Conflicting variables will be skipped
/// ```
pub fn warning(message: &str) {
    println!("{} {}", "⚠️".yellow(), message.yellow());
}

/// Print a cyan info message with info icon.
///
/// ```text
/// ℹ️  Tip: Run 'evnx validate' to check configuration
/// ```
pub fn info(message: &str) {
    println!("{} {}", "ℹ️".cyan(), message.dimmed());
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
