//! Terminal UI utilities — progress bars, boxed output, and status messages.
//!
//! All functions in this module write directly to stdout. They are intentionally
//! thin wrappers around `colored` and `indicatif` so the rest of the codebase
//! does not need to import those crates directly.
//!
//! # Future work
//!
//! - Respect a global `--no-color` flag by checking `colored::control::SHOULD_COLORIZE`.
//! - Add a `spinner()` helper for indeterminate operations.
//! - Add a `table()` helper for structured multi-column output.

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

/// Create a styled progress bar with `len` steps and an initial `message`.
///
/// The bar uses the format:
/// ```text
/// ⠙ [████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 12/50 Processing KEY
/// ```
///
/// The caller is responsible for calling [`ProgressBar::inc`] and
/// [`ProgressBar::finish_with_message`] when done.
pub fn progress_bar(len: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message(message.to_string());
    pb
}

/// Print a cyan boxed header with an optional multi-line body.
///
/// ```text
/// ┌─ Title ──────────────────────────────────────────────┐
/// │ Optional message line                                │
/// └──────────────────────────────────────────────────────┘
/// ```
///
/// Pass an empty string for `message` to render a title-only box.
pub fn print_box(title: &str, message: &str) {
    let width = 60;
    let border = "─".repeat(width - 4);

    println!("\n{}", format!("┌─{}─┐", border).cyan());
    println!(
        "{}",
        format!("│ {:<width$} │", title, width = width - 4).cyan()
    );

    if !message.is_empty() {
        for line in message.lines() {
            println!(
                "{}",
                format!("│ {:<width$} │", line, width = width - 4).cyan()
            );
        }
    }

    println!("{}\n", format!("└─{}─┘", border).cyan());
}

/// Print a green `✓ <message>` success line.
pub fn success(message: &str) {
    println!("{} {}", "✓".green(), message);
}

/// Print a red `✗ <message>` error line.
pub fn error(message: &str) {
    println!("{} {}", "✗".red(), message);
}

/// Print a yellow `⚠️ <message>` warning line.
pub fn warning(message: &str) {
    println!("{} {}", "⚠️".yellow(), message);
}

/// Print a cyan `ℹ️ <message>` info line.
pub fn info(message: &str) {
    println!("{} {}", "ℹ️".cyan(), message);
}

/// Print a dimmed horizontal rule (60 dashes).
pub fn separator() {
    println!("{}", "─".repeat(60).dimmed());
}

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
}
