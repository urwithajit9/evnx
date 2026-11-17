//! The "Exit codes" block of every `--help`, rendered from
//! [`core::exit_table`](crate::core::exit_table) (WP-9a).
//!
//! clap's derive wants `after_help` as a literal, so the block is added to the
//! built [`Command`] tree instead: [`command`] is `Cli::command()` plus one
//! generated block per subcommand. `main`, `evnx commands`, completions and
//! the removed-spelling check all build the tree through it, so there is one
//! `--help` and one table.

use clap::{Command, CommandFactory};

use crate::cli::Cli;
use crate::core::exit_table::render_block;

/// The whole command tree, with every command's exit codes in its help.
pub fn command() -> Command {
    with_exit_blocks(Cli::command(), &mut Vec::new())
}

/// `cmd` and its subcommands with the block appended to each `after_help`.
pub fn with_exit_blocks(cmd: Command, path: &mut Vec<String>) -> Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    let mut cmd = cmd;
    for name in names {
        path.push(name.clone());
        cmd = cmd.mut_subcommand(&name, |s| with_exit_blocks(s, path));
        path.pop();
    }
    if path.is_empty() {
        return cmd;
    }
    let p: Vec<&str> = path.iter().map(String::as_str).collect();
    let existing = cmd
        .get_after_help()
        .map(|s| s.to_string())
        .unwrap_or_default();
    cmd.after_help(insert(&existing, &render_block(&p)))
}

/// Put `block` before the "Full guide:" line when there is one (the guide
/// link stays last), else after the existing text.
fn insert(existing: &str, block: &str) -> String {
    let existing = existing.trim_end_matches('\n');
    let guide = existing
        .rmatch_indices("Full guide:")
        .next()
        .map(|(i, _)| existing[..i].rfind('\n').map_or(0, |n| n + 1));
    match guide {
        Some(at) => {
            let (head, tail) = existing.split_at(at);
            let head = head.trim_end_matches('\n');
            if head.is_empty() {
                format!("{block}\n\n{tail}")
            } else {
                format!("{head}\n\n{block}\n\n{tail}")
            }
        }
        None if existing.is_empty() => block.to_string(),
        None => format!("{existing}\n\n{block}"),
    }
}
