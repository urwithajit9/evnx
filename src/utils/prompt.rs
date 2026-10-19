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
    if !crate::utils::term::interactive() {
        return Err(NoTerminal {
            hint: hint.to_string(),
        }
        .into());
    }
    Ok(())
}

/// A choice that has a flag, asked for with no flag and no terminal to ask on.
///
/// ⚠️ Without this, dialoguer's `Select` fails with "IO error: not a terminal",
/// which `convert` wrapped into "Format selection cancelled by user: … not a
/// terminal: not a terminal" — nobody cancelled anything, and the flag that
/// answers the question went unnamed (I-083). `migrate --to` already said it
/// this way; one wording for both.
///
/// `example` is the command line with the flag, `available` what it accepts.
pub fn choice_needed(flag: &str, example: &str, available: &[&str]) -> anyhow::Error {
    anyhow::anyhow!(
        "{flag} is required when there is no terminal to ask.\n\n\
         \x20 Pass one explicitly:\n\
         \x20   {example}\n\n\
         \x20 Available: {}",
        available.join(", ")
    )
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

/// A yes/no question that is answered on Enter, not on the first keystroke,
/// and shows the key as it is pressed.
///
/// ⚠️ Two defects this replaces, one on each side of dialoguer's `Confirm`:
///
/// * Answered on the first key (its default), the Enter people type after `y`
///   answered the *next* question with its default without the user seeing
///   it: on 0.10.0, `y⏎` at "Generate .env files?" silently answered
///   "Overwrite it?" with No; on 0.10.1, `y⏎` at `add custom`'s "Is this
///   variable required?" answered "Add another variable?" (N-09). Twelve
///   questions still asked that way until 0.10.9 (I-265).
/// * Answered on Enter (`wait_for_newline`), it hid the cursor, never printed
///   the key, and only redrew the hint with the pending answer as its
///   default — `[Y/n]` ↔ `[y/N]`, and under a No default `n` changed nothing
///   — so a keypress looked ignored until Enter (I-264, owner report).
///
/// So: `y`/`n` (any case) is shown after the hint (`[y/N] n`), Backspace takes
/// it back, Enter answers; Enter alone takes the default. With
/// [`YesNo::interact_opt`], Esc (or `q`) then Enter cancels (`None`); the hint
/// is redrawn as `[y/n]` after Esc, as before. The line left behind is the
/// same as before: `Question? yes`.
pub fn yes_no() -> YesNo {
    YesNo {
        prompt: String::new(),
        default: None,
    }
}

/// The question built by [`yes_no`].
#[derive(Debug, Clone)]
pub struct YesNo {
    prompt: String,
    default: Option<bool>,
}

impl YesNo {
    /// The question.
    #[must_use]
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// The answer Enter gives when nothing was typed; shown as `[Y/n]` or
    /// `[y/N]`.
    #[must_use]
    pub fn default(mut self, value: bool) -> Self {
        self.default = Some(value);
        self
    }

    /// Ask until there is an answer. Esc does nothing here.
    ///
    /// # Errors
    ///
    /// No terminal on stderr, or the terminal failed — as dialoguer's
    /// `Error`, so a caller's message for that case is unchanged.
    pub fn interact(self) -> dialoguer::Result<bool> {
        // Without a cancel, Enter returns only once there is an answer.
        Ok(self.run(false)?.unwrap_or(false))
    }

    /// Ask; Esc (or `q`) then Enter cancels with `None`.
    ///
    /// # Errors
    ///
    /// As [`YesNo::interact`].
    pub fn interact_opt(self) -> dialoguer::Result<Option<bool>> {
        self.run(true)
    }

    fn run(self, allow_cancel: bool) -> dialoguer::Result<Option<bool>> {
        let term = dialoguer::console::Term::stderr();
        // ⚠️ dialoguer's own error, word for word: `restore`'s demo locks
        // "IO error: not a terminal: not a terminal" (I-148, its own patch).
        if !term.is_term() {
            return Err(dialoguer::Error::IO(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "not a terminal",
            )));
        }
        let mut pending = Pending::default();
        term.write_str(&self.line(&pending))?;
        term.flush()?;
        loop {
            match pending.key(term.read_key()?, self.default, allow_cancel) {
                Step::Ignore => {}
                Step::Redraw => {
                    term.clear_line()?;
                    term.write_str(&self.line(&pending))?;
                    term.flush()?;
                }
                Step::Done(answer) => {
                    term.clear_line()?;
                    term.write_line(&self.answered(answer))?;
                    term.flush()?;
                    return Ok(answer);
                }
            }
        }
    }

    /// `Question? [y/N] n`: the hint shows the default (none after Esc), then
    /// what was typed, where the cursor is.
    fn line(&self, pending: &Pending) -> String {
        let default = if pending.cancelled {
            None
        } else {
            self.default
        };
        let hint = match default {
            Some(true) => "[Y/n]",
            Some(false) => "[y/N]",
            None => "[y/n]",
        };
        let typed = pending.typed.map(String::from).unwrap_or_default();
        if self.prompt.is_empty() {
            format!("{hint} {typed}")
        } else {
            format!("{} {hint} {typed}", self.prompt)
        }
    }

    /// The line left once answered: `Question? yes`, or the question alone
    /// when cancelled.
    fn answered(&self, answer: Option<bool>) -> String {
        let word = match answer {
            Some(true) => "yes",
            Some(false) => "no",
            None => return self.prompt.clone(),
        };
        if self.prompt.is_empty() {
            word.to_string()
        } else {
            format!("{} {word}", self.prompt)
        }
    }
}

