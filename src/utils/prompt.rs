//! Prompt rules shared by every interactive command.
//!
//! Moved here from `commands/init/shared.rs` in 0.10.2 so `add` follows the
//! same rules as `init` instead of a copy of them. `init` re-exports both.

use anyhow::Result;

/// Stop before a confirmation prompt that cannot be shown.
///
/// ⚠️ Without a terminal dialoguer fails with "IO error: not a terminal" (twice),
/// which says nothing about what to do. A script that forgot `--yes` got
/// exactly that (init N-08, add I-020). Exit code stays 2.
///
/// `example` is the command line that would work: the same one with `--yes`.
pub fn require_terminal(example: &str) -> Result<()> {
    require_terminal_or(&format!("Add --yes to write without asking:\n  {example}"))
}

/// [`require_terminal`] with a hint of the caller's own, for commands where
/// `--yes` is not the answer.
pub fn require_terminal_or(hint: &str) -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(NoTerminal {
            hint: hint.to_string(),
        }
        .into());
    }
    Ok(())
}

/// A prompt was needed and there was no terminal to show it on.
///
/// `main` reports it like any other error — exit 2, "No verdict" — and `hint`
/// names the flag that makes the question unnecessary. Typed so a caller can
/// tell it apart (`err.downcast_ref::<NoTerminal>()`) from a prompt that failed
/// for another reason.
#[derive(Debug, thiserror::Error)]
#[error("there is no terminal to confirm with.\n\n{hint}")]
pub struct NoTerminal {
    /// What to do instead, usually the same command with `--yes`.
    pub hint: String,
}

/// A yes/no question that is answered on Enter, not on the first keystroke.
///
/// ⚠️ dialoguer's default answers the moment `y` or `n` is pressed. People type
/// `y⏎`, and the Enter then answered the *next* question with its default
/// without the user seeing it: on 0.10.0, `y⏎` at "Generate .env files?"
/// silently answered "Overwrite it?" with No, so the stack the user asked for
/// was never written; on 0.10.1, `y⏎` at `add custom`'s "Is this variable
/// required?" silently answered "Add another variable?" (N-09). Every yes/no
/// prompt goes through this. `y`, `yes`, `n`, `no` then need Enter; Enter alone
/// takes the default.
///
/// Use it with `interact_opt()`: Esc, then Enter, cancels (`None`).
pub fn yes_no() -> dialoguer::Confirm<'static> {
    dialoguer::Confirm::new().wait_for_newline(true)
}
