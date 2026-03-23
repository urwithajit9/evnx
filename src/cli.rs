//! CLI argument parsing for evnx.
//!
//! Uses clap derive macros for type-safe argument handling.

use crate::docs;
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
// MigrateOptions: flattened struct for the Migrate subcommand
//
// Extracted from the inline variant fields so the Commands enum
// variant becomes Migrate(Box<MigrateOptions>) — a single pointer
// (~8 bytes) instead of ~435 bytes inline, eliminating the
// large_enum_variant clippy warning without boxing individual fields.
// ─────────────────────────────────────────────────────────────

#[cfg(feature = "migrate")]
#[derive(Args, Debug)]
pub struct MigrateOptions {
    // ── Source ────────────────────────────────────────────────────────────────
    /// Source system: `env-file` (default) or `environment`
    #[arg(long)]
    pub from: Option<String>,

    /// Path to the source .env file (used when --from env-file)
    #[arg(long, default_value = ".env")]
    pub source_file: String,

    // ── Destination ───────────────────────────────────────────────────────────
    /// Destination system: github-actions | aws-secrets-manager | doppler |
    /// infisical | gcp-secret-manager | azure-keyvault | vercel | heroku | railway
    #[arg(long)]
    pub to: Option<String>,

    // ── Behaviour ─────────────────────────────────────────────────────────────
    /// Preview what would be migrated without making any changes
    #[arg(long)]
    pub dry_run: bool,

    /// Skip secrets that already exist at the destination
    #[arg(long)]
    pub skip_existing: bool,

    /// Overwrite secrets that already exist without prompting
    #[arg(long)]
    pub overwrite: bool,

    // ── Filtering / key transforms ────────────────────────────────────────────
    /// Comma-separated glob patterns — only migrate matching keys.
    /// Example: --include "DB_*,AWS_*"
    #[arg(long, value_delimiter = ',')]
    pub include: Option<Vec<String>>,

    /// Comma-separated glob patterns — skip matching keys.
    /// Example: --exclude "*_LOCAL,*_TEST"
    #[arg(long, value_delimiter = ',')]
    pub exclude: Option<Vec<String>>,

    /// Strip this prefix from key names before uploading.
    /// Example: --strip-prefix "APP_"  →  APP_DB_URL becomes DB_URL
    #[arg(long)]
    pub strip_prefix: Option<String>,

    /// Add this prefix to key names before uploading.
    /// Example: --add-prefix "PROD_"
    #[arg(long)]
    pub add_prefix: Option<String>,

    // ── GitHub Actions ────────────────────────────────────────────────────────
    /// GitHub repository in owner/repo format
    #[arg(long)]
    pub repo: Option<String>,

    /// GitHub Personal Access Token (or set GITHUB_TOKEN env var)
    #[arg(long, env = "GITHUB_TOKEN")]
    pub github_token: Option<String>,

    // ── AWS Secrets Manager ───────────────────────────────────────────────────
    /// AWS Secrets Manager secret name, e.g. prod/myapp/config
    #[arg(long)]
    pub secret_name: Option<String>,

    /// AWS CLI named profile
    #[arg(long)]
    pub aws_profile: Option<String>,

    // ── Doppler / Infisical ───────────────────────────────────────────────────
    /// Doppler project slug or Infisical project ID
    #[arg(long)]
    pub project: Option<String>,

    /// Doppler config name (dev / staging / prd)
    #[arg(long)]
    pub doppler_config: Option<String>,

    /// Infisical environment name (dev / staging / prod)
    #[arg(long)]
    pub infisical_env: Option<String>,

    // ── Azure Key Vault ───────────────────────────────────────────────────────
    /// Azure Key Vault name
    #[arg(long)]
    pub vault_name: Option<String>,

    // ── Heroku ────────────────────────────────────────────────────────────────
    /// Heroku application name
    #[arg(long)]
    pub heroku_app: Option<String>,

    // ── Vercel ────────────────────────────────────────────────────────────────
    /// Vercel project ID or name
    #[arg(long)]
    pub vercel_project: Option<String>,

    // ── Railway ───────────────────────────────────────────────────────────────
    /// Railway project ID
    #[arg(long)]
    pub railway_project: Option<String>,
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
    #[command(after_help = docs::INIT.after_help)]
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
    #[command(after_help = docs::ADD.after_help)]
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
    #[command(after_help = docs::VALIDATE.after_help)]
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
    #[command(after_help = docs::SCAN.after_help)]
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
    #[command(after_help = docs::DIFF.after_help)]
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

📖  Full guide: https://www.evnx.dev/guides/commands/convert
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
    ///
    /// Supports: github-actions, aws-secrets-manager, doppler, infisical,
    /// gcp-secret-manager, azure-keyvault, vercel, heroku, railway
    ///
    /// Examples:
    ///   evnx migrate --to github-actions --repo owner/repo
    ///   evnx migrate --to aws-secrets-manager --secret-name prod/myapp/config
    ///   evnx migrate --to doppler --project myapp --doppler-config dev --dry-run
    #[cfg(feature = "migrate")]
    #[command(after_help = docs::MIGRATE.after_help)]
    Migrate(Box<MigrateOptions>),

