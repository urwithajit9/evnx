//! CLI argument parsing for evnx.
//!
//! Uses clap derive macros for type-safe argument handling.

use clap::{Args, Parser, Subcommand, ValueEnum};

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

// ------------------------------------
// sync related Enum and implementation
// -------------------------------------

/// Direction for sync operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum SyncDirection {
    /// Sync .env → .env.example (add new vars to template)
    #[default]
    Forward,
    /// Sync .env.example → .env (add missing vars to local env)
    Reverse,
}

impl std::fmt::Display for SyncDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncDirection::Forward => write!(f, "forward"),
            SyncDirection::Reverse => write!(f, "reverse"),
        }
    }
}

/// Keep .env and .env.example in sync.
#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Direction of sync operation
    #[arg(long, value_enum, default_value_t = SyncDirection::Forward)]
    pub direction: SyncDirection,

    /// Use placeholder values when adding new variables
    #[arg(long)]
    pub placeholder: bool,

    /// Preview changes without writing to files
    #[arg(long, short = 'n')]
    pub dry_run: bool,

    /// Skip interactive prompts (for CI/CD usage)
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Path to custom placeholder template config (JSON)
    #[arg(long, value_name = "PATH")]
    pub template_config: Option<std::path::PathBuf>,

    /// Warn on non-standard env var naming (default: warn)
    #[arg(long, value_enum, default_value_t = NamingPolicy::Warn)]
    pub naming_policy: NamingPolicy,
}

/// Policy for handling non-standard environment variable names
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum NamingPolicy {
    /// Warn but continue (default)
    #[default]
    Warn,
    /// Treat non-standard names as errors
    Error,
    /// Ignore naming conventions entirely
    Ignore,
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

        /// Comma-separated list of issue types to ignore
        #[arg(long, value_delimiter = ',')]
        ignore: Vec<String>,

        /// Validate value formats: url, port, email
        #[arg(long)]
        validate_formats: bool,

        /// Environment pattern (.env.production, .env.local, etc.)
        #[arg(long, short = 'p')]
        pattern: Option<String>,
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
        /// Ignore these keys (comma-separated) — useful for env-specific vars
        #[arg(long, value_delimiter = ',')]
        ignore_keys: Vec<String>,
        /// Include extended statistics in JSON output
        #[arg(long, default_value_t = false)]
        with_stats: bool,
        /// Enable interactive merge mode (patch format only)
        #[arg(short, long, default_value_t = false)]
        interactive: bool,
    },

    /// Transform `.env` files into multiple output formats.
    ///
    /// Supports 14+ formats including JSON, YAML, cloud configs,
    /// CI/CD variables, and infrastructure-as-code.
    ///
    /// # Examples
    ///
    /// ```text
    /// # Basic JSON conversion
    /// evnx convert --to json
    ///
    /// # Kubernetes secret with transformations
    /// evnx convert \
    ///   --to kubernetes \
    ///   --include "PROD_*" \
    ///   --prefix "MYAPP_" \
    ///   --transform uppercase \
    ///   --output k8s-secret.yaml
    ///
    /// # Interactive mode (opens format selector)
    /// evnx convert
    /// ```
    #[command(after_help = "\
Supported formats:
  Generic:        json, yaml, shell
  Cloud:          aws-secrets, gcp-secrets, azure-keyvault
  CI/CD:          github-actions
  Containers:     docker-compose, kubernetes
  IaC:            terraform
  Secret Managers: doppler, heroku, vercel, railway

Aliases:
  k8s → kubernetes, tf → terraform, yml → yaml
  gh-actions → github-actions, compose → docker-compose

Use 'evnx convert' without --to for interactive format selection.
")]
    Convert {
        /// Path to input .env file
        #[arg(long, default_value = ".env", value_name = "PATH")]
        env: String,

        /// Target output format (omit for interactive selection)
        ///
        /// Supported: json, yaml, shell, aws-secrets, gcp-secrets,
        /// azure-keyvault, github-actions, docker-compose, kubernetes,
        /// terraform, doppler, heroku, vercel, railway
        #[arg(long, value_name = "FORMAT")]
        to: Option<String>,

        /// Write output to file instead of stdout
        #[arg(long, short, value_name = "FILE")]
        output: Option<String>,

        /// Include only variables matching this glob pattern
        ///
        /// Examples: "AWS_*", "*_URL", "*_SECRET_*"
        /// Supports: prefix*, *suffix, *contains*, exact
        #[arg(long, value_name = "PATTERN")]
        include: Option<String>,

        /// Exclude variables matching this glob pattern
        ///
        /// Examples: "*_DEBUG", "TEST_*"
        /// Applied after --include filtering
        #[arg(long, value_name = "PATTERN")]
        exclude: Option<String>,

        /// Base64-encode all values before output
        ///
        /// Note: Some formats (e.g., kubernetes) always base64-encode
        /// regardless of this flag
        #[arg(long)]
        base64: bool,

        /// Prefix to prepend to all variable names
        ///
        /// Example: --prefix "APP_" transforms "KEY" → "APP_KEY"
        /// Applied before key casing transformation
        #[arg(long, value_name = "PREFIX")]
        prefix: Option<String>,

        /// Transform variable name casing
        ///
        /// Options: uppercase, lowercase, camelCase, snake_case
        ///
        /// Examples:
        ///   "database_url" --transform uppercase → "DATABASE_URL"
        ///   "DATABASE_URL" --transform camelCase → "databaseUrl"
        ///   "DatabaseURL" --transform snake_case → "database_url"
        #[arg(long, value_name = "MODE")]
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
        #[command(flatten)]
        args: SyncArgs,
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
        /// Decrypt and validate but do not write any files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Diagnose common setup issues.
    #[command(about = "Check .env files, Git config, project structure, and security")]
    Doctor {
        /// Project directory to analyze
        #[arg(default_value = ".", index = 1)]
        path: String,

        /// Show detailed diagnostic output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Generate shell completions.
    Completions { shell: String },
}