/// What has been typed at a [`YesNo`] so far.
#[derive(Debug, Default)]
struct Pending {
    /// `y`, `Y`, `n` or `N`, as pressed.
    typed: Option<char>,
    /// Esc was pressed (and nothing typed since).
    cancelled: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Ignore,
    Redraw,
    Done(Option<bool>),
}

impl Pending {
    fn key(&mut self, key: dialoguer::console::Key, default: Option<bool>, cancel: bool) -> Step {
        use dialoguer::console::Key;
        match key {
            Key::Char(c @ ('y' | 'Y' | 'n' | 'N')) => {
                self.typed = Some(c);
                self.cancelled = false;
                Step::Redraw
            }
            Key::Backspace | Key::Del if self.typed.is_some() => {
                self.typed = None;
                Step::Redraw
            }
            Key::Escape | Key::Char('q') if cancel => {
                self.typed = None;
                self.cancelled = true;
                Step::Redraw
            }
            Key::Enter => match self.typed {
                Some(c) => Step::Done(Some(c.eq_ignore_ascii_case(&'y'))),
                None if self.cancelled => Step::Done(None),
                None => match default {
                    Some(d) => Step::Done(Some(d)),
                    // No default and nothing typed: `interact_opt` cancels,
                    // as dialoguer's did; `interact` waits for an answer.
                    None if cancel => Step::Done(None),
                    None => Step::Ignore,
                },
            },
            _ => Step::Ignore,
        }
    }
}

