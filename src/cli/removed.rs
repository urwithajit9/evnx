//! Spellings that were removed, and the error that names their replacement.
//!
//! A removed flag is not kept as a hidden alias: typing it stops the run with
//! exit 2 and a line saying what to type instead. [`check`] runs in `main`
//! before clap parses, so nothing is written and no banner is printed.

use crate::cli::Cli;
use clap::{Arg, Command, CommandFactory};
use std::ffi::OsString;

/// One removed spelling.
#[derive(Debug, Clone, Copy)]
pub enum Removed {
    /// A flag, in every spelling it had: `["-f", "--force"]`.
    Flag {
        /// The command it belonged to, e.g. `["cloud", "push"]`.
        path: &'static [&'static str],
        old: &'static [&'static str],
        /// What to type now.
        new: &'static str,
    },
    /// A value of a flag that is still accepted, e.g. `--format text`.
    Value {
        path: &'static [&'static str],
        flag: &'static str,
        value: &'static str,
        new: &'static str,
    },
}

impl Removed {
    pub fn path(&self) -> &'static [&'static str] {
        match self {
            Removed::Flag { path, .. } | Removed::Value { path, .. } => path,
        }
    }

    pub fn new_spelling(&self) -> &'static str {
        match self {
            Removed::Flag { new, .. } | Removed::Value { new, .. } => new,
        }
    }

    /// The old spelling as shown to a person: `-f, --force` or `--format text`.
    pub fn old_display(&self) -> String {
        match self {
            Removed::Flag { old, .. } => old.join(", "),
            Removed::Value { flag, value, .. } => format!("{flag} {value}"),
        }
    }
}

/// Every removed spelling. Each change that removes one adds a row here.
pub const REMOVED: &[Removed] = &[];

/// The message for a match: the spelling as typed.
pub fn message(path: &[&str], typed: &str, new: &str) -> String {
    format!(
        "error: '{typed}' was removed from 'evnx {}' in 0.14.2; use '{new}'",
        path.join(" ")
    )
}

/// `Some(message)` when `argv` (program name first) uses a removed spelling.
pub fn check(argv: &[OsString]) -> Option<String> {
    check_with(REMOVED, argv)
}

fn takes_value(arg: &Arg) -> bool {
    !matches!(
        arg.get_action(),
        clap::ArgAction::SetTrue
            | clap::ArgAction::SetFalse
            | clap::ArgAction::Count
            | clap::ArgAction::Help
            | clap::ArgAction::Version
    )
}

fn find_sub<'a>(cmd: &'a Command, name: &str) -> Option<&'a Command> {
    cmd.get_subcommands()
        .find(|s| s.get_name() == name || s.get_all_aliases().any(|a| a == name))
}

fn find_long<'a>(cmd: &'a Command, long: &str) -> Option<&'a Arg> {
    cmd.get_arguments().find(|a| a.get_long() == Some(long))
}

fn find_short(cmd: &Command, short: char) -> Option<&Arg> {
    cmd.get_arguments().find(|a| a.get_short() == Some(short))
}

/// The canonical command path the arguments name, and that command.
fn resolve<'a>(root: &'a Command, tokens: &[&str]) -> (Vec<String>, &'a Command) {
    let mut cmd = root;
    let mut path = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        if t == "--" {
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            // `--opt value` skips the value, unless it was given as `--opt=value`.
            if !long.contains('=')
                && find_long(cmd, long)
                    .or_else(|| find_long(root, long))
                    .is_some_and(takes_value)
            {
                i += 1;
            }
        } else if t.starts_with('-') && t.len() > 1 {
            // A value-taking short ends the cluster; last, it consumes the next token.
            let cluster: Vec<char> = t[1..].chars().collect();
            for (n, c) in cluster.iter().enumerate() {
                let arg = find_short(cmd, *c).or_else(|| find_short(root, *c));
                if arg.is_some_and(takes_value) {
                    if n + 1 == cluster.len() {
                        i += 1;
                    }
                    break;
                }
            }
        } else if let Some(sub) = find_sub(cmd, t) {
            path.push(sub.get_name().to_string());
            cmd = sub;
        } else {
            // A positional: whatever follows belongs to this command.
            break;
        }
        i += 1;
    }
    (path, cmd)
}

