// src/docs.rs
//
// Single source of truth for all evnx documentation URLs.
// To update a URL: change it here — it propagates to help text
// (via cli.rs after_help) and command output (via ui::print_docs_hint) automatically.

pub const BASE_URL: &str = "https://www.evnx.dev";

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
    /// Output: `  📖 Docs: https://www.evnx.dev/guides/commands/init`
    pub fn hint_line(&self) -> String {
        format!("  📖 Docs: {}", self.url)
    }
}

// ── Registry ──────────────────────────────────────────────────────────────────
//
// One entry per subcommand. `after_help` is a plain string literal so it can
// be used directly in #[command(after_help = docs::INIT.after_help)].

pub const INIT: CommandDoc = CommandDoc {
    command: "init",
    url: "https://www.evnx.dev/guides/commands/init",
    description: "Project setup, stacks, and service presets",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/init",
};

pub const ADD: CommandDoc = CommandDoc {
    command: "add",
    url: "https://www.evnx.dev/guides/commands/add",
    description: "Adding variables interactively or from blueprints",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/add",
};

pub const VALIDATE: CommandDoc = CommandDoc {
    command: "validate",
    url: "https://www.evnx.dev/guides/commands/validate",
    description: "Validation rules, CI flags, and strict mode",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/validate",
};

pub const SCAN: CommandDoc = CommandDoc {
    command: "scan",
    url: "https://www.evnx.dev/guides/commands/scan",
    description: "Secret detection patterns and entropy analysis",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/scan",
};

pub const DIFF: CommandDoc = CommandDoc {
    command: "diff",
    url: "https://www.evnx.dev/guides/commands/diff",
    description: "Comparing .env vs .env.example",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/diff",
};

pub const CONVERT: CommandDoc = CommandDoc {
    command: "convert",
    url: "https://www.evnx.dev/guides/commands/convert",
    description: "All 14 output formats and filtering options",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/convert",
};

pub const SYNC: CommandDoc = CommandDoc {
    command: "sync",
    url: "https://www.evnx.dev/guides/commands/sync",
    description: "Keeping .env and .env.example in sync",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/sync",
};

pub const MIGRATE: CommandDoc = CommandDoc {
    command: "migrate",
    url: "https://www.evnx.dev/guides/commands/migrate",
    description: "Migrating secrets to cloud managers",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/migrate",
};

pub const DOCTOR: CommandDoc = CommandDoc {
    command: "doctor",
    url: "https://www.evnx.dev/guides/commands/doctor",
    description: "Diagnosing setup and gitignore issues",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/doctor",
};

pub const TEMPLATE: CommandDoc = CommandDoc {
    command: "template",
    url: "https://www.evnx.dev/guides/commands/template",
    description: "Generating config files from templates",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/template",
};

pub const BACKUP: CommandDoc = CommandDoc {
    command: "backup",
    url: "https://www.evnx.dev/guides/commands/backup",
    description: "AES-256-GCM encrypted backups",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/backup",
};

pub const RESTORE: CommandDoc = CommandDoc {
    command: "restore",
    url: "https://www.evnx.dev/guides/commands/restore",
    description: "Restoring from encrypted backups",
    after_help: "📖  Full guide: https://www.evnx.dev/guides/commands/restore",
};