/// A destructive action's yes/no, the same way in every command (A-39, DEC-AU7).
///
/// * `yes` (`--yes`/`-y`): no question, no stdin read — `Ok(true)`.
/// * No terminal: [`NoTerminal`] naming `--yes` and `example` — exit 2. Never a
///   silent "no" and never a silent "yes": `org` read raw stdin, so `echo y |`
///   confirmed and end of input read as "Cancelled" with exit 0, while other
///   commands failed with dialoguer's bare "not a terminal" (I-218).
/// * A terminal: `question`, answered on Enter ([`yes_no`]), default **No**; Esc
///   is No.
///
/// The caller prints what is about to happen first and asks a specific
/// question ("Revoke session 1a2b3c4d?"); this only decides how it is asked.
pub fn confirm(question: &str, yes: bool, example: &str) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    require_terminal_or(&format!(
        "Add --yes (-y) to go ahead without being asked:\n  {example}"
    ))?;
    Ok(yes_no()
        .with_prompt(question)
        .default(false)
        .interact_opt()?
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--yes` answers without touching stdin — whatever stdin is.
    #[test]
    fn t_i218_yes_never_asks() {
        assert!(confirm("Delete it?", true, "evnx x --yes").unwrap());
    }

    use dialoguer::console::Key;

    fn press(keys: &[Key], default: Option<bool>, cancel: bool) -> (Pending, Vec<Step>) {
        let mut p = Pending::default();
        let steps = keys
            .iter()
            .map(|k| p.key(k.clone(), default, cancel))
            .collect();
        (p, steps)
    }

    /// I-264: the key shows on the line, before Enter.
    #[test]
    fn t_i264_the_typed_key_is_on_the_line() {
        let q = yes_no().with_prompt("Generate?").default(true);
        let (p, steps) = press(&[Key::Char('n')], Some(true), true);
        assert_eq!(steps, [Step::Redraw]);
        assert_eq!(q.line(&p), "Generate? [Y/n] n");
        assert_eq!(q.line(&Pending::default()), "Generate? [Y/n] ");
    }

    #[test]
    fn t_i264_enter_answers_backspace_takes_back() {
        let enter = |keys: &[Key], default| press(keys, default, true).1.pop().unwrap();
        assert_eq!(
            enter(&[Key::Char('y'), Key::Enter], Some(false)),
            Step::Done(Some(true))
        );
        assert_eq!(
            enter(&[Key::Char('N'), Key::Enter], Some(true)),
            Step::Done(Some(false))
        );
        assert_eq!(enter(&[Key::Enter], Some(true)), Step::Done(Some(true)));
        assert_eq!(
            enter(&[Key::Char('n'), Key::Backspace, Key::Enter], Some(true)),
            Step::Done(Some(true)),
            "Backspace takes the answer back; Enter gives the default"
        );
        // Other keys do nothing, and change nothing.
        assert_eq!(
            enter(&[Key::Char('x'), Key::Enter], Some(false)),
            Step::Done(Some(false))
        );
    }

    /// Esc then Enter cancels, and the hint loses its default (`[y/n]`), which
    /// the pty tests wait for.
    #[test]
    fn t_i264_escape_cancels_only_where_allowed() {
        let q = yes_no().with_prompt("Overwrite it?").default(false);
        let (p, steps) = press(&[Key::Escape], Some(false), true);
        assert_eq!(steps, [Step::Redraw]);
        assert_eq!(q.line(&p), "Overwrite it? [y/n] ");
        assert_eq!(
            press(&[Key::Escape, Key::Enter], Some(false), true).1[1],
            Step::Done(None)
        );
        // `interact`: Esc is just another key; Enter takes the default.
        assert_eq!(
            press(&[Key::Escape, Key::Enter], Some(false), false).1,
            [Step::Ignore, Step::Done(Some(false))]
        );
        // No default, nothing typed: `interact` waits for an answer.
        assert_eq!(press(&[Key::Enter], None, false).1, [Step::Ignore]);
    }

    #[test]
    fn t_i264_the_line_left_behind_is_unchanged() {
        let q = yes_no().with_prompt("Overwrite it?");
        assert_eq!(q.answered(Some(true)), "Overwrite it? yes");
        assert_eq!(q.answered(Some(false)), "Overwrite it? no");
        assert_eq!(q.answered(None), "Overwrite it?");
    }

    /// Without a terminal it fails the way dialoguer did, so the callers'
    /// `require_terminal` guards and messages are unchanged.
    #[test]
    fn no_terminal_is_not_connected() {
        if dialoguer::console::Term::stderr().is_term() {
            return; // run by hand in a terminal
        }
        let dialoguer::Error::IO(err) = yes_no().with_prompt("x").interact_opt().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotConnected);
    }
}
