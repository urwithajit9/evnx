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
    /// The environment file to sync from (forward) or into (reverse).
    #[arg(long, default_value = ".env")]
    pub env: String,

    /// The template to sync into (forward) or from (reverse).
    #[arg(long, default_value = ".env.example")]
    pub example: String,

    /// Use `.env.<NAME>` instead of `.env`.
    ///
    /// `--env-name production` resolves to `.env.production`. A name whose file
    /// does not exist is an error listing the environments that do.
    #[arg(long, value_name = "NAME", conflicts_with = "env")]
    pub env_name: Option<String>,

    /// Direction of sync operation
    #[arg(long, value_enum, default_value_t = SyncDirection::Forward)]
    pub direction: SyncDirection,

    /// Use placeholder values when adding new variables
    #[arg(long)]
    pub placeholder: bool,

    /// Preview changes without writing to files
    #[arg(long, short = 'n')]
    pub dry_run: bool,

    /// Report whether the files are in step through the exit code.
    ///
    /// Implies --dry-run: nothing is ever written. This is the CI gate.
    ///
    ///   0  in sync
    ///   1  out of sync — commit the result of `evnx sync`
    ///   2  error — a file is missing, or will not parse
    ///
    /// The three codes follow `diff` and `grep`, so a pipeline can tell a stale
    /// template from a broken run. Plain --dry-run previews and always exits 0.
    #[arg(long, verbatim_doc_comment)]
    pub check: bool,

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

/// Subcommands for `evnx auth`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum AuthCommands {
    /// Create an evnx cloud account.
    ///
    /// Keys are derived on this machine. The master password is never sent —
    /// the server receives an SRP verifier, two salts, two public keys, and the
    /// private key sealed under the master key.
    Register {
        /// Email address. Prompted for when omitted.
        #[arg(long, value_name = "EMAIL")]
        email: Option<String>,

        /// Read the master password from stdin instead of prompting.
        ///
        /// For scripts and the end-to-end test harness. The password is taken up
        /// to the first newline; spaces are kept, because a passphrase may
        /// contain them.
        #[arg(long)]
        password_stdin: bool,
    },

    /// Sign in to evnx cloud.
    ///
    /// Runs the SRP-6a exchange: the password is never sent, and the server has
    /// to prove it holds your verifier before any session is stored.
    Login {
        /// Email address. Prompted for when omitted.
        #[arg(long, value_name = "EMAIL")]
        email: Option<String>,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// Sign out of evnx cloud.
    ///
    /// Local credentials are removed even if the server cannot be reached, so
    /// `logout` on a borrowed machine always clears that disk.
    Logout,

    /// Manage API tokens for CI/CD.
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },

    /// Manage two-factor authentication.
    Totp {
        #[command(subcommand)]
        command: TotpCommands,
    },

    /// List and revoke active sessions.
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Show your account as the server sees it.
    ///
    /// Makes an authenticated request. For local state without a network call,
    /// use `evnx cloud status`.
    Status,
}

/// Subcommands for `evnx auth token`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum TokenCommands {
    /// Mint a token. Shown once, then only revocable.
    ///
    /// A token authenticates API requests; it does not decrypt. A pipeline also
    /// needs the master password, so scope the token tightly.
    Create {
        /// A name you will recognise later, e.g. "github-actions".
        name: String,

        /// `read` or `read_write`.
        #[arg(long, default_value = "read")]
        scope: String,

        /// Restrict the token to one vault. Strongly recommended for CI.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Expire after this many days. Omit for a token that never expires.
        #[arg(long, value_name = "DAYS")]
        expires_in_days: Option<i64>,
    },

    /// List the account's live tokens.
    List,

    /// Revoke a token by name or id.
    Revoke {
        /// Token name or id.
        target: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

/// Subcommands for `evnx auth totp`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum TotpCommands {
    /// Turn on two-factor authentication.
    ///
    /// Shows a QR code to scan, then issues ten single-use recovery codes.
    /// Save them: the server holds only ciphertext, so a lost authenticator
    /// with no recovery code is an account nobody can restore.
    Enable,

    /// Turn two-factor authentication off.
    ///
    /// Requires a current code, not just a live session, so a stolen session
    /// cannot strip the second factor off your account.
    Disable,

    /// Issue a fresh set of recovery codes, invalidating the old ones.
    RecoveryCodes,
}

