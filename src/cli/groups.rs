//! The four groups every command list is printed in, and what each command
//! needs from the network (WP-10).
//!
//! One table, [`GROUPS`], feeds three places: the command list in `evnx
//! --help`, the plain output of `evnx commands`, and the `group` and `network`
//! fields of `evnx commands --json`. A command named here but absent from a
//! build (`backup` without the `backup` feature) is skipped; a command the
//! table forgets is a failing test, not a silent gap.

use clap::Command;

/// What a command needs from the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// Reads and writes local files only.
    Local,
    /// Local, and some modes reach a service (GitHub, Vercel, the release
    /// list) when asked to.
    LocalWithNetwork,
    /// Talks to an evnx account: needs sign-in and a connection.
    Account,
}

impl Network {
    /// The value in `evnx commands --json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Network::Local => "local",
            Network::LocalWithNetwork => "local-with-network",
            Network::Account => "account",
        }
    }
}

/// One group of the command list.
pub struct Group {
    /// The heading.
    pub title: &'static str,
    /// Its commands, in the order they are listed.
    pub commands: &'static [(&'static str, Network)],
}

use Network::{Account, Local, LocalWithNetwork};

/// Every top-level command, grouped.
pub const GROUPS: &[Group] = &[
    Group {
        title: "Local files",
        commands: &[
            ("init", Local),
            ("add", Local),
            ("set", Local),
            ("get", Local),
            ("remove", Local),
            ("validate", Local),
            ("sync", Local),
            ("align", Local),
            ("diff", Local),
            ("scan", Local),
            ("doctor", Local),
            ("config", Local),
        ],
    },
    Group {
        title: "Convert and run",
        commands: &[
            ("convert", Local),
            ("template", Local),
            ("run", Local),
            ("migrate", LocalWithNetwork),
            ("parse", Local),
            ("spec", Local),
            ("backup", Local),
            ("restore", Local),
            ("undo", Local),
            ("history", Local),
        ],
    },
    Group {
        title: "Cloud",
        commands: &[
            ("auth", Account),
            ("org", Account),
            ("vault", Account),
            ("cloud", Account),
            ("platform", LocalWithNetwork),
        ],
    },
    Group {
        title: "This tool",
        commands: &[
            ("update", LocalWithNetwork),
            ("completions", Local),
            ("commands", Local),
        ],
    },
];

/// The three lines that follow the groups in `evnx --help`.
pub const START_HERE: &str = "\
Start here:
  New project       evnx init
  Existing project  evnx validate, then evnx doctor
  Any command       evnx <command> --help";

/// The group and network category of a top-level command, by name or alias.
pub fn lookup(name: &str) -> Option<(&'static str, Network)> {
    GROUPS.iter().find_map(|g| {
        g.commands
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, net)| (g.title, *net))
    })
}

/// The command list of `evnx --help` and `evnx commands`: the groups, each
/// command with its one-line description, for the commands `cmd` has.
///
/// A visible top-level command the table does not name is listed last, under
/// "Other", so it is never missing from `--help`.
pub fn render(cmd: &Command, indent: &str) -> String {
    use std::fmt::Write;
    let describe = |c: &Command| -> String {
        let about = c.get_about().map(|a| a.to_string()).unwrap_or_default();
        let about = about.lines().next().unwrap_or("").trim_end_matches('.');
        let aliases: Vec<&str> = c.get_visible_aliases().collect();
        if aliases.is_empty() {
            about.to_string()
        } else {
            format!("{about} [aliases: {}]", aliases.join(", "))
        }
    };
    let visible: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .collect();
    let width = visible
        .iter()
        .map(|c| c.get_name().len())
        .max()
        .unwrap_or(8);

    let mut out = String::new();
    let mut listed: Vec<&str> = Vec::new();
    let section = |out: &mut String, title: &str, rows: Vec<&Command>| {
        if rows.is_empty() {
            return;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = writeln!(out, "{indent}{title}:");
        for c in rows {
            let _ = writeln!(
                out,
                "{indent}  {:width$}  {}",
                c.get_name(),
                describe(c),
                width = width
            );
        }
    };
    for g in GROUPS {
        let rows: Vec<&Command> = g
            .commands
            .iter()
            .filter_map(|(n, _)| visible.iter().find(|c| c.get_name() == *n).copied())
            .collect();
        listed.extend(rows.iter().map(|c| c.get_name()));
        section(&mut out, g.title, rows);
    }
    let rest: Vec<&Command> = visible
        .iter()
        .filter(|c| !listed.contains(&c.get_name()))
        .copied()
        .collect();
    section(&mut out, "Other", rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_in_one_group_only() {
        let mut seen = std::collections::BTreeSet::new();
        for g in GROUPS {
            for (name, _) in g.commands {
                assert!(seen.insert(*name), "{name} is listed twice");
            }
        }
    }

    #[test]
    fn every_command_the_table_names_exists_and_none_is_missing() {
        let cmd = crate::cli::exit_help::command();
        let have: Vec<&str> = cmd
            .get_subcommands()
            .filter(|c| !c.is_hide_set() && c.get_name() != "help")
            .map(|c| c.get_name())
            .collect();
        for name in &have {
            assert!(lookup(name).is_some(), "{name} is in no group");
        }
        // A command the build has no feature for is skipped, so the other way
        // round holds only for what this build has: nothing in the table may
        // be spelled differently from the command.
        for g in GROUPS {
            for (name, _) in g.commands {
                let optional = [
                    "backup", "restore", "migrate", "auth", "org", "vault", "cloud", "platform",
                ];
                assert!(
                    have.contains(name) || optional.contains(name),
                    "{name} names no command"
                );
            }
        }
    }
}