fn check_with(table: &[Removed], argv: &[OsString]) -> Option<String> {
    if table.is_empty() {
        return None;
    }
    let tokens: Vec<String> = argv
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let mut root = Cli::command();
    root.build();
    let (path, cmd) = resolve(&root, &refs);
    let entries: Vec<&Removed> = table
        .iter()
        .filter(|r| r.path().iter().copied().eq(path.iter().map(String::as_str)))
        .collect();
    if entries.is_empty() {
        return None;
    }
    let path_ref: Vec<&str> = path.iter().map(String::as_str).collect();

    let mut i = 0;
    while i < refs.len() {
        let t = refs[i];
        if t == "--" {
            break;
        }
        let next = refs.get(i + 1).copied();
        for r in &entries {
            match r {
                Removed::Flag { old, new, .. } => {
                    for o in *old {
                        let hit = if o.starts_with("--") {
                            t == *o || t.strip_prefix(*o).is_some_and(|r| r.starts_with('='))
                        } else {
                            let c = o.chars().nth(1).unwrap_or('\0');
                            t == *o || short_in_cluster(cmd, t, c)
                        };
                        if hit {
                            return Some(message(&path_ref, o, new));
                        }
                    }
                }
                Removed::Value {
                    flag, value, new, ..
                } => {
                    let eq = format!("{flag}={value}");
                    if t == eq || (t == *flag && next == Some(*value)) {
                        return Some(message(&path_ref, &format!("{flag} {value}"), new));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Whether `t` is a cluster of shorts (`-yf`) containing `c`. Stops at a short
/// that takes a value, whose remainder is that value.
fn short_in_cluster(cmd: &Command, t: &str, c: char) -> bool {
    if t.starts_with("--") || !t.starts_with('-') || t.len() <= 2 {
        return false;
    }
    for x in t[1..].chars() {
        if x == c {
            return true;
        }
        if find_short(cmd, x).is_some_and(takes_value) {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &[Removed] = &[
        Removed::Flag {
            path: &["remove"],
            old: &["-p", "--path"],
            new: "-C, --dir",
        },
        Removed::Flag {
            path: &["init"],
            old: &["-f", "--force"],
            new: "--yes",
        },
        Removed::Value {
            path: &["diff"],
            flag: "--format",
            value: "text",
            new: "--format pretty",
        },
    ];

    fn run(args: &[&str]) -> Option<String> {
        let argv: Vec<OsString> = std::iter::once("evnx")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect();
        check_with(TABLE, &argv)
    }

    #[test]
    fn long_flag_matches_and_names_the_replacement() {
        assert_eq!(
            run(&["remove", "--path", "x"]).as_deref(),
            Some("error: '--path' was removed from 'evnx remove' in 0.14.2; use '-C, --dir'")
        );
    }

    #[test]
    fn alias_resolves_to_the_command() {
        assert!(run(&["rm", "-p", "x", "KEY"])
            .unwrap()
            .contains("'evnx remove'"));
    }

    #[test]
    fn equals_form_matches() {
        assert!(run(&["remove", "--path=x"]).is_some());
        assert!(run(&["diff", "--format=text"]).is_some());
    }

    #[test]
    fn short_in_a_cluster_matches() {
        let m = run(&["init", "-qf"]).unwrap();
        assert!(m.contains("'-f'"), "{m}");
        assert!(run(&["init", "-fq"]).is_some());
    }

    #[test]
    fn double_dash_stops_scanning() {
        assert_eq!(run(&["remove", "--", "--path"]), None);
    }

    #[test]
    fn other_commands_are_untouched() {
        assert_eq!(run(&["diff", "--path", "x"]), None);
        assert_eq!(run(&["scan", "-f"]), None);
        assert_eq!(run(&[]), None);
    }

    #[test]
    fn global_options_before_the_command_are_skipped() {
        assert!(run(&["-q", "remove", "--path", "x"]).is_some());
        assert!(run(&["--no-color", "rm", "-p", "x"]).is_some());
    }

    #[test]
    fn a_dashed_option_value_still_matches() {
        // Acceptable: the value of another option that looks like a removed flag.
        assert!(run(&["remove", "--env-name", "--path"]).is_some());
    }

    #[test]
    fn value_entries_match_both_forms() {
        assert_eq!(
            run(&["diff", "--format", "text"]).as_deref(),
            Some("error: '--format text' was removed from 'evnx diff' in 0.14.2; use '--format pretty'")
        );
        assert_eq!(run(&["diff", "--format", "json"]), None);
    }
}
