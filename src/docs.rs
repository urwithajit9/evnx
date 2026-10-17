// src/docs.rs
//
// The documentation URLs this binary prints — in `--help` via cli.rs
// `after_help`, and in command output via `ui::print_docs_hint`.
//
// ⚠️ This file is NOT a single source of truth, and the header here used to say
// it was: "change it here — it propagates automatically". Nothing propagated.
// `after_help` feeds Clap's derive attribute, so it has to be a `&'static str`
// literal with no allocation, which means every URL below spells out the host
// in full. When the guides moved to docs.evnx.dev, all 33 of them kept pointing
// at the old one — and kept *working*, because a 301 is not a broken link, so
// nothing failed and nothing warned. They shipped that way in two releases.
//
// BASE_URL is the host and prefix every URL below must start with, and the
// tests at the bottom are what actually hold them to it.

pub const BASE_URL: &str = "https://docs.evnx.dev/cli";

/// A documentation entry for one evnx command.
///
/// All fields are `&'static str` so they can be used directly
/// in Clap's `#[command(after_help = …)]` derive attribute
/// without allocations or LazyLock.
pub struct CommandDoc {
    /// The subcommand name, e.g. "init"
    pub command: &'static str,
    /// Full URL to the command's guide page
    pub url: &'static str,
    /// One-liner shown alongside the URL in terminal output
    pub description: &'static str,
    /// Pre-formatted string for Clap's after_help (static, no allocation)
    pub after_help: &'static str,
}

