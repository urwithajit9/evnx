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
    after_help: "Full guide: https://docs.evnx.dev/cli/commands/template",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `CommandDoc` in this file.
    ///
    /// Hand-maintained, which is a risk — so
    /// `the_list_above_covers_every_command_doc_in_this_file` counts the
    /// declarations in this file's own source and fails if the two disagree.
    const ALL: &[&CommandDoc] = &[
        &INIT, &SPEC, &ADD, &VALIDATE, &SCAN, &DIFF, &CONVERT, &SYNC, &MIGRATE, &DOCTOR, &TEMPLATE,
        &BACKUP, &RESTORE, &CLOUD, &AUTH, &VAULT,
    ];

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