/// Subcommands for `evnx auth sessions`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum SessionCommands {
    /// List active sessions.
    List,

    /// Revoke one session by id, or by the short form the listing shows.
    Revoke {
        /// Session id, or enough of its start to be unambiguous.
        target: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Revoke every session except this one.
    RevokeOthers {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

/// Subcommands for `evnx vault`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum VaultCommands {
    /// Create a vault.
    ///
    /// A fresh 256-bit key is generated here and wrapped under your master key
    /// before it leaves the machine. The server stores only the wrapped form.
    Create {
        /// Vault name — letters, digits and hyphens.
        name: String,

        /// One of: production, staging, development, test.
        #[arg(long, short, default_value = "development")]
        env: String,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// List the vaults you can reach.
    List,

    /// Delete a vault and every version in it.
    Delete {
        /// Vault to delete: `name`, `name/environment`, or an id.
        target: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Share a vault with another evnx account.
    ///
    /// Your master password opens the vault key on this machine, and it is
    /// re-wrapped to the recipient's public keys before it leaves. The server
    /// never sees it unwrapped and cannot grant access itself.
    ///
    /// The wrap is hybrid X25519 + ML-KEM-768, so it holds even against an
    /// adversary recording it today for a quantum computer later.
    Share {
        /// Vault to share: `name`, `name/environment`, or an id.
        target: String,

        /// Email address of the evnx account to share with.
        #[arg(long)]
        with: String,

        /// Permission level: viewer, developer, or admin.
        #[arg(long, default_value = "developer")]
        role: String,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// List who can reach a vault.
    ///
    /// Shows every member, their role, and whether they hold a post-quantum
    /// sharing key. Any member can run this — seeing who else holds a key to a
    /// vault you hold a key to is how you notice a grant that should not be
    /// there.
    Members {
        /// Vault to inspect: `name`, `name/environment`, or an id.
        target: String,
    },

    /// Change a member's role.
    ///
    /// You can only set a role below your own, and only for someone you already
    /// outrank — so an admin can move people between viewer and developer, and
    /// only the owner can make or unmake an admin.
    Role {
        /// Vault: `name`, `name/environment`, or an id.
        target: String,

        /// Email address of the member whose role is changing.
        #[arg(long)]
        user: String,

        /// New role: viewer, developer, or admin.
        #[arg(long)]
        role: String,
    },

    /// Revoke a member's access, rotating the vault key.
    ///
    /// ⚠️ This re-encrypts every version of the vault under a fresh key and
    /// re-wraps it for everyone who stays. It is not a quick operation and it is
    /// not reversible.
    ///
    /// What it achieves: the removed member can no longer read anything pushed
    /// from now on. What it cannot: recall copies of what they could already
    /// read. Rotate those secrets at their source.
    Revoke {
        /// Vault: `name`, `name/environment`, or an id.
        target: String,

        /// Email address of the member to remove.
        #[arg(long)]
        user: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Remove the member WITHOUT rotating the vault key.
        ///
        /// ⚠️ They keep the ability to decrypt every version they already had
        /// access to, including ones pushed after this. Only use it when you are
        /// re-keying separately, or when the member never held the key.
        #[arg(long)]
        no_rekey: bool,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
}

/// Subcommands for `evnx cloud`.
///
/// Only `status` exists so far. Push, pull and history arrive with the sync
/// commands; account and vault management live under `evnx auth` and
/// `evnx vault` rather than here, matching the server's own route grouping.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum CloudCommands {
    /// Encrypt a .env and upload it as a new version.
    ///
    /// The file is encrypted here, byte for byte — comments, ordering and
    /// quoting all survive. Only the key *names* are sent in the clear, so the
    /// dashboard can list what a vault holds.
    Push {
        /// Vault to push to: `name`, `name/environment`, or an id.
        ///
        /// Optional in a directory bound with `evnx cloud link`.
        // No short form: `-V` collides with clap's auto-generated `--version`,
        // and `-v` is already `--verbose`. The collision is a debug assertion
        // that fires only when the parser is built, so it panics for the user
        // rather than failing the build — see the `cli_definition_is_valid` test.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// File to push.
        ///
        /// Defaults to `.env.<environment>` when the vault names one and that
        /// file exists — pushing `app/production` picks up `.env.production` —
        /// and to `.env` otherwise. The file chosen is always printed.
        #[arg(long, short)]
        file: Option<std::path::PathBuf>,

        /// Use `.env.<NAME>` instead of the derived or default file.
        ///
        /// A name whose file does not exist is an error listing the ones that do.
        #[arg(long, value_name = "NAME", conflicts_with = "file")]
        env_name: Option<String>,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// Download a version and decrypt it.
    ///
    /// `--version` here means the vault version, so clap's auto-generated
    /// `--version` is disabled on this subcommand. Use `evnx --version` for the
    /// program version.
    #[command(disable_version_flag = true)]
    Pull {
        /// Vault to pull from: `name`, `name/environment`, or an id.
        ///
        /// Optional in a directory bound with `evnx cloud link`.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Where to write the decrypted file.
        ///
        /// Defaults to `.env.<environment>` when the vault names one and that
        /// file already exists, and to `.env` otherwise — a first pull into a
        /// fresh checkout writes `.env`. The file chosen is always printed.
        #[arg(long, short)]
        file: Option<std::path::PathBuf>,

        /// Use `.env.<NAME>` instead of the derived or default file.
        ///
        /// A name whose file does not exist is an error listing the ones that do.
        #[arg(long, value_name = "NAME", conflicts_with = "file")]
        env_name: Option<String>,

        /// Version to pull. Defaults to the latest.
        #[arg(long, value_name = "N")]
        version: Option<i32>,

        /// Overwrite the target without asking.
        #[arg(long)]
        force: bool,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// List a vault's versions, newest first.
    ///
    /// Read-only: no password, no decryption, no blob download.
    History {
        /// Vault to inspect. Optional in a bound directory.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// How many versions to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Bind this directory to a vault, so push and pull need no --vault.
    ///
    /// Writes `[cloud] vault` into .evnx.toml, preserving everything else in
    /// the file. Safe to commit: a vault name is not a secret.
    Link {
        /// Vault to bind: `name`, `name/environment`, or an id.
        vault: String,
    },

    /// Remove this directory's vault binding.
    Unlink,

    /// Show whether this machine is set up for cloud sync.
    Status {
        /// Also check that the server is reachable.
        ///
        /// Without this, `status` reads local files only and works offline.
        /// With it, the command exits non-zero when the server cannot be
        /// reached, so it can be used as a health check in a script.
        #[arg(long)]
        ping: bool,
    },
}

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

        /// Skip all prompts. Creates blank .env files unless --blueprint names a stack.
        #[arg(short, long)]
        yes: bool,

        /// Generate from a named stack blueprint, e.g. `t3_modern`.
        ///
        /// Selects Blueprint mode without prompting. An unknown id is an error
        /// that lists the valid ones, so this doubles as the way to see them.
        #[arg(long, value_name = "ID")]
        blueprint: Option<String>,

        /// Overwrite an existing .env.example.
        ///
        /// `.env.example` is normally a tracked, hand-maintained file, so init
        /// asks before replacing it and refuses outright under --yes. This is how
        /// a script says it meant to replace it. `.env` is never overwritten.
        #[arg(long)]
        force: bool,
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

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

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

        /// Lowest confidence worth reporting: high, medium or low.
        ///
        /// Filters what is reported, counted and exited on — all three describe
        /// the same set. `--severity high` reports only detections that match a
        /// known provider format, which is the usual choice for a blocking CI
        /// gate; the default reports everything.
        #[arg(long, value_name = "LEVEL", default_value = "low")]
        severity: String,

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

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

        /// Compare against `.env.<NAME>` instead of `.env.example`.
        ///
        /// Comparing two real environments is the useful diff —
        /// `evnx diff --env-name production --against staging`. The default
        /// compares against the template, where every filled-in value differs by
        /// construction.
        #[arg(long, value_name = "NAME", conflicts_with = "example")]
        against: Option<String>,
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

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

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

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,
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

        /// Back up `.env.<NAME>` instead.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

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

    /// Manage your evnx cloud account.
    #[cfg(feature = "cloud")]
    #[command(after_help = docs::AUTH.after_help)]
    Auth {
        #[command(subcommand)]
        command: AuthCommands,

        /// evnx server to talk to. Overrides EVNX_SERVER and config.toml.
        #[arg(long, value_name = "URL", global = true)]
        server: Option<String>,
    },

    /// Manage encrypted vaults.
    #[cfg(feature = "cloud")]
    #[command(after_help = docs::VAULT.after_help)]
    Vault {
        #[command(subcommand)]
        command: VaultCommands,

        /// evnx server to talk to. Overrides EVNX_SERVER and config.toml.
        #[arg(long, value_name = "URL", global = true)]
        server: Option<String>,
    },

    /// Sync encrypted .env files with evnx cloud.
    ///
    /// Secrets are encrypted on this machine before upload. The server stores
    /// ciphertext and cannot read them.
    #[cfg(feature = "cloud")]
    #[command(after_help = docs::CLOUD.after_help)]
    Cloud {
        #[command(subcommand)]
        command: CloudCommands,

        /// evnx server to talk to. Overrides EVNX_SERVER and config.toml.
        ///
        /// Plain http:// is refused for anything but a loopback address, because
        /// the access and refresh tokens travel in request headers.
        #[arg(long, value_name = "URL", global = true)]
        server: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Build the entire command tree and run clap's own consistency checks.
    ///
    /// clap validates argument definitions with debug assertions that fire when
    /// the parser is *constructed*, not when the crate is compiled. So a
    /// duplicated short flag, a bad default, or a conflicting group compiles
    /// cleanly, passes clippy, passes every other test — and then panics for the
    /// user on their first invocation.
    ///
    /// That is exactly what happened: `--vault -V` collided with clap's
    /// auto-generated `--version`, and nothing caught it until the command was
    /// actually run. This test builds the tree so the next one fails here.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
