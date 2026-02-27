//! CLI argument parsing for evnx.
//!
//! Uses clap derive macros for type-safe argument handling.

use clap::{Parser, Subcommand};

// ─────────────────────────────────────────────────────────────
// AddTarget: Subcommands for `evnx add`
// ─────────────────────────────────────────────────────────────

/// Target for adding environment variables.
#[derive(Subcommand, Debug, Clone)]
pub enum AddTarget {
    /// Add variables for a specific service.
    ///
    /// Example: evnx add service postgresql
    Service {
        /// Service ID (e.g., "postgresql", "redis", "stripe").
        #[arg()]
        service: String,
    },

    /// Add variables for a framework.
    ///
    /// Example: evnx add framework --language python django
    Framework {
        /// Language ID (e.g., "python", "javascript_typescript").
        #[arg(long, short)]
        language: String,

        /// Framework ID (e.g., "django", "nextjs", "axum_actix").
        #[arg()]
        framework: String,
    },

    /// Add variables from a stack blueprint (without overwriting existing).
    ///
    /// Example: evnx add blueprint t3_modern
    Blueprint {
        /// Blueprint ID (e.g., "t3_modern", "rust_high_perf").
        #[arg()]
        blueprint: String,
    },

    /// Add custom variables interactively.
    Custom,
}

// ─────────────────────────────────────────────────────────────
// Cli: Top-level CLI structure
// ─────────────────────────────────────────────────────────────

/// evnx — Manage .env files with validation, secret scanning, and format conversion.
#[derive(Parser)]
#[command(
    name = "evnx",
    about = "Manage .env files — validation, secret scanning, and format conversion",
    version,
    author,
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,
}

// ─────────────────────────────────────────────────────────────
// Commands: All available subcommands
// ─────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Interactive project setup — generates .env.example.
    ///
    /// Three modes available:
    /// • Blank: Create empty .env files (add variables later with `evnx add`)
    /// • Blueprint: Use a pre-configured stack like "T3 Turbo" or "Rust High-Perf"
    /// • Architect: Build your stack step-by-step (language → framework → services → infra)
    Init {
        /// Output path for .env files.
        #[arg(long, default_value = ".")]
        path: String,

        /// Skip all prompts and use defaults.
        #[arg(short, long)]
        yes: bool,
    },

    /// Add environment variables to existing .env files.
    ///
    /// Subcommands:
    /// • service: Add vars for a service (e.g., postgresql)
    /// • framework: Add vars for a framework (e.g., django)
    /// • blueprint: Add vars from a stack blueprint
    /// • custom: Interactive custom variable addition
    Add {
        #[command(subcommand)]
        target: AddTarget,

        /// Output path (default: current directory).
        #[arg(long, short, default_value = ".", global = true)]
        path: String,

        /// Skip confirmation prompts.
        #[arg(long, short, global = true)]
        yes: bool,
    },

    /// Check .env against .env.example, find issues.
    Validate {
        #[arg(long, default_value = ".env")]
        env: String,
        #[arg(long, default_value = ".env.example")]
        example: String,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        fix: bool,
        #[arg(long, default_value = "pretty")]
        format: String,
        #[arg(long)]
        exit_zero: bool,
    },

    /// Detect secrets that look real (AWS keys, tokens, etc.).
    Scan {
        #[arg(default_value = ".")]
        path: Vec<String>,
        #[arg(long, default_value = ".env.example")]
        exclude: Vec<String>,
        #[arg(long)]
        pattern: Vec<String>,
        #[arg(long)]
        ignore_placeholders: bool,
        #[arg(long, default_value = "pretty")]
        format: String,
        #[arg(long)]
        exit_zero: bool,
    },

    /// Compare .env vs .env.example — show missing/extra vars.
    Diff {
        #[arg(long, default_value = ".env")]
        env: String,
        #[arg(long, default_value = ".env.example")]
        example: String,
        #[arg(long)]
        show_values: bool,
        #[arg(long, default_value = "pretty")]
        format: String,
        #[arg(long)]
        reverse: bool,
    },

    /// Transform to different formats (JSON, YAML, shell, etc.).
    Convert {
        #[arg(long, default_value = ".env")]
        env: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        output: Option<String>,
        #[arg(long)]
        include: Option<String>,
        #[arg(long)]
        exclude: Option<String>,
        #[arg(long)]
        base64: bool,
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        transform: Option<String>,
    },

    /// Full migration workflow to secret managers.
    #[cfg(feature = "migrate")]
    Migrate {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = ".env")]
        source_file: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        secret_name: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        skip_existing: bool,
        #[arg(long)]
        overwrite: bool,
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,
        #[arg(long)]
        aws_profile: Option<String>,
    },

    /// Keep .env and .env.example in sync.
    Sync {
        #[arg(long, default_value = "forward")]
        direction: String,
        #[arg(long)]
        placeholder: bool,
    },

    /// Generate config files from templates.
    Template {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output: String,
        #[arg(long, default_value = ".env")]
        env: String,
    },

    /// Create encrypted backup of .env.
    #[cfg(feature = "backup")]
    Backup {
        #[arg(default_value = ".env")]
        env: String,
        #[arg(long)]
        output: Option<String>,
    },

    /// Restore from encrypted backup.
    #[cfg(feature = "backup")]
    Restore {
        backup: String,
        #[arg(long, default_value = ".env")]
        output: String,
    },

    /// Diagnose common setup issues.
    Doctor {
        #[arg(default_value = ".")]
        path: String,
    },

    /// Generate shell completions.
    Completions { shell: String },
}