    /// Keep .env and .env.example in sync.
    #[command(after_help = docs::SYNC.after_help)]
    Sync {
        #[command(flatten)]
        args: SyncArgs,
    },

    /// Generate config files from templates.
    #[command(after_help = docs::TEMPLATE.after_help)]
    Template {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output: String,
        #[arg(long, default_value = ".env")]
        env: String,
        /// Automatically add the output file to .gitignore (no prompt).
        /// Useful in CI scripts. Mutually exclusive with --no-gitignore.
        #[arg(long, conflicts_with = "no_gitignore")]
        gitignore: bool,

        /// Skip all .gitignore checks and warnings.
        /// Use when you manage .gitignore externally. Mutually exclusive with --gitignore.
        #[arg(long, conflicts_with = "gitignore")]
        no_gitignore: bool,
    },

    /// Create encrypted backup of .env.
    #[cfg(feature = "backup")]
    #[command(after_help = docs::BACKUP.after_help)]
    Backup {
        /// Path to the .env file to back up.
        #[arg(default_value = ".env")]
        env: String,

        /// Destination path for the encrypted backup (default: <env>.backup).
        #[arg(long)]
        output: Option<String>,

        /// Path to a key file to use as the encryption password.
        ///
        /// Enables non-interactive / CI usage. The file's contents are read and
        /// fed into Argon2id in place of a typed password. UTF-8 content is used
        /// as-is (trimmed); binary content is Base64-encoded first.
        #[arg(long, value_name = "PATH")]
        key_file: Option<String>,

        /// Number of previous backups to retain alongside the new one.
        ///
        /// Before writing, existing backups are rotated:
        /// <output> → <output>.1 → <output>.2 → … → <output>.{keep-1}.
        /// Files beyond this limit are warned about but never deleted.
        /// Set to 0 to disable rotation (overwrite silently).
        #[arg(long, default_value = "3", value_name = "N")]
        keep: u32,

        /// Re-decrypt the backup after writing and verify content integrity.
        ///
        /// Costs one additional Argon2id round (~1 s) but proves the backup is
        /// readable before you discard the source file. Exit code 6 on failure.
        #[arg(long)]
        verify: bool,
    },

    /// Restore from encrypted backup.
    #[cfg(feature = "backup")]
    #[command(after_help = docs::RESTORE.after_help)]
    Restore {
        backup: String,
        #[arg(long, default_value = ".env")]
        output: String,
        /// Decrypt and validate but do not write any files.
        #[arg(long)]
        dry_run: bool,
        /// List variable names (never values) without writing any files.
        ///
        /// Decrypts the backup and prints each key name to stdout.
        /// Values are never displayed. No files are written and no overwrite
        /// prompt is shown. Use this to answer "what keys are in this backup?"
        /// before committing to a full restore.
        ///
        /// Example:
        ///   evnx restore .env.backup --inspect
        #[arg(long)]
        inspect: bool,

        /// Read the decryption password from a file instead of prompting.
        ///
        /// The file should contain only the password, with an optional
        /// trailing newline (stripped automatically). Intended for CI/CD
        /// pipelines where interactive prompts are not possible.
        ///
        /// Security: less secure than the interactive prompt. The file path
        /// may appear in process listings and shell history. Use a secrets
        /// manager or a tmpfs-backed path (e.g. /run/secrets/) in production.
        ///
        /// EVNX_PASSWORD environment variable is also accepted and takes
        /// lower priority than --password-file.
        ///
        /// Examples:
        ///   evnx restore .env.backup --password-file /run/secrets/evnx-pass
        ///   EVNX_PASSWORD=mypass evnx restore .env.backup
        #[arg(long, value_name = "PATH")]
        password_file: Option<String>,
    },

    /// Diagnose common setup issues.
    #[command(about = "Check .env files, Git config, project structure, and security")]
    #[command(after_help = docs::DOCTOR.after_help)]
    Doctor {
        /// Project directory to analyze
        #[arg(default_value = ".", index = 1)]
        path: String,

        /// Show detailed diagnostic output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Generate shell completions.
    // #[command(after_help = docs::INIT.after_help)]
    Completions { shell: String },
}