impl CommandDoc {
    /// Compact one-line hint for end of command output.
    ///
    /// Output: `  📖 Docs: https://docs.evnx.dev/cli/commands/init`
    pub fn hint_line(&self) -> String {
        format!("  Docs: {}", self.url)
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────
//
// One entry per subcommand. `after_help` is a plain string literal so it can
// be used directly in #[command(after_help = docs::INIT.after_help)].

pub const INIT: CommandDoc = CommandDoc {
    command: "init",
    url: "https://docs.evnx.dev/cli/commands/init",
    description: "Project setup, stacks, and service presets",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/init",
};

pub const SPEC: CommandDoc = CommandDoc {
    command: "spec",
    url: "https://docs.evnx.dev/cli/commands/spec",
    description: "Declare what each variable is: required, format, secret",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/spec",
};

pub const ADD: CommandDoc = CommandDoc {
    command: "add",
    url: "https://docs.evnx.dev/cli/commands/add",
    description: "Adding variables interactively or from blueprints",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/add",
};

pub const VALIDATE: CommandDoc = CommandDoc {
    command: "validate",
    url: "https://docs.evnx.dev/cli/commands/validate",
    description: "Validation rules, CI flags, and strict mode",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/validate",
};

pub const SCAN: CommandDoc = CommandDoc {
    command: "scan",
    url: "https://docs.evnx.dev/cli/commands/scan",
    description: "Secret detection patterns and entropy analysis",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/scan",
};

pub const DIFF: CommandDoc = CommandDoc {
    command: "diff",
    url: "https://docs.evnx.dev/cli/commands/diff",
    description: "Comparing .env vs .env.example",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/diff",
};

pub const CONVERT: CommandDoc = CommandDoc {
    command: "convert",
    url: "https://docs.evnx.dev/cli/commands/convert",
    description: "All 14 output formats and filtering options",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/convert",
};

pub const SYNC: CommandDoc = CommandDoc {
    command: "sync",
    url: "https://docs.evnx.dev/cli/commands/sync",
    description: "Keeping .env and .env.example in sync",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/sync",
};

pub const MIGRATE: CommandDoc = CommandDoc {
    command: "migrate",
    url: "https://docs.evnx.dev/cli/commands/migrate",
    description: "Migrating secrets to cloud managers",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/migrate",
};

pub const DOCTOR: CommandDoc = CommandDoc {
    command: "doctor",
    url: "https://docs.evnx.dev/cli/commands/doctor",
    description: "Diagnosing setup and gitignore issues",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/doctor",
};

pub const TEMPLATE: CommandDoc = CommandDoc {
    command: "template",
    url: "https://docs.evnx.dev/cli/commands/template",
    description: "Generating config files from templates",
    after_help: "\
References:\n\
\x20 {{VAR}}  ${VAR}  $VAR        replaced by the value from the env file\n\
\n\
Filters, one per placeholder: {{VAR|filter}}\n\
\x20 upper  lower                 case\n\
\x20 title                        Title Case For Each Word\n\
\x20 capitalize                   First character only\n\
\x20 bool                         true/yes/on/1/y or false/no/off/0/n; anything else is an error\n\
\x20 int                          a whole number; anything else is an error\n\
\x20 json                         a quoted, escaped JSON string\n\
\x20 default:VALUE                VALUE when VAR is empty or not defined\n\
An unknown filter, or two chained, is an error and nothing is written.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/template",
};

pub const BACKUP: CommandDoc = CommandDoc {
    command: "backup",
    url: "https://docs.evnx.dev/cli/commands/backup",
    description: "AES-256-GCM encrypted backups",
    after_help: "\
Exit codes:\n\
  0  Success\n\
  1  Generic error (IO, unexpected failure)\n\
  2  Source file not found or not a regular file\n\
  3  Password confirmation did not match\n\
  4  Encryption failed\n\
  5  Failed to write backup file\n\
  6  Post-write integrity check failed (--verify)\n\
\n\
📖  Full guide: https://docs.evnx.dev/cli/commands/backup",
};

pub const RESTORE: CommandDoc = CommandDoc {
    command: "restore",
    url: "https://docs.evnx.dev/cli/commands/restore",
    description: "Restoring from encrypted backups",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/restore",
};

pub const CLOUD: CommandDoc = CommandDoc {
    command: "cloud",
    url: "https://docs.evnx.dev/cli/commands/cloud",
    description: "Zero-knowledge encrypted .env sync",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/cloud",
};

pub const AUTH: CommandDoc = CommandDoc {
    command: "auth",
    url: "https://docs.evnx.dev/cli/commands/auth",
    description: "Account registration and sign-in",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/auth",
};

pub const VAULT: CommandDoc = CommandDoc {
    command: "vault",
    url: "https://docs.evnx.dev/cli/commands/vault",
    description: "Creating and managing encrypted vaults",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/vault",
};

pub const ORG: CommandDoc = CommandDoc {
    command: "org",
    url: "https://docs.evnx.dev/cli/commands/org",
    description: "Organisations: a plan, a directory, and seats",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/org",
};

pub const COMMANDS: CommandDoc = CommandDoc {
    command: "commands",
    url: "https://docs.evnx.dev/cli/commands/commands",
    description: "What this build can do, and which features it has",
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/commands",
};

pub const COMPLETIONS: CommandDoc = CommandDoc {
    command: "completions",
    url: "https://docs.evnx.dev/cli/commands/completions",
    description: "Shell completion scripts, and where each shell reads them",
    after_help: "\
Install, for the shell you use (create the folder first if it does not exist),\n\
then open a new shell:\n\
\x20 bash        evnx completions bash > ~/.local/share/bash-completion/completions/evnx\n\
\x20 zsh         evnx completions zsh > ~/.zfunc/_evnx\n\
\x20             and in ~/.zshrc, before compinit: fpath=(~/.zfunc $fpath)\n\
\x20 fish        evnx completions fish > ~/.config/fish/completions/evnx.fish\n\
\x20 powershell  evnx completions powershell >> $PROFILE\n\
\x20 elvish      evnx completions elvish >> ~/.config/elvish/rc.elv\n\
\n\
The script lists this build's commands: generate it again after an upgrade.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/completions",
};

pub const UPDATE: CommandDoc = CommandDoc {
    command: "update",
    url: "https://docs.evnx.dev/cli/commands/update",
    description: "How to upgrade, and whether a newer release exists",
    after_help: "\
Exit codes:\n\
\x20 0  without --check, always; with it, this is the latest release or newer\n\
\x20 1  --check: a newer release exists\n\
\x20 2  --check could not tell: no network support, no answer, or rate-limited\n\
\n\
-q prints only the upgrade command; with --check, the status line and the command.\n\
\n\
Full guide: https://docs.evnx.dev/cli/commands/update",
};

/// Every `CommandDoc` in this file.
///
/// Hand-maintained, which is a risk — so
/// `the_list_above_covers_every_command_doc_in_this_file` counts the
/// declarations in this file's own source and fails if the two disagree.
///
/// ⚠️ Not test-only. `evnx surface` reads it so the documentation URL the tour
/// links to is the **same string** the binary prints in `--help`. Two lists
/// would be two things to keep in step, which is the failure this whole area
/// exists to stop.
pub const ALL: &[&CommandDoc] = &[
    &INIT,
    &SPEC,
    &ADD,
    &VALIDATE,
    &SCAN,
    &DIFF,
    &CONVERT,
    &SYNC,
    &MIGRATE,
    &DOCTOR,
    &TEMPLATE,
    &BACKUP,
    &RESTORE,
    &CLOUD,
    &AUTH,
    &VAULT,
    &ORG,
    &COMMANDS,
    &COMPLETIONS,
    &UPDATE,
];

/// The documentation URL for a top-level command, if it has one.
///
/// ⚠️ `None` is a real answer, not a gap to paper over: a command that ships
/// before its guide has no `CommandDoc` yet. A caller that invented a URL from
/// the command name would produce a 404 for exactly that command. Today every
/// top-level command has an entry, and `tests/surface_integration.rs` checks
/// it (I-195).
pub fn url_for(command: &str) -> Option<&'static str> {
    ALL.iter().find(|d| d.command == command).map(|d| d.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ The test P6 needed and did not have.
    ///
    /// A stale URL that 301s is the one kind of wrong nothing notices: it
    /// resolves, the reader lands on the right page, and the extra hop is
    /// invisible unless something is looking for it. Nothing was.
    #[test]
    fn every_url_points_at_the_docs_host() {
        for d in ALL {
            assert!(
                d.url.starts_with(BASE_URL),
                "{}: url is {}, which does not start with {BASE_URL}",
                d.command,
                d.url
            );
        }
    }

    /// `after_help` is what a reader sees in `--help`; `url` is what the hint
    /// line prints. A copy-paste leaving one on another command's guide sends
    /// people somewhere plausible and wrong, which is worse than a 404.
    #[test]
    fn after_help_links_to_the_same_page_as_url() {
        for d in ALL {
            assert!(
                d.after_help.contains(d.url),
                "{}: after_help does not contain its own url {}",
                d.command,
                d.url
            );
        }
    }

    /// ⚠️ A trailing slash is a 308 here, and the instinct to add one comes
    /// from `app.evnx.dev`, where it is the opposite: that host is a static
    /// export and 308s `/login` *to* `/login/`. Verified against the live
    /// host — `/cli/commands/init/` redirects back to `/cli/commands/init`.
    #[test]
    fn no_url_ends_in_a_slash() {
        for d in ALL {
            assert!(
                !d.url.ends_with('/'),
                "{}: {} ends in a slash, which docs.evnx.dev 308s away",
                d.command,
                d.url
            );
        }
    }

    /// The pre-split path must not come back in any printed field.
    #[test]
    fn the_old_guides_path_is_gone() {
        for d in ALL {
            for (field, text) in [
                ("url", d.url),
                ("after_help", d.after_help),
                ("description", d.description),
            ] {
                assert!(
                    !text.contains("/guides"),
                    "{}: {field} still references the pre-split /guides path",
                    d.command
                );
            }
        }
    }

    /// Reads this file's own source so a command added above cannot sit
    /// outside every test here by being left out of `ALL`.
    #[test]
    fn the_list_above_covers_every_command_doc_in_this_file() {
        let declared = include_str!("docs.rs")
            .lines()
            .filter(|l| l.starts_with("pub const ") && l.contains(": CommandDoc"))
            .count();
        assert_eq!(
            declared,
            ALL.len(),
            "{declared} CommandDoc consts are declared in docs.rs but ALL lists {} \
             — add the new one to ALL",
            ALL.len()
        );
    }
}
