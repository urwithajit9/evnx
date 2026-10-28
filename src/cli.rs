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
    /// Add variables for a service, infrastructure target or framework.
    ///
    /// Takes any id `evnx init --with` takes, aliases included: a service
    /// (postgresql, redis, stripe), an infrastructure target (vercel, docker)
    /// or a framework (nextjs). `add infra` is the same command.
    ///
    /// Example: evnx add service postgresql
    #[command(visible_alias = "infra")]
    Service {
        /// Catalogue id (e.g., "postgresql", "vercel"). `evnx add --list` lists
        /// them. Leave it out in a terminal to pick one.
        #[arg()]
        service: Option<String>,
    },

    /// Add variables for a framework.
    ///
    /// Framework ids are unique across languages, so `--language` is optional;
    /// when given it is checked.
    ///
    /// Example: evnx add framework nextjs
    Framework {
        /// Language id (e.g., "python", "javascript_typescript"). Optional.
        #[arg(long, short)]
        language: Option<String>,

        /// Framework id (e.g., "django", "nextjs", "axum_actix"). Leave it out
        /// in a terminal to pick one.
        #[arg()]
        framework: Option<String>,
    },

    /// Add variables from a stack blueprint (without overwriting existing).
    ///
    /// Example: evnx add blueprint t3_modern
    Blueprint {
        /// Blueprint id (e.g., "t3_modern", "rust_high_perf"). Leave it out in
        /// a terminal to pick one.
        #[arg()]
        blueprint: Option<String>,
    },

    /// Add your own variables: as arguments, or one prompt at a time.
    ///
    /// Examples:
    ///   evnx add custom API_URL=https://api.example.com DEBUG=false --yes
    ///   evnx add custom STRIPE_WEBHOOK_SECRET --desc "Stripe webhook signing secret" --required
    ///   evnx add custom            (asks for each variable)
    ///
    /// A name without `=value` gets a placeholder. When it looks like a secret
    /// (`*_SECRET`, `*_TOKEN`, `*_KEY`, `*_PASSWORD`, `*_PWD`, …) and there is a
    /// terminal, its value is asked for without echo instead. A value given as
    /// an argument lands in your shell history.
    Custom {
        /// Variables, as `NAME=value` or `NAME`. None: ask one at a time.
        #[arg(value_name = "NAME[=VALUE]")]
        vars: Vec<String>,

        /// A description comment for every variable given.
        #[arg(long)]
        desc: Option<String>,

        /// A category for every variable given: Application, Security,
        /// Database, Authentication, Service or Other.
        #[arg(long)]
        category: Option<String>,

        /// Mark every variable given as required.
        #[arg(long)]
        required: bool,

        /// Mark every variable given as optional.
        ///
        /// With a `[vars]` contract in `.evnx.toml` this writes
        /// `required = false` for each name (what `add custom` declares by
        /// default; the flag says so). `.env.example` alone cannot say
        /// "optional": without a contract the variables are still added, and
        /// the run says how to make them optional (`evnx spec init`).
        #[arg(long, conflicts_with = "required")]
        optional: bool,
    },
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
    #[arg(long)]
    pub env: Option<String>,

    /// The template to sync into (forward) or from (reverse).
    #[arg(long)]
    pub example: Option<String>,

    /// Use `.env.<NAME>` instead of `.env`.
    ///
    /// `--env-name production` resolves to `.env.production`. A name whose file
    /// does not exist is an error listing the environments that do.
    #[arg(long, value_name = "NAME", conflicts_with = "env")]
    pub env_name: Option<String>,

    /// Which file to fill: forward (the default) adds to the template,
    /// reverse adds to `.env`.
    ///
    /// Leaving it out also decides how far `--check` looks. Without
    /// it the check is two-way, for a developer's own pair: a name only the
    /// template has is drift as well. Naming a direction makes the check
    /// one-way — only names the target lacks — which is how CI and the
    /// pre-commit hook ask: `evnx sync --check --direction forward`.
    #[arg(long, value_enum)]
    pub direction: Option<SyncDirection>,

    /// Deprecated: does what --force does (placeholders, no prompt).
    ///
    /// Without a terminal sync already uses placeholders and says so.
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
    ///   1  out of sync — the message names the command that fixes each side
    ///   2  no verdict — a file is missing or will not parse, the config is
    ///      wrong, or (forward) .env is empty beside a populated template
    ///
    /// Without --direction it looks both ways: names the template lacks, and
    /// names only the template has. With --direction it looks one way, as CI
    /// and the pre-commit hook should:
    ///
    ///   evnx sync --check --direction forward
    ///
    /// With no .env, the template is checked against the required `[vars]` in
    /// .evnx.toml. The three codes follow `diff` and `grep`. Plain --dry-run
    /// previews and always exits 0.
    #[arg(long, verbatim_doc_comment)]
    pub check: bool,

    /// Output format for `--check`: `pretty` (default) or `json`.
    ///
    /// ⚠️ `json` requires `--check`, because without it `sync` edits files and a
    /// machine-readable report of an edit already made is worth less than the
    /// edit. The report says *what* drifted — until now the only answer a
    /// pipeline could get was the exit code.
    #[arg(
        long,
        value_name = "FORMAT",
        default_value = "pretty",
        value_parser = ["pretty", "json"],
        requires = "check"
    )]
    pub format: String,

    /// Skip the prompts and add placeholder values.
    ///
    /// Without a terminal, or with CI set, sync does this anyway and says so.
    #[arg(long, short = 'f')]
    pub force: bool,

    /// With no `.env` and nothing else to compare, skip (exit 0) and say so.
    ///
    /// For pre-commit hooks: a fresh clone has a template and no `.env`, and a
    /// hook that exits 2 there blocks every new contributor's first commit.
    /// A spec (`[vars]` in `.evnx.toml`) is still checked. Forward only.
    ///
    /// ⚠️ Not for CI: without it, `--check` with nothing to compare exits 2, so
    /// a gate cannot pass on a check that never ran.
    #[arg(long)]
    pub skip_without_env: bool,

    /// A JSON file of placeholder rules: `patterns`, `default`, `allow_actual`.
    ///
    /// An unknown key is an error naming the accepted ones, so a misspelt
    /// setting cannot be silently off.
    #[arg(long, value_name = "PATH")]
    pub template_config: Option<std::path::PathBuf>,

    /// What to do with a name that is not UPPER_SNAKE_CASE.
    ///
    /// Defaults to `[sync] naming_policy` in .evnx.toml, else `warn`. `error`
    /// stops the run with no verdict (exit 2), in every mode.
    #[arg(long, value_enum)]
    pub naming_policy: Option<NamingPolicy>,
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

impl NamingPolicy {
    /// `[sync] naming_policy` from `.evnx.toml`: a value the flag accepts, or
    /// an error naming them (I-121, DEC-S11). The shared rule for a value from
    /// a fixed list, as `[validate] fail_on` uses.
    ///
    /// ⚠️ An unknown value used to fall through to `warn` without a word, so
    /// `naming_policy = "eror"` switched off the protection it was written to
    /// turn on.
    pub fn from_config(value: &str) -> anyhow::Result<Self> {
        crate::core::args::one_of(
            "naming policy",
            value,
            &[
                ("warn", Self::Warn),
                ("error", Self::Error),
                ("ignore", Self::Ignore),
            ],
            &["warn", "error", "ignore"],
        )
    }
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
    /// Source system: `env-file` or `environment`
    ///
    /// Without --from, evnx asks in a terminal and uses `env-file` without one.
    /// `environment` requires `--include`: a shell's or CI job's environment holds
    /// tokens you did not mean to send. evnx lists the names it will send.
    #[arg(long)]
    pub from: Option<String>,

    /// The .env file to read (default: .env, or `[defaults] env_name`)
    #[arg(long, value_name = "PATH", conflicts_with = "env_name")]
    pub source_file: Option<String>,

    /// Read `.env.<NAME>` instead of `.env`.
    ///
    /// `--env-name production` reads `.env.production`. A name whose file does
    /// not exist is an error listing the environments that do. Without it,
    /// `[defaults] env_name` in `.evnx.toml` applies, as for the other commands.
    #[arg(long, value_name = "NAME")]
    pub env_name: Option<String>,

    // ── Destination ───────────────────────────────────────────────────────────
    /// Destination system: github-actions | aws-secrets-manager | doppler |
    /// infisical | gcp-secret-manager | azure-keyvault | vercel | heroku | railway
    #[arg(long)]
    pub to: Option<String>,

    // ── Behaviour ─────────────────────────────────────────────────────────────
    /// Preview what would be migrated without making any changes
    #[arg(long)]
    pub dry_run: bool,

    /// Write only the commands to stdout; everything else goes to stderr.
    ///
    /// `evnx migrate --to heroku --heroku-app my-app --print-only > run.sh`
    /// saves a script you can read, then run. Not for github-actions, which
    /// uploads instead of printing.
    #[arg(long, conflicts_with = "dry_run")]
    pub print_only: bool,

    /// Skip secrets that already exist at the destination
    #[arg(long)]
    pub skip_existing: bool,

    /// Overwrite secrets that already exist without prompting
    #[arg(long)]
    pub overwrite: bool,

    /// Upload without asking "Proceed with migration?" (github-actions).
    ///
    /// For CI. Without a terminal and without `--yes` the upload stops before
    /// contacting GitHub (exit 2). ⚠️ It does not answer "overwrite?": a secret
    /// that already exists still needs `--overwrite` or `--skip-existing`.
    #[arg(long, short = 'y')]
    pub yes: bool,

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

    /// When two names become one, keep the last value.
    ///
    /// `--strip-prefix` and `--add-prefix` can merge names (`API_X` and `X`
    /// under `--strip-prefix API_`), and Azure Key Vault and GitHub ignore case.
    /// Without this flag that is an error naming both, exit 2.
    #[arg(long)]
    pub allow_collisions: bool,

    // ── GitHub Actions ────────────────────────────────────────────────────────
    /// GitHub repository in owner/repo format
    #[arg(long)]
    pub repo: Option<String>,

    // ⛔ `hide_env_values` is not optional here, and this note is a `//` comment
    // rather than a `///` one **on purpose**: clap renders doc comments into
    // `--help`, so an explanation that quotes the leak reproduces it.
    //
    // By default clap prints the *current value* of an `env =` fallback. With
    // GITHUB_TOKEN exported — which is exactly how CI runs — `evnx migrate
    // --help` printed `[env: GITHUB_TOKEN=<the real token>]`. With this set it
    // prints `[env: GITHUB_TOKEN]`, so the variable stays discoverable and its
    // value does not.
    //
    // ⚠️ Passing the token as `--github-token` also puts it in argv, visible in
    // `ps` and in shell history (I-108). evnx warns when the flag is used, which
    // is why there is no `env = "GITHUB_TOKEN"` here any more: with it, clap
    // filled this field from the variable too, and a warning could not tell a
    // typed token from an exported one. The registry reads GITHUB_TOKEN itself.
    // That also retires the `hide_env_values` guard: nothing here can print the
    // variable's value now.
    /// GitHub token. Prefer exporting GITHUB_TOKEN: evnx reads it when this flag
    /// is absent, and a flag value lands in shell history
    #[arg(long)]
    pub github_token: Option<String>,

    // ── AWS Secrets Manager ───────────────────────────────────────────────────
    /// AWS Secrets Manager secret name, e.g. prod/myapp/config
    #[arg(long)]
    pub secret_name: Option<String>,

    /// AWS CLI named profile
    #[arg(long)]
    pub aws_profile: Option<String>,

    // ── Doppler / Infisical ───────────────────────────────────────────────────
    /// Doppler project, Infisical project ID, or GCP project ID (required for
    /// gcp-secret-manager)
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
    /// Vercel project ID or name (default: the linked project)
    #[arg(long)]
    pub vercel_project: Option<String>,

    // ── Railway ───────────────────────────────────────────────────────────────
    /// Railway project ID (default: the linked project). Needs
    /// --railway-environment, as Railway does
    #[arg(long)]
    pub railway_project: Option<String>,

    /// Railway environment, e.g. production
    #[arg(long)]
    pub railway_environment: Option<String>,

    /// Railway service (default: the linked service, or the project's only one)
    #[arg(long)]
    pub railway_service: Option<String>,
}

// ─────────────────────────────────────────────────────────────
// Cli: Top-level CLI structure
// ─────────────────────────────────────────────────────────────

/// Subcommands for `evnx auth`.
/// Organisations — billing and a directory.
///
/// ⛔ **An organisation does not give anyone access to a vault.** It owns a plan
/// and a set of seats; a seat holder gets that plan's limits. The server cannot
/// wrap a vault key, so no membership, role or seat can grant vault access —
/// sharing is `evnx vault share`, done by someone who holds the key.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum OrgCommands {
    /// Create an organisation. You become its owner.
    ///
    /// You hold no seat at first, deliberately: a seat can only be held in one
    /// organisation at a time, and a new organisation is on the free plan, so a
    /// seat would change nothing until billing is set up.
    Create {
        /// Display name, e.g. "Acme Corp".
        name: String,

        /// Short handle used by `--org`, e.g. `acme`. Lowercase letters, digits
        /// and hyphens.
        #[arg(long, value_name = "SLUG")]
        slug: String,
    },

    /// List the organisations this account belongs to.
    List,

    /// Who is in an organisation, and who holds a seat.
    Members {
        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.
        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Invite someone by email.
    ///
    /// ⚠️ They will not gain access to any vault. The invitation decides which
    /// plan's limits apply to them once an administrator assigns a seat.
    Invite {
        /// Email address to invite.
        email: String,

        /// `member` (default) or `admin`. ⚠️ `owner` cannot be granted — an
        /// organisation has exactly one, set when it is created.
        #[arg(long, value_name = "ROLE")]
        role: Option<String>,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Invitations sent but not yet redeemed.
    Invites {
        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.
        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Withdraw an invitation.
    Uninvite {
        /// Invitation id, from `evnx org invites`.
        invite_id: String,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Redeem an invitation token.
    ///
    /// Takes no `--org`: the token names the organisation, and you are not a
    /// member yet. Only the invited address can redeem it.
    Accept {
        /// The `evnx_inv_…` token from the invitation.
        token: String,
    },

    /// What this organisation is on, and until when.
    ///
    /// ⚠️ Read-only. Changing a plan, a seat count or a payment method happens in
    /// the browser — a card form does not belong in a terminal, and Paddle is
    /// the merchant of record for the transaction either way. This answers the
    /// question a terminal is actually the right place for: *why are my limits
    /// what they are?*
    Billing {
        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.
        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Show or set how many seats are purchased.
    ///
    /// Bare, it shows. ⚠️ Setting the count is owner-only — it is what an invoice
    /// is computed from.
    Seats {
        /// Set the purchased seat count.
        #[arg(long, value_name = "N")]
        set: Option<i64>,

        /// No seat limit.
        #[arg(long)]
        unlimited: bool,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Give someone a seat.
    ///
    /// ⚠️ A seat is a plan change: their limits become this organisation's.
    Assign {
        /// Email address, or user id.
        who: String,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Take a seat back.
    ///
    /// ⚠️ Their limits drop to their own plan. Nothing is deleted — a vault over
    /// the new limit stays readable, and the next push to it is refused.
    Release {
        /// Email address, or user id.
        who: String,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Change someone's role.
    Role {
        /// Email address, or user id.
        who: String,

        /// `member` or `admin`.
        #[arg(long, value_name = "ROLE")]
        set: String,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Remove someone from an organisation.
    Remove {
        /// Email address, or user id.
        who: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Delete an organisation.
    ///
    /// ⚠️ Every seat holder drops back to their own plan. No vault is touched and
    /// nothing anyone already pulled is recalled.
    ///
    /// Owner-only. This is also the release valve for account deletion, which is
    /// blocked while you own an organisation.
    Delete {
        /// Delete even though seats are assigned.
        #[arg(long)]
        force: bool,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },

    /// Leave an organisation.
    ///
    /// An owner cannot leave their own — transfer ownership or delete it.
    Leave {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Organisation slug, e.g. `acme`. Needed only when you belong to more than one.

        #[arg(long, value_name = "SLUG")]
        org: Option<String>,
    },
}

#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum AuthCommands {
    /// Create an evnx cloud account.
    ///
    /// Keys are derived on this machine. The master password is never sent —
    /// the server receives an SRP verifier, two salts, three public keys
    /// (Ed25519, X25519 and ML-KEM-768), and the private key sealed under the
    /// master key.
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
    ///
    /// For CI and other unattended jobs, use an API token instead: create one
    /// with `evnx auth token create` and set it as `EVNX_TOKEN`. A login can ask
    /// for a two-factor code, which no pipeline can type.
    // DEC-AU11: no unattended second factor; the help says CI uses API tokens.
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
    ///
    /// Two-factor applies to `evnx auth login`. Unattended jobs use an API token
    /// (`evnx auth token create`, then `EVNX_TOKEN`), which never asks for a
    /// code — a second factor cannot be scripted, by design.
    // DEC-AU11, as on `Login`.
    Totp {
        #[command(subcommand)]
        command: TotpCommands,
    },

    /// List and revoke active sessions.
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Networks and browsers this account has signed in from.
    ///
    /// ⚠️ Not the same as `sessions`. A session is a live credential you can
    /// revoke; a device is somewhere you have signed in from, which may have no
    /// session left at all.
    ///
    /// ⚠️ There is no location in this. evnx stores a keyed hash of the client
    /// address and the user agent, never the values, so it cannot tell where
    /// you were — a device that looks new can simply be a phone that
    /// reconnected.
    Devices {
        #[command(subcommand)]
        command: DeviceCommands,
    },

    /// Show your account as the server sees it.
    ///
    /// Makes an authenticated request. For local state without a network call,
    /// use `evnx cloud status`.
    Status,

    /// Change your master password.
    ///
    /// ⚠️ **Not a settings toggle.** The master key is derived from the password,
    /// so changing it re-derives the key and every vault key wrapped under it is
    /// re-wrapped here, on this machine, in the same command. The server receives
    /// a new verifier and a set of blobs it cannot read.
    ///
    /// What it does: signs out every other device; re-wraps the vault keys you
    /// hold under your own master key. What it does not: touch vaults shared
    /// *with* you (those are wrapped to your keypair, which this re-seals rather
    /// than replaces), and it does not re-key a vault — a former member who kept
    /// a key is unaffected. Rotating a vault key is what `evnx vault rekey`
    /// does, and what `evnx vault revoke` does when it removes someone.
    ///
    /// ⚠️ **API tokens keep working, but CI breaks.** A token authenticates; it
    /// does not decrypt. Any pipeline holding your master password as a secret
    /// fails until that secret is updated.
    ///
    /// Before sending anything this writes the previous wraps to a 0600 file in
    /// your config directory, and afterwards confirms every key still opens.
    RotateMasterPassword {
        /// Read both passwords from stdin: the current one on the first line,
        /// the new one on the second.
        ///
        /// For scripts and the end-to-end harness. Each is taken up to its
        /// newline; spaces are kept, because a passphrase may contain them.
        /// Implies `--yes`, since there is nobody to prompt.
        #[arg(long)]
        password_stdin: bool,

        /// The current password is believed to be in someone else's hands.
        ///
        /// Keeps **no undo at all**: no server-side undo window, and no local
        /// recovery file. The change takes effect immediately and cannot be
        /// reversed. Both of those are redeemed with the OLD password, which is
        /// the thing you are saying leaked.
        ///
        /// ⚠️ **This rotates the password, not your keys.** Your identity
        /// keypair and every vault key are re-wrapped under the new password,
        /// not replaced — so anyone who already extracted a vault key still has
        /// it. Follow this with `evnx vault rekey <vault>` for each vault that
        /// was exposed; that is what gives the old key nothing left to open.
        #[arg(long)]
        compromised: bool,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Undo a master password change, by proving the previous password.
    ///
    /// Does **not** need a session — whoever needs this is whoever the change
    /// locked out, and a session is exactly what they do not have. The only
    /// credential it asks for is the old master password.
    ///
    /// Only available for a window after the change, and only when the change
    /// kept one: `--compromised` keeps none.
    UndoPasswordChange {
        /// Email address. Taken from the stored session, or prompted for.
        #[arg(long, value_name = "EMAIL")]
        email: Option<String>,

        /// Read the previous master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// Download everything the server holds about your account, as JSON.
    ///
    /// Which vaults exist, who can reach them, what each version contained by
    /// variable NAME, your API tokens by name and scope, and your activity.
    ///
    /// ⚠️ **Your secrets are not in it, and cannot be.** They are encrypted on
    /// your machine under a key derived from your master password, and the
    /// server has never held that password, that key, or any plaintext value.
    /// To get the secrets themselves, run `evnx cloud pull` for each vault. The
    /// file says this too, so it is still clear months later.
    ///
    /// Nothing that could be used as a credential is included — not your SRP
    /// verifier, not your 2FA secret, not your API token values.
    ///
    /// ⚠️ The file still describes the SHAPE of your configuration: variable
    /// names, vault names, and the email of everyone you share with. Treat it
    /// as sensitive even though no secret value appears in it.
    ///
    /// ⚠️ Deliberately NOT called `export`. `evnx cloud export` is the
    /// anti-lock-in escape hatch — it writes every vault's secrets, DECRYPTED, to
    /// disk. Two commands named `export` with opposite risk profiles would mean
    /// anyone who learned "export holds no secrets" here carried that belief to
    /// the one that writes plaintext credentials.
    DownloadData {
        /// Where to write it. Defaults to `evnx-account-data-<date>.json` here.
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<std::path::PathBuf>,

        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },

    /// Delete your account and everything only you can reach.
    ///
    /// ⚠️ **Irreversible, and it is not a logout.** The account, its sessions,
    /// its API tokens, its 2FA enrolment and every vault you are the only member
    /// of are destroyed, ciphertext included. There is no undo and no support
    /// path back — the server never held anything that could rebuild them.
    ///
    /// Vaults you own that other people are members of **block the deletion**,
    /// and are named when they do. Deleting the account would delete those vaults
    /// and everyone else's access with them, so the choice is left to you: remove
    /// the other members, or delete the vaults.
    ///
    /// Your email must be typed back to confirm, and a 2FA code is required when
    /// 2FA is enabled.
    DeleteAccount {
        /// Read the confirmation email from stdin instead of prompting.
        ///
        /// For the rare scripted case. ⚠️ There is no `--yes`: a flag that
        /// deletes an account is one typo in a shell history away from doing it,
        /// and the whole point of the typed confirmation is that it cannot be
        /// supplied by accident.
        #[arg(long)]
        confirm_stdin: bool,
    },
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

        /// Expire after this many days, 1 to 3650. Omit for a token that never expires.
        #[arg(long, value_name = "DAYS", value_parser = crate::core::args::parse_days, allow_hyphen_values = true)]
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

/// Subcommands for `evnx auth devices`.
#[cfg(feature = "cloud")]
#[derive(Subcommand, Debug)]
pub enum DeviceCommands {
    /// List the networks and browsers this account has signed in from.
    List,

    /// Report that a device was not you.
    ///
    /// Records the judgement, signs out **every** session including this one,
    /// and tells you to change your master password — which is the part that
    /// actually protects your vaults.
    Disavow {
        /// Device id, from `evnx auth devices list`.
        device_id: String,

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
        /// Vault name — lower-case letters, digits and hyphens.
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
        ///
        /// ⚠️ `--user` is accepted too. `share` took `--with` while `revoke` and
        /// `role` took `--user` for the same thing — the member's email — so
        /// whichever you learned first was wrong half the time. Both work
        /// everywhere now; neither is deprecated, because there is no reason to
        /// break the muscle memory of anyone who learned either one.
        #[arg(long, visible_alias = "user")]
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
        ///
        /// `--with` is accepted too — see `vault share`.
        #[arg(long, visible_alias = "with")]
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
        ///
        /// `--with` is accepted too — see `vault share`.
        #[arg(long, visible_alias = "with")]
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

    /// Rotate a vault's key without removing anyone.
    ///
    /// Re-encrypts every version under a fresh key and re-wraps it for every
    /// current member. Nobody loses access and nobody is signed out — members
    /// pick up the new key on their next command.
    ///
    /// Use it when a key may have been exposed but the team has not changed: a
    /// lost laptop with a signed-in CLI, a compromised CI runner that held the
    /// master password, an account breach where the person stays.
    ///
    /// ⚠️ This fixes the future, not the past. Anyone who could already read a
    /// version may still hold a copy of it, and no rotation recalls that. Rotate
    /// those secrets at their source.
    ///
    /// Removing a member rotates the key too — that is `vault revoke`.
    Rekey {
        /// Vault: `name`, `name/environment`, or an id.
        target: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

    /// Leave a vault that was shared with you.
    ///
    /// Deletes your copy of its key from the server. Anything you already
    /// pulled stays on this machine, and the vault key is not rotated — someone
    /// who stays can run `vault rekey`. The owner cannot leave; delete the vault
    /// instead.
    Leave {
        /// Vault: `name`, `name/environment`, or an id.
        target: String,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
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

        /// Push the default `.env` into a production vault without asking.
        ///
        /// Only that case asks: a file named with --file or --env-name never does.
        #[arg(long, short = 'y')]
        yes: bool,
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
        ///
        /// `-` writes it to stdout instead, and no file is created; the status
        /// lines go to stderr, so stdout holds only the `.env` text.
        #[arg(long, short)]
        file: Option<std::path::PathBuf>,

        /// Use `.env.<NAME>` instead of the derived or default file.
        ///
        /// A name whose file does not exist is an error listing the ones that do.
        #[arg(long, value_name = "NAME", conflicts_with = "file")]
        env_name: Option<String>,

        /// Version to pull. Defaults to the latest.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(i32).range(1..))]
        version: Option<i32>,

        /// Overwrite the target without asking.
        #[arg(long)]
        force: bool,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,

        /// Add the pulled file to .gitignore without asking (no prompt).
        #[arg(long, conflicts_with = "no_gitignore")]
        gitignore: bool,

        /// Skip the .gitignore check for the pulled file.
        #[arg(long, conflicts_with = "gitignore")]
        no_gitignore: bool,
    },

    /// Run a command with a vault's variables in its environment.
    ///
    /// The secrets are decrypted in memory and handed to the child process.
    /// Nothing is written to disk, nothing enters your shell history, and
    /// nothing appears in `ps` — values travel in the environment block, never
    /// in the argument vector.
    ///
    /// ⚠️ This removes the plaintext *file*, not the master password. The vault
    /// key is wrapped under your master key, so decryption needs it wherever
    /// this runs; in CI, feed it with --password-stdin.
    ///
    /// The command runs directly, not through a shell. For a pipeline or a
    /// shell builtin, ask for one: `-- sh -c 'a | b'`.
    ///
    /// `--version` here means the vault version, so clap's auto-generated
    /// `--version` is disabled on this subcommand, as it is on `pull`. Use
    /// `evnx --version` for the program version.
    #[command(disable_version_flag = true)]
    Run {
        /// Vault to read: `name`, `name/environment`, or an id.
        ///
        /// Optional in a directory bound with `evnx cloud link`.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Version to inject. Defaults to the latest.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(i32).range(1..))]
        version: Option<i32>,

        /// Inject only variables whose name matches one of these globs.
        ///
        /// Example: --include "DB_*,AWS_*"
        ///
        /// Least privilege for a subprocess: a build step that needs only
        /// `NEXT_PUBLIC_*` has no reason to be handed your database password.
        #[arg(long, value_delimiter = ',', value_name = "GLOB")]
        include: Option<Vec<String>>,

        /// Skip variables whose name matches one of these globs.
        ///
        /// Example: --exclude "*_LOCAL,*_TEST"
        ///
        /// Applied after --include, so a name matching both is skipped.
        #[arg(long, value_delimiter = ',', value_name = "GLOB")]
        exclude: Option<Vec<String>>,

        /// Read the master password from stdin instead of prompting.
        ///
        /// Only the first line is consumed; the child inherits the rest of
        /// stdin.
        #[arg(long)]
        password_stdin: bool,

        /// Inject variables that make a program load or run other code.
        ///
        /// `PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `BASH_ENV` and similar are
        /// refused by default: anyone who can push to the vault could use them
        /// to run code in your process. Pass this only if the vault is meant to
        /// set them.
        #[arg(long)]
        allow_unsafe_names: bool,

        /// Mask secret values in the command's output as `***`.
        ///
        /// On by default when output is not a terminal (a CI log, an agent),
        /// where what is printed is kept. Only values the classifier calls
        /// secret (by name or by value) are masked. It prevents accidents, not
        /// exfiltration: a command that transforms a value gets past it.
        #[arg(long, conflicts_with = "no_mask")]
        mask: bool,

        /// Never mask the command's output.
        #[arg(long)]
        no_mask: bool,

        /// The command to run, after `--`.
        ///
        /// `last = true` makes the separator mandatory, so a flag meant for the
        /// child — `-- npm run build --prod` — cannot be mistaken for one of
        /// evnx's own.
        #[arg(
            last = true,
            required = true,
            allow_hyphen_values = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
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

    /// Watch a vault and report when someone pushes a new version.
    ///
    /// Runs in the foreground until Ctrl-C, like `tail -f`.
    ///
    /// ⚠️ **It never pulls.** Two reasons: overwriting your `.env` because a
    /// teammate pushed would destroy local edits with no undo, and pulling needs
    /// the master password — a watcher that auto-pulled would hold the vault key
    /// in memory for hours. Taking a change stays a deliberate `evnx cloud pull`.
    ///
    /// Polls metadata only: no password, no decryption, no blob download. What
    /// it buys you is finding out a teammate changed a secret **before** your
    /// next deploy picks it up silently.
    Watch {
        /// Vault to watch. Optional in a bound directory.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Seconds between polls. Minimum 5.
        #[arg(long, value_name = "SECONDS", default_value_t = 15)]
        interval: u64,

        /// Check once and exit instead of looping. For scripts and tests.
        #[arg(long)]
        once: bool,

        /// Report your own pushes too. By default only someone else's are shown.
        #[arg(long)]
        include_mine: bool,
    },

    /// What changed between two versions — key names only, never values.
    ///
    /// ⚠️ **`+` here means "added in the newer version"**, which is the opposite
    /// of `evnx diff`, where `+` means "missing from .env". The direction is
    /// printed above the list so the two cannot be confused.
    ///
    /// Defaults to the two newest versions, which is usually the question:
    /// what did that last push change?
    ///
    /// ⚠️ **A changed value is invisible to this command.** The server holds key
    /// names and ciphertext and has never held a value, so rotating a secret
    /// without renaming it produces two versions with identical key sets. When
    /// that happens the blob hashes are compared and the answer says so, rather
    /// than reporting "no changes".
    ///
    /// Read-only: one request, no blob download, no password. `--local`
    /// compares the local file with a version instead, values included; that
    /// needs the master password and decrypts in memory.
    ///
    /// `--version` here means the vault version (with `--local`), so clap's
    /// auto-generated `--version` is disabled; `evnx --version` still works.
    #[command(disable_version_flag = true)]
    Diff {
        /// Vault to inspect. Optional in a bound directory.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Older version number. Defaults to the second-newest.
        #[arg(long, value_name = "N")]
        from: Option<i32>,

        /// Newer version number. Defaults to the newest.
        #[arg(long, value_name = "N")]
        to: Option<i32>,

        /// Compare the local file with a version instead: key names added,
        /// removed and changed, never values. Exit 0 when they match, 1 when
        /// they differ.
        ///
        /// Decrypted in memory, so it needs the master password; nothing is
        /// written. The file is chosen as `cloud pull` chooses it (`--file`,
        /// `--env-name`, `[cloud] file`, then `.env`).
        #[arg(long, conflicts_with_all = ["from", "to"])]
        local: bool,

        /// With --local: the version to compare with. Defaults to the newest.
        #[arg(long, value_name = "N", requires = "local", value_parser = clap::value_parser!(i32).range(1..))]
        version: Option<i32>,

        /// With --local: the local file.
        #[arg(long, short = 'f', value_name = "FILE", requires = "local")]
        file: Option<std::path::PathBuf>,

        /// With --local: use `.env.<NAME>`.
        #[arg(long, value_name = "NAME", requires = "local", conflicts_with = "file")]
        env_name: Option<String>,

        /// With --local: read the master password from stdin.
        #[arg(long, requires = "local")]
        password_stdin: bool,
    },

    /// Who did what to a vault, newest first.
    ///
    /// Pushes, pulls, member changes, re-keys and token creation. Any member can
    /// read it — the trail is how a team notices something it did not expect.
    ///
    /// ⚠️ **`--last` filters what the server already returned**, and the server
    /// trims by plan first. On a plan with seven days of visibility,
    /// `--last 30d` still shows seven; the output says so rather than implying
    /// the vault was quiet.
    ///
    /// ⚠️ Retention is a **visibility** limit, not deletion. The events table is
    /// append-only and nothing prunes it; the plan decides how far back the API
    /// will show.
    ///
    /// Read-only: no password, no decryption.
    Audit {
        /// Vault to inspect. Optional in a bound directory.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// How far back to show: `7d`, `30d`, or `all`.
        #[arg(long, value_name = "WINDOW", default_value = "all")]
        last: String,

        /// Show only this event type, e.g. `pull`.
        #[arg(long, value_name = "TYPE")]
        event: Option<String>,
    },

    /// Delete one version of a vault, and its encrypted blob.
    ///
    /// ⚠️ Permanent. The blob is removed from storage and the server has never
    /// held anything that could rebuild it.
    ///
    /// **The latest version cannot be deleted.** `evnx cloud pull` and `evnx
    /// cloud run` fetch it, so removing it would silently change what every
    /// consumer of the vault receives. Push a newer version first, or delete the
    /// whole vault.
    ///
    /// Needs admin on the vault: a developer can push new versions but not
    /// remove old ones. Use `evnx cloud history` to see what a vault holds.
    DeleteVersion {
        /// Version number to delete, as `evnx cloud history` shows it.
        ///
        /// ⚠️ Named `version_num` rather than `version` on purpose: clap derives an
        /// argument name from the field, and `version` collides with the
        /// auto-generated `--version` flag. `value_name` keeps the help reading
        /// `<VERSION>`. Caught by `cli_definition_is_valid`, which exists because
        /// this same collision once shipped and only appeared at runtime.
        #[arg(value_name = "VERSION", value_parser = clap::value_parser!(i32).range(1..))]
        version_num: i32,

        /// Vault to act on. Optional in a bound directory.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Bind this directory to a vault, so push and pull need no --vault.
    ///
    /// Writes `[cloud] vault` into .evnx.toml, preserving everything else in
    /// the file. Safe to commit: a vault name is not a secret.
    Link {
        /// Vault to bind: `name`, `name/environment`, or an id.
        vault: String,

        /// The file push and pull use with this vault, from any folder below
        /// the binding (`--file deploy/.env`). Stored as `[cloud] file`,
        /// relative to the folder the `.evnx.toml` is in, so it names the
        /// same file wherever the command runs. `--file` and `--env-name` on
        /// push and pull still win.
        #[arg(long, short = 'f', value_name = "FILE")]
        file: Option<std::path::PathBuf>,
    },

    /// Remove this directory's vault binding.
    Unlink,

    /// Write a vault's secrets to a directory, decrypted — the way out.
    ///
    /// ⚠️ This is the one evnx command whose purpose is to put plaintext
    /// secrets on a filesystem. Everything else works to avoid that: `cloud
    /// pull` writes a single file and asks first, and `cloud run` writes none
    /// at all. Use this to leave, to archive, or to prove you can — not as part
    /// of a pipeline.
    ///
    /// With no --to the files are the bytes that were pushed, verbatim:
    /// comments, ordering and quoting all survive, because that is how they
    /// were encrypted. --to converts instead, through the same 14 formats
    /// `evnx convert` offers, and a conversion keeps only keys and values.
    ///
    /// ⚠️ Not the same as `evnx auth download-data`, which writes account
    /// metadata and contains no secrets at all.
    Export {
        /// Vault to export: `name`, `name/environment`, or an id.
        ///
        /// Optional in a directory bound with `evnx cloud link`.
        #[arg(long, value_name = "VAULT")]
        vault: Option<String>,

        /// Directory to write into. Defaults to ./evnx-export
        ///
        /// A per-vault subdirectory is created inside it, so exporting two
        /// vaults into one place does not mix them up.
        ///
        /// ⚠️ Point this outside your working tree. An export inside a
        /// repository is one `git add .` away from being committed.
        #[arg(long, short, value_name = "DIR")]
        output: Option<std::path::PathBuf>,

        /// Export every version, not just the latest.
        ///
        /// Files are named by version — v1.env, v2.env — and manifest.json
        /// records which one was current.
        #[arg(long)]
        all_versions: bool,

        /// Convert to one of `evnx convert`'s formats instead of writing the
        /// file as it was pushed.
        ///
        /// Supported: json, yaml, shell, aws-secrets, gcp-secrets,
        /// azure-keyvault, github-actions, docker-compose, kubernetes,
        /// terraform, doppler, heroku, vercel, railway
        #[arg(long, value_name = "FORMAT")]
        to: Option<String>,

        /// Google Cloud project for --to gcp-secrets (required there)
        #[arg(long, requires = "to", value_name = "PROJECT")]
        project: Option<String>,

        /// Azure Key Vault for --to azure-keyvault (required there)
        #[arg(long, requires = "to", value_name = "NAME")]
        vault_name: Option<String>,

        /// Heroku app for --to heroku (required there)
        #[arg(long, requires = "to", value_name = "APP")]
        heroku_app: Option<String>,

        /// Name of the Kubernetes Secret for --to kubernetes (required there)
        #[arg(long, requires = "to", value_name = "NAME")]
        name: Option<String>,

        /// Kubernetes namespace for --to kubernetes (optional)
        #[arg(long, requires = "to", value_name = "NAMESPACE")]
        namespace: Option<String>,

        /// With --to: when two names become one in the target, keep the last
        /// value instead of refusing.
        #[arg(long, requires = "to")]
        allow_collisions: bool,

        /// Overwrite a directory that already holds an export.
        ///
        /// Without it a non-empty directory is refused before the master
        /// password is asked for, so the refusal costs nothing.
        #[arg(long)]
        force: bool,

        /// Read the master password from stdin instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },

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
// ⚠️ Shown only by a build that is actually missing something, so the prebuilt
// binaries — which are all `--all-features` — never carry it.
//
// `backup`, `restore`, `migrate`, `auth`, `vault` and `cloud` are `#[cfg]`-gated
// out of the `Commands` enum, so a default build answers `evnx backup` with
// clap's bare "unrecognized subcommand" and no hint at all. Each command module
// contains a "feature not enabled" stub written for exactly this case, and all
// three are unreachable: there is no variant left to dispatch to.
//
// It matters because `cargo install evnx` is the only channel that builds with
// `default = []`, and it is the canonical one.
//
// ⚠️ One cargo line, `--all-features` (DEC-SF11): a line per feature told
// someone missing `cloud` to rebuild without `migrate` and `backup`. The table
// names what each feature adds; `evnx commands` says which this build has.
#[cfg_attr(
    not(all(feature = "backup", feature = "migrate", feature = "cloud")),
    command(after_help = "\
This binary was built without some optional features (`evnx commands` lists
what it has). What each one adds:
  backup     backup, restore
  migrate    migrate
  cloud      auth, org, vault, cloud

`cargo install evnx` compiles with no features. For all of them:
  cargo install evnx --all-features

Every prebuilt binary — Homebrew, Scoop, winget, npm, PyPI and the GitHub
Release — already includes all of them.")
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Print nothing on success. Findings, warnings, errors and requested data still print.
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
    /// Interactive project setup — the template, .env and the .evnx.toml contract.
    ///
    /// Three modes available:
    /// • Blank: Create empty .env files (add variables later with `evnx add`)
    /// • Blueprint: Use a pre-configured stack like "T3 Turbo" or "Rust High-Perf"
    /// • Architect: Build your stack step-by-step (language → framework → services → infra)
    ///
    /// Writes into the template the project uses (`[defaults] example`, else
    /// .env.example, .env.sample or .env.template). `.env` is never replaced:
    /// names it lacks are appended. `.evnx.toml` gets a [vars] entry per
    /// variable, or the new names are recorded into the one in force.
    #[command(after_help = docs::INIT.after_help)]
    Init {
        /// Output path for .env files.
        // `-p` matches `evnx add`, and the documentation showed it for `init`
        // before it existed (I-023).
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Skip all prompts. Creates blank .env files unless --blueprint names a stack.
        #[arg(short, long)]
        yes: bool,

        /// Generate from a named stack blueprint, e.g. `t3_modern`.
        ///
        /// Selects Blueprint mode without prompting. An unknown id is an error
        /// that lists the valid ones, so this doubles as the way to see them.
        ///
        /// A blueprint is a shorthand for a component list — `t3_modern` is
        /// `--with nextjs,postgresql,clerk,aws_s3,stripe,github_actions,vercel`.
        #[arg(long, value_name = "ID", conflicts_with = "with")]
        blueprint: Option<String>,

        /// Name the components directly, e.g. `--with nextjs,postgresql,stripe`.
        ///
        /// Ten fixed blueprints cannot span the stacks people have — "Django +
        /// Kafka + Clerk" needed someone to add a `django_kafka_clerk` stack
        /// before it could be expressed. This names the pieces instead.
        ///
        /// Repeatable and comma-separated: `--with a,b --with c` is the same as
        /// `--with a,b,c`. An unknown name is an error that suggests near
        /// matches; `--list-components` prints them all.
        #[arg(long, value_name = "LIST", value_delimiter = ',', num_args = 1..)]
        with: Vec<String>,

        /// Print every component `--with` accepts, and exit.
        #[arg(long, conflicts_with_all = ["with", "blueprint", "yes"])]
        list_components: bool,

        /// Accept what the project declares, without asking.
        ///
        /// Run interactively with no flags, `init` reads `package.json`,
        /// `docker-compose.yml` and friends and proposes what it finds. This is
        /// how a script says yes to that proposal.
        ///
        /// ⚠️ Detection is a heuristic — a `stripe` dependency in a test fixture
        /// looks exactly like one in a checkout flow. Prefer `--with` where the
        /// answer is known; this is for bootstrapping a project you trust.
        // ⚠️ No longer conflicts with `--yes`. `--detect` already implies it —
        // `init/mod.rs` passes `yes: yes || detect_flag`, because accepting the
        // proposal without asking is what detection is for. Rejecting the
        // combination meant a CI script that passes `--yes` out of habit got an
        // argument error for asking twice for the same thing.
        #[arg(long, conflicts_with_all = ["with", "blueprint"])]
        detect: bool,

        /// Build the template from the variables the code actually reads.
        ///
        /// Scans the project's source and compares it to `.env.example`, so it
        /// finds both what is missing and what is declared but named nowhere —
        /// the second being something no other evnx command can tell you, since
        /// they all treat `.env.example` as the definition of "required".
        ///
        /// ⚠️ A heuristic. Variables consumed entirely inside a library, and
        /// dynamic access like `process.env[key]`, are invisible in principle.
        ///
        /// Only ever appends, so it cannot be combined with --force.
        // `force` used to be accepted here and silently ignored (I-012).
        #[arg(long, conflicts_with_all = ["with", "blueprint", "detect", "list_components", "force"])]
        from_source: bool,

        /// Overwrite an existing .env.example.
        ///
        /// `.env.example` is normally a tracked, hand-maintained file, so init
        /// asks before replacing it and refuses outright under --yes. This is how
        /// a script says it meant to replace it. `.env` is never overwritten.
        /// Not valid with --from-source, which only appends.
        #[arg(long)]
        force: bool,

        /// Write `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` creates or fills `.env.production` from the
        /// template; the template and `.gitignore` are the project's, as without
        /// it. Defaults to `[defaults] env_name` from `.evnx.toml`.
        #[arg(long, value_name = "NAME")]
        env_name: Option<String>,

        /// Show what would be written — the template, `.env`, `.gitignore` and
        /// `.evnx.toml` — and write nothing. Asks no confirmation; a menu is
        /// still shown when nothing was named.
        #[arg(long)]
        dry_run: bool,

        /// Print the result as one JSON document on stdout (needs --yes or
        /// --dry-run: it never asks). The same fields as `evnx add --json`.
        #[arg(long)]
        json: bool,
    },

    /// Add environment variables to existing .env files.
    ///
    /// Subcommands:
    /// • service (or infra): Add vars for any catalogue id (e.g., postgresql, vercel)
    /// • framework: Add vars for a framework (e.g., django)
    /// • blueprint: Add vars from a stack blueprint
    /// • custom: Interactive custom variable addition
    ///
    /// Exit codes: 0 when the variables were added, were already there, or
    /// the run was a --dry-run or answered no; 2 when it could not run
    /// (unknown id, bad argument, no terminal to ask, a write error).
    // ⚠️ Not `args_conflicts_with_subcommands`: that also refuses every flag
    // written before the subcommand (`evnx add --path api service redis`),
    // which has always worked. `--list` with a subcommand is refused in `run`.
    #[command(after_help = docs::ADD.after_help, arg_required_else_help = true)]
    Add {
        #[command(subcommand)]
        target: Option<AddTarget>,

        /// List every id `add` takes: services, infrastructure, frameworks
        /// and blueprints.
        #[arg(long)]
        list: bool,

        /// Show what would be added, skipped and kept; write nothing. Needs no
        /// terminal.
        #[arg(long, global = true)]
        dry_run: bool,

        /// Print the result as one JSON document on stdout (needs --yes or
        /// --dry-run: it never asks).
        #[arg(long, global = true)]
        json: bool,

        /// Output path (default: current directory).
        #[arg(long, short, default_value = ".", global = true)]
        path: String,

        /// Write to `.env.<NAME>` instead of `.env` (one file per run; loop
        /// over names for several). Defaults to `[defaults] env_name`.
        #[arg(long, value_name = "NAME", global = true)]
        env_name: Option<String>,

        /// Skip confirmation prompts.
        #[arg(long, short, global = true)]
        yes: bool,
    },

    /// Check .env against .env.example, find issues.
    #[command(after_help = docs::VALIDATE.after_help)]
    Validate {
        /// Path to the environment file to check.
        ///
        /// Defaults to `.env`, or `[defaults] env_name` from `.evnx.toml`.
        #[arg(long)]
        env: Option<String>,

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

        /// The template to compare against.
        ///
        /// Defaults to `.env.example`, or `[defaults] example` from `.evnx.toml`.
        #[arg(long)]
        example: Option<String>,

        /// Also report variables that are present but not declared.
        ///
        /// Measured against `.env.example`, or against `[vars]` when the project
        /// has a contract. Without it, an undeclared variable is not mentioned.
        #[arg(long)]
        strict: bool,

        /// Repair what can be repaired, instead of only reporting it.
        ///
        /// Fills missing variables, replaces placeholder values and corrects
        /// boolean traps. Only `.env` is written, the template is never touched,
        /// and the original is kept beside it as `.env.bak`.
        ///
        /// A missing variable gets the template's value. A new secret is
        /// generated only for one the app holds alone — its own signing key
        /// (`SECRET_KEY`, `JWT_SECRET`, `SESSION_SECRET`, …) or a password; a
        /// key a provider issues (`STRIPE_KEY`, an AWS secret, a token) gets a
        /// placeholder, because evnx cannot invent it. Those are listed under
        /// "Needs your input", not as fixed, and the run exits 1 until they are
        /// filled in.
        ///
        /// ⛔ **It will not overwrite a value that is not a placeholder.** A
        /// weak secret with a real value behind it is reported and left alone —
        /// see `--rotate-weak-secrets`.
        #[arg(long)]
        fix: bool,

        /// With `--fix`, show what it would change and write nothing.
        ///
        /// No `.env` change and no backup. The report and the exit code are
        /// those of the file as it stands; a secret `--fix` would generate is
        /// not shown, because the real run makes a new one.
        #[arg(long, requires = "fix")]
        dry_run: bool,

        /// With `--fix`, also replace weak secrets that hold a **real value**.
        ///
        /// ⛔ **This destroys credentials, and that is what it is for.** Until
        /// 2026-10-07 `--fix` did it unasked: `DATABASE_PASSWORD="p@ss word#1"`
        /// became `DATABASE_PASSWORD=ad0da713…c508`, and the run reported "All
        /// checks passed".
        ///
        /// The check behind it judges length and wording. It cannot tell a
        /// placeholder nobody filled in from the real password, short because
        /// someone chose a short one — and the second is the common case,
        /// because shortness is what trips it. Whether anything *else* holds
        /// the credential is not a fact in the file, so evnx cannot decide it;
        /// you can.
        ///
        /// Rotate at the provider first. A new value here that the provider
        /// does not have is an outage, not a repair — so a key a provider
        /// issues is never replaced, even with this flag: only the app's own
        /// secrets and passwords are.
        #[arg(long, requires = "fix")]
        rotate_weak_secrets: bool,

        /// Output format: `pretty`, `json` or `github-actions`.
        ///
        /// `json` is the machine-readable form, with a `summary` object for
        /// `jq`. `github-actions` emits `::error` annotations that appear
        /// inline on a pull request. An unrecognised value is an error.
        #[arg(long, default_value = "pretty")]
        format: String,

        /// Always exit 0, even when problems are found.
        ///
        /// For a step that should report without failing the build. It
        /// suppresses the *findings* exit code only — a file that cannot be read
        /// still exits 2, because "I found nothing" and "I could not look" are
        /// different answers.
        #[arg(long)]
        exit_zero: bool,

        /// Comma-separated list of issue types to ignore
        ///
        /// Added to `[validate] ignore` in `.evnx.toml`. An unknown type is an
        /// error that lists the valid ones, so a typo cannot silence nothing.
        #[arg(long, value_delimiter = ',')]
        ignore: Vec<String>,

        /// Validate value formats: url, port, email
        #[arg(long)]
        validate_formats: bool,

        /// The lowest severity that fails the run: `error` or `warning`.
        ///
        /// Defaults to `error`, or `[validate] fail_on` from `.evnx.toml`.
        /// `warning` makes warnings exit 1 too, for gates that want them
        /// fixed. `--exit-zero` still wins.
        #[arg(long, value_name = "LEVEL")]
        fail_on: Option<String>,
    },

    /// Detect secrets that look real (AWS keys, tokens, etc.).
    #[command(after_help = docs::SCAN.after_help)]
    Scan {
        /// Files or folders to scan. Repeatable; defaults to the current folder.
        ///
        /// A folder is walked. A file named here is always read — whatever
        /// `--exclude`, `--extensions` or the size limit say — and always
        /// reported in full, even one git ignores. `-` reads stdin, in memory
        /// (`git diff | evnx scan -`): as a `.env` when it parses as one,
        /// otherwise as text. Empty stdin is an error, not a clean result.
        #[arg(default_value = ".")]
        path: Vec<String>,

        /// Skip paths in a folder walk. Repeatable; adds to `[scan] exclude`.
        ///
        /// A value with `*`, `?` or `[` is a glob, matched against the path
        /// from the folder and against the file name: `'fixtures/**'`,
        /// `'*.log'`. Any other value matches a whole path component, or
        /// several in a row: `--exclude test` skips `test/` and a file named
        /// `test`, not `latest/` or `contest.py`; `vendor/cache` skips that
        /// folder wherever it is.
        ///
        /// ⚠️ This narrows the scan. `.git`, `node_modules`, `target`, `dist`,
        /// `build` and the `.env.example` family are skipped whatever this
        /// says, and a file named on the command line is read regardless.
        #[arg(long, default_value = ".env.example")]
        exclude: Vec<String>,

        /// Skip walked files larger than this: `500K`, `5MB`, `2MiB`.
        ///
        /// Defaults to 1 MiB, or `[scan] max_file_size` from `.evnx.toml`. A
        /// skipped file is counted and, with `-v`, listed. A file named on the
        /// command line is read whatever its size.
        #[arg(long, value_name = "SIZE", value_parser = crate::core::args::parse_size, allow_hyphen_values = true)]
        max_file_size: Option<u64>,

        /// Read only files with these extensions in folders (comma-separated:
        /// `env,yml,json`). By default every file is read, and binary or very
        /// large files are skipped with a note.
        ///
        /// ⚠️ This narrows the scan, and the run says so. A file named on the
        /// command line is read whatever this says. `[scan] extensions` in
        /// `.evnx.toml` sets the same thing for a project.
        #[arg(long, value_delimiter = ',', value_name = "EXT")]
        extensions: Option<Vec<String>>,

        /// Also report values matching this regular expression. Repeatable.
        ///
        /// For the formats evnx does not ship with — an internal token, a
        /// vendor key shape. `--pattern 'ACME-[A-Z0-9]{32}'` reports matches as
        /// `Custom pattern 1`, numbered in the order the flags are given.
        ///
        /// It takes an expression and nothing else. For a name that means
        /// something, and for a rule the whole team scans with, declare it under
        /// `[[scan.patterns]]` in `.evnx.toml` instead; both sets apply
        /// together.
        ///
        /// An expression that does not compile exits 2 — no verdict — rather
        /// than reporting a clean scan it never ran.
        #[arg(long, value_name = "REGEX")]
        pattern: Vec<String>,

        /// Skip values evnx recognises as filler.
        ///
        /// `changeme`, `your_api_key_here`, `xxx` and similar. Useful on a first
        /// run over a template-heavy repository; prefer `--severity high` if the
        /// goal is simply less noise.
        #[arg(long, overrides_with = "no_ignore_placeholders")]
        ignore_placeholders: bool,

        /// Report filler-looking values even when `[scan] ignore_placeholders`
        /// is `true`. The last of the two flags wins.
        #[arg(long, overrides_with = "ignore_placeholders")]
        no_ignore_placeholders: bool,

        /// Lowest confidence worth reporting: high, medium or low.
        ///
        /// Filters what is reported, counted and exited on — all three describe
        /// the same set. `--severity high` keeps values that matched a format —
        /// a provider's, or one this project declared under `[[scan.patterns]]`
        /// — and long values in a variable named like a secret (`JWT_SECRET`,
        /// `DB_PASSWORD`), so a committed one still fails the gate. It is the
        /// usual choice for a blocking CI gate; the default reports everything.
        /// Defaults to `low` (report everything), or `[scan] severity` from
        /// `.evnx.toml` when that is set.
        #[arg(long, value_name = "LEVEL")]
        severity: Option<String>,

        /// Output format: `pretty`, `json`, `sarif` or `github`.
        ///
        /// `sarif` uploads to GitHub's Security tab; `github` emits `::error`
        /// annotations inline on a pull request; `json` carries a `summary`
        /// object for `jq`. An unrecognised value is an error, not a silent
        /// fall back to `pretty`.
        #[arg(long, default_value = "pretty")]
        format: String,

        /// Always exit 0, even when secrets are found.
        ///
        /// ⚠️ Suppresses exit 1 only. A scan that could not run still exits 2 —
        /// the flag means "do not fail my build over findings", not "never tell
        /// me the scan was impossible".
        #[arg(long)]
        exit_zero: bool,

        /// Report findings in files git ignores and does not track.
        ///
        /// In a git repository a folder walk counts what git would commit: a
        /// filled-in `.env` that `.gitignore` covers is where secrets belong, so
        /// its findings are summarised in one line and do not fail the run. This
        /// lists them as findings instead. A file named on the command line, and
        /// any folder outside a repository, is always reported in full.
        #[arg(long)]
        include_ignored: bool,
    },

    /// Compare two env files: keys missing or extra, and values between environments.
    ///
    /// Against a template (`.env.example`) only key names are compared; between
    /// two environments (`--against`) values are compared too.
    //
    // ⚠️ The first line said "Compare .env vs .env.example — show missing/extra
    // vars", and the command compared values against the template as well, so
    // a filled-in `.env` exited 1 on every real project (I-078). The rule is
    // in `after_help` (`docs::DIFF`), where clap prints it as written.
    #[command(after_help = docs::DIFF.after_help)]
    Diff {
        /// The environment file on the left of the comparison.
        ///
        /// Defaults to `.env`, or `[defaults] env_name` from `.evnx.toml`.
        #[arg(long)]
        env: Option<String>,

        /// The template on the right of the comparison: key names only.
        ///
        /// Defaults to `.env.example`, or `[defaults] example`. A template holds
        /// placeholders, so its values are not compared unless `--values` is
        /// given. To compare two real environments use `--against`.
        #[arg(long)]
        example: Option<String>,

        /// Operate on `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` resolves to `.env.production`. A name whose
        /// file does not exist is an error listing the environments that do —
        /// it never falls back to `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

        /// Compare against the environment `.env.<NAME>`: keys and values.
        ///
        /// `evnx diff --env-name production --against staging` lists the keys
        /// one environment lacks and the keys whose values differ (values are
        /// shown only with `--show-values`).
        #[arg(long, value_name = "NAME", conflicts_with = "example")]
        against: Option<String>,

        /// Compare values against a template too, not only key names.
        ///
        /// Every filled-in value differs from its placeholder, so expect each
        /// one to be listed. `--against` compares values without it.
        #[arg(long)]
        values: bool,

        /// Print the values, not just which keys differ.
        ///
        /// In `pretty` and `json` a secret stays masked: by its name, or by the
        /// value itself (`postgres://admin:***@db`). A `patch` that carries
        /// real values (`--values`, `--against`, `--reverse`) needs this flag,
        /// and then has the exact values so it applies; `--interactive` shows
        /// `<withheld>` in its questions without it.
        ///
        /// ⚠️ With `patch` and `--interactive` this puts real secrets on your
        /// terminal and into its scrollback, and into any CI log.
        #[arg(long)]
        show_values: bool,

        /// Output format: `pretty`, `json` or `patch`.
        ///
        /// An unrecognised value is an error (exit 2), not a silent fall back
        /// to `pretty` (I-074). `patch` is a unified diff of the left file that
        /// `patch -p1` applies: it adds the missing names and, when values are
        /// compared, replaces the different ones. Keys only in the left file
        /// are kept and named in comment lines.
        #[arg(long, default_value = "pretty", value_parser = ["pretty", "json", "patch"])]
        format: String,

        /// Swap the two sides: compare the example against the environment.
        #[arg(long)]
        reverse: bool,

        /// Leave these keys out of the comparison (comma-separated, exact case).
        ///
        /// Added to `[diff] ignore_keys` from `.evnx.toml`. The summary says how
        /// many keys were ignored; `-v` lists names that matched nothing.
        #[arg(long, value_delimiter = ',', value_name = "KEYS")]
        ignore_keys: Vec<String>,

        /// What fails the run (exit 1): `missing`, `extra`, `different`.
        ///
        /// Comma-separated; all three by default. Only the exit code changes —
        /// the report lists everything. `--fail-on missing` fails only when the
        /// left file lacks a key the right one has. `different` needs values to
        /// be compared (`--against`, or `--values`).
        #[arg(
            long,
            value_delimiter = ',',
            value_name = "WHAT",
            value_parser = ["missing", "extra", "different"]
        )]
        fail_on: Vec<String>,

        /// With no `.env`, skip (exit 0) and say so.
        ///
        /// For pre-commit hooks: a fresh clone has a template and no `.env`, and
        /// a hook that exits 2 there blocks every new contributor's first
        /// commit. Only the left file is skipped; a missing template is still
        /// an error. ⚠️ Not for CI, where a check that never ran must not pass.
        #[arg(long)]
        skip_without_env: bool,

        /// Add a `stats` object to the JSON (`--format json` only).
        #[arg(long, default_value_t = false)]
        with_stats: bool,

        /// Prompt for each change and write the chosen ones (`--format patch`
        /// only).
        ///
        /// The previous file is kept as `.bak`. Needs a terminal. Not with
        /// `--reverse`, which would write `.env`'s values into the template.
        #[arg(short, long, default_value_t = false)]
        interactive: bool,
    },

    /// Transform `.env` files into multiple output formats.
    ///
    /// Supports 14 formats including JSON, YAML, cloud configs,
    /// CI/CD variables, and infrastructure-as-code.
    //
    // ⚠️ The examples live in `after_help` below, not here. clap strips the
    // markdown fence and collapses newlines in a doc comment, so a fenced block
    // renders as one unusable line with the backslash continuations inline and
    // the fence markers still visible. `after_help` is passed through verbatim.
    // (These are `//` comments on purpose: a `///` line here was printed by
    // `--help`, I-082.)
    #[command(after_help = "\
Examples:
  # Basic JSON conversion
  evnx convert --to json

  # Kubernetes secret with transformations
  evnx convert \\
    --to kubernetes --name myapp-env \\
    --include \"PROD_*\" \\
    --prefix \"MYAPP_\" \\
    --transform uppercase \\
    --output k8s-secret.yaml

  # A script that creates or updates every secret in a GCP project
  evnx convert --to gcp-secrets --project my-gcp-project -o upload.sh

  # Interactive mode (opens the format selector)
  evnx convert

Supported formats:
  Generic:        json, yaml, shell
  Cloud:          aws-secrets (JSON for create-secret),
                  gcp-secrets (--project), azure-keyvault (--vault-name)
  CI/CD:          github-actions (text to paste into the GitHub web form)
  Containers:     docker-compose, kubernetes (--name, optional --namespace)
  IaC:            terraform
  Secret Managers: doppler (experimental), heroku (--heroku-app),
                  vercel (experimental), railway (experimental)

Aliases:
  k8s → kubernetes, tf → terraform, yml → yaml
  gh-actions → github-actions, compose → docker-compose

Exit codes: 0 converted; 2 nothing converted (bad flag, missing file, names that
collide, a filter or file with no variables).

Use 'evnx convert' without --to for interactive format selection.

📖  Full guide: https://docs.evnx.dev/cli/commands/convert
")]
    Convert {
        /// Path to input .env file
        #[arg(long, value_name = "PATH")]
        env: Option<String>,

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

        /// Replace FILE when it already exists (with --output).
        ///
        /// Without it an existing output is refused and nothing is written.
        /// The input file is never replaced, even with --force.
        #[arg(long, requires = "output")]
        force: bool,

        /// Add the --output file to .gitignore without asking (no prompt).
        ///
        /// The file holds every value from the .env. Without this flag evnx
        /// asks when there is a terminal, and warns when there is not.
        #[arg(long, requires = "output", conflicts_with = "no_gitignore")]
        gitignore: bool,

        /// Skip the .gitignore check for the --output file.
        #[arg(long, requires = "output", conflicts_with = "gitignore")]
        no_gitignore: bool,

        /// Include only variables whose name matches a glob pattern
        ///
        /// `*` matches any run of characters, `?` exactly one; case-sensitive.
        /// Repeat the flag or separate patterns with commas.
        /// Examples: "AWS_*", "*_URL", "DB_*_URL", "PORT,API_*"
        #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
        include: Vec<String>,

        /// Exclude variables whose name matches a glob pattern
        ///
        /// Same patterns as --include; exclude wins over include.
        /// Examples: "*_DEBUG", "TEST_*"
        #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
        exclude: Vec<String>,

        /// Keep only the secrets, or only the rest
        ///
        /// A variable is a secret when its name says so (`*_PASSWORD`,
        /// `*_SECRET`, `*_TOKEN`, …) or its value does (a provider key, a
        /// private key, a URL with a password). Applied with --include and
        /// --exclude. For example, public values to a build system and secrets
        /// to a secret manager.
        #[arg(long, value_enum, value_name = "WHICH")]
        only: Option<crate::core::converter::Only>,

        /// Convert even when no variable is left (an empty .env, or filters
        /// that match nothing). Without it that is an error, exit 2.
        #[arg(long)]
        allow_empty: bool,

        /// When two names become one name in the output, keep the last value.
        ///
        /// `--transform` can merge names (`Api` and `API` under uppercase), and
        /// so can a format's own naming: gcp-secrets lowercases and uses `-`,
        /// terraform lowercases, and Azure Key Vault and GitHub ignore case.
        /// Without this flag that is an error naming both, exit 2.
        #[arg(long)]
        allow_collisions: bool,

        /// Base64-encode all values before output.
        ///
        /// For `kubernetes` this chooses the field: with the flag values are
        /// base64 under `data:`; without it they are literal under
        /// `stringData:`, which Kubernetes encodes itself on apply.
        #[arg(long)]
        base64: bool,

        /// Prefix to prepend to all variable names
        ///
        /// Example: --prefix "APP_" transforms "KEY" → "APP_KEY"
        /// Applied before key casing transformation
        ///
        /// Letters, digits and `_`, not starting with a digit, so every name
        /// stays a variable name.
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

        /// Google Cloud project for --to gcp-secrets (required there)
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,

        /// Azure Key Vault for --to azure-keyvault (required there)
        #[arg(long, value_name = "NAME")]
        vault_name: Option<String>,

        /// Heroku app for --to heroku (required there)
        #[arg(long, value_name = "APP")]
        heroku_app: Option<String>,

        /// Name of the Kubernetes Secret for --to kubernetes (required there)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,

        /// Kubernetes namespace for --to kubernetes (optional)
        #[arg(long, value_name = "NAMESPACE")]
        namespace: Option<String>,
    },

    /// Move a .env into a secret manager: print the commands, or upload to GitHub.
    ///
    /// For eight destinations evnx prints the commands for you to run and
    /// contacts nothing; only github-actions uploads (over the GitHub API).
    // ⚠️ The first line said "Full migration workflow to secret managers", and
    // the banner "Generate destination commands" — true for eight of nine
    // (D-59). The examples were a `///` block, which clap folded into one line
    // (D-60, as convert's I-082); `after_help` is printed as written.
    #[cfg(feature = "migrate")]
    #[command(after_help = "\
Examples:
  # Print the commands, read them, then run them
  evnx migrate --to heroku --heroku-app my-app

  # Save only the commands, as a script
  evnx migrate --to gcp-secret-manager --project my-gcp-project --print-only > upload.sh

  # Another environment: .env.production
  evnx migrate --env-name production --to azure-keyvault --vault-name prod-vault

  # Upload to GitHub Actions from CI (token from GITHUB_TOKEN)
  evnx migrate --to github-actions --repo owner/repo --yes --skip-existing

  # Preview: no commands printed, nothing uploaded
  evnx migrate --to doppler --project myapp --doppler-config dev --dry-run

Destinations:
  Prints commands: aws-secrets-manager (--secret-name), gcp-secret-manager (--project),
                   azure-keyvault (--vault-name), heroku (--heroku-app), vercel,
                   doppler, infisical, railway
  Uploads:         github-actions (--repo; GITHUB_TOKEN)

Exit codes: 0 done; 1 some GitHub uploads failed; 2 could not run (bad flag, missing
file, names that collide, a filter that leaves nothing).

📖  Full guide: https://docs.evnx.dev/cli/commands/migrate
")]
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
        /// The template to render. `{{VAR}}`, `${VAR}` and `$VAR` are
        /// substituted; filters are listed below.
        #[arg(long)]
        input: String,

        /// Where to write the rendered file, or `-` for stdout.
        ///
        /// ⚠️ The result usually contains real values — see `--gitignore`. A
        /// new file is created readable by you only (0600); an existing one is
        /// replaced and keeps its mode. The template and the env file are never
        /// overwritten.
        #[arg(long)]
        output: String,

        /// File mode for the output, in octal (e.g. 0640). Applied to a new or
        /// an existing file. Without it a new file is 0600.
        #[arg(long, value_name = "MODE", value_parser = parse_mode)]
        mode: Option<u32>,

        /// Create the output's missing directories, readable by you only (0700).
        #[arg(long)]
        mkdir: bool,

        /// Render and report what would be written, and write nothing.
        ///
        /// Fails where the real run would (an unknown filter, an unresolved
        /// placeholder without `--allow-unresolved`, a missing directory without
        /// `--mkdir`).
        /// Never prints the rendered text, and leaves `.gitignore` alone.
        #[arg(long)]
        dry_run: bool,

        /// The environment file to take values from.
        ///
        /// Defaults to `.env`, or `[defaults] env_name` from `.evnx.toml`.
        #[arg(long)]
        env: Option<String>,

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

        /// Write an unresolved placeholder through literally and exit 0.
        ///
        /// Since 0.11.0 a placeholder with no value is an error: nothing is
        /// written, every unresolved name is listed, and the exit code is 2. An
        /// unresolved `{{DATABASE_URL}}` in a generated config is read by
        /// whatever runs next as the literal setting. Give a variable a fallback
        /// with `{{NAME|default:value}}` to keep it optional.
        #[arg(long, conflicts_with = "strict")]
        allow_unresolved: bool,

        /// The default since 0.11.0; accepted, and removed in 1.0.
        #[arg(long)]
        strict: bool,
    },

    /// Create encrypted backup of .env.
    #[cfg(feature = "backup")]
    #[command(after_help = docs::BACKUP.after_help)]
    Backup {
        /// Path to the .env file to back up.
        env: Option<String>,

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
        /// with leading and trailing whitespace trimmed; binary content is
        /// Base64-encoded first. It must come to at least 8 characters, as a
        /// typed passphrase must.
        ///
        /// Also accepted as --password-file, which is what `evnx restore` calls
        /// the same thing. EVNX_PASSWORD is honoured too, at lower priority,
        /// so a scheduled backup needs no passphrase on disk; when both are
        /// set, the key file wins and a warning says so.
        ///
        /// Examples:
        ///   evnx backup --key-file /run/secrets/evnx-pass
        ///   EVNX_PASSWORD=$PASS evnx backup
        #[arg(long, value_name = "PATH", visible_alias = "password-file")]
        key_file: Option<String>,

        /// Number of previous backups to retain alongside the new one.
        ///
        /// Once the new backup is written, existing ones shift up one place:
        /// <output> → <output>.1 → … → <output>.N. The one already at <output>.N
        /// is replaced by the shift (with a warning); files past it are never
        /// touched. Set to 0 to disable rotation (overwrite silently).
        /// Defaults to `3`, or `[backup] keep` from `.evnx.toml`.
        #[arg(long, value_name = "N")]
        keep: Option<u32>,

        /// Re-decrypt the backup after writing and verify content integrity.
        ///
        /// Costs one more Argon2id round (a fraction of a second to about a
        /// second, depending on the machine) but proves the backup is readable
        /// before you discard the source file. Exit code 6 on failure.
        #[arg(long)]
        verify: bool,
    },

    /// Restore from encrypted backup.
    #[cfg(feature = "backup")]
    #[command(after_help = docs::RESTORE.after_help)]
    Restore {
        /// The backup file to decrypt, as written by `evnx backup`.
        ///
        /// A file on disk. For a vault in evnx cloud, use `evnx cloud pull`.
        backup: String,

        /// Where to write the restored file.
        ///
        /// Defaults to the file the backup was made from — `.env.staging` for a
        /// backup of `.env.staging` — in the current folder (since 0.11.0; it
        /// was always `.env`). A recorded name that is not `.env` or `.env.<name>`
        /// restores to `.env`.
        ///
        /// An existing file is never replaced silently: evnx asks, or takes
        /// --yes, and keeps the replaced file as <output>.bak. Content that is
        /// not a .env evnx can read goes to <output>.restored instead (exit 5).
        #[arg(long)]
        output: Option<String>,

        /// Replace an existing --output without asking.
        ///
        /// For scripts and CI, where there is no terminal to ask on. The file
        /// it replaces is kept beside it as <output>.bak (.bak.2, … when one
        /// exists). Without it, and without a terminal, an existing output
        /// stops the run (exit 2). Also accepted as --force, as in `cloud pull`.
        #[arg(long, short = 'y', visible_alias = "force")]
        yes: bool,

        /// Add the restored file to .gitignore without asking (no prompt).
        #[arg(long, conflicts_with = "no_gitignore")]
        gitignore: bool,

        /// Skip the .gitignore check for the restored file.
        #[arg(long, conflicts_with = "gitignore")]
        no_gitignore: bool,

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
        /// The file should contain only the password; leading and trailing
        /// whitespace, such as a final newline, is trimmed. Intended for CI/CD
        /// pipelines where interactive prompts are not possible.
        ///
        /// Security: less secure than the interactive prompt. The file path
        /// may appear in process listings and shell history. Use a secrets
        /// manager or a tmpfs-backed path (e.g. /run/secrets/) in production.
        ///
        /// EVNX_PASSWORD environment variable is also accepted and takes
        /// lower priority than --password-file; when both are set, the file
        /// wins and a warning says so.
        ///
        /// Also accepted as --key-file, which is what `evnx backup` calls the
        /// same thing. Either spelling reads the file identically on both
        /// sides, so one file round-trips.
        ///
        /// Examples:
        ///   evnx restore .env.backup --password-file /run/secrets/evnx-pass
        ///   EVNX_PASSWORD=mypass evnx restore .env.backup
        #[arg(long, value_name = "PATH", visible_alias = "key-file")]
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

    /// Manage organisations — billing and a directory.
    ///
    /// ⛔ An organisation does **not** give anyone access to a vault. It owns a
    /// plan and a set of seats; a seat decides which plan's limits apply to the
    /// holder. Sharing a vault is `evnx vault share`, and only someone who holds
    /// the key can do it.
    #[cfg(feature = "cloud")]
    #[command(after_help = docs::ORG.after_help)]
    Org {
        #[command(subcommand)]
        command: OrgCommands,

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

        /// Project directory to analyze (same as the positional argument).
        ///
        /// Accepted because the documentation used this form for a long time
        /// before the positional existed in print. Giving both is an error
        /// rather than a silent preference.
        #[arg(long = "path", value_name = "PATH", conflicts_with = "path")]
        project_path: Option<String>,

        /// Repair what can be repaired, instead of only reporting it.
        ///
        /// Fixes are conservative and local: `.gitignore` gains the `.env*`
        /// entries `evnx init` writes, and every env file other users can read
        /// is set to 0600 — a symlinked one only when its target is inside the
        /// project. Every fix is checked again afterwards; one that did not
        /// work says "not fixed". Nothing is deleted, nothing is committed, and
        /// no file's contents are edited.
        ///
        /// ⚠️ `EVNX_AUTO_FIX=1` does the same thing and still works. Either
        /// saying yes is enough — a flag can turn repair on, never off.
        #[arg(long)]
        fix: bool,

        /// Count warnings as problems, so they affect the exit code.
        ///
        /// Without it, warnings are reported and exit 0 — a missing
        /// `.env.example` should not fail a build that never asked it to.
        #[arg(long)]
        strict: bool,

        /// With no env file, skip (exit 0) and say so.
        ///
        /// For pre-commit hooks: a fresh clone has a template and no `.env`,
        /// and doctor calls that an error, which would block every new
        /// contributor's first commit. ⚠️ Not for CI, where a check that never
        /// ran must not pass.
        #[arg(long)]
        skip_without_env: bool,

        /// Output format: `pretty` or `json`.
        ///
        /// ⚠️ `EVNX_OUTPUT_JSON=1` does the same thing and still works. It
        /// shipped first and is documented, but an environment variable is not a
        /// discoverable surface — which is how `--fix` came to be documented into
        /// existence before it existed. The flag wins when both are given.
        ///
        /// Unknown values are an error, not a silent fall back to `pretty`: a
        /// typo in a CI step must not quietly turn a JSON contract into prose
        /// that the next `jq` in the pipeline cannot read.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,

        /// Show detailed diagnostic output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Declare what each variable is: required, format, secret.
    ///
    /// Writes `[vars.*]` into `.evnx.toml` — the contract `.env.example` cannot
    /// express. A project with no spec behaves exactly as it does today.
    #[command(after_help = docs::SPEC.after_help)]
    Spec {
        #[command(subcommand)]
        action: SpecAction,
    },

    /// Run a command with a `.env`'s variables in its environment.
    ///
    /// The local `evnx cloud run`: the file is read and handed to the child
    /// process in its environment block, never its arguments, so `ps` shows
    /// the command and not the values, and the command never reads `.env`
    /// itself. evnx's own `EVNX_*` settings are not passed on.
    ///
    /// The command runs directly, not through a shell. For a pipeline or a
    /// builtin, ask for one: `-- sh -c 'a | b'`. Exits with the command's code.
    #[command(after_help = docs::RUN.after_help)]
    Run {
        /// The file to read. Defaults to `.env`, or `[defaults] env_name`.
        #[arg(long)]
        env: Option<String>,

        /// Read `.env.<NAME>` instead of `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

        /// Inject only variables whose name matches one of these globs.
        #[arg(long, value_delimiter = ',', value_name = "GLOB")]
        include: Option<Vec<String>>,

        /// Skip variables whose name matches one of these globs.
        #[arg(long, value_delimiter = ',', value_name = "GLOB")]
        exclude: Option<Vec<String>>,

        /// Inject variables that make a program load or run other code
        /// (`PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `BASH_ENV`, …), refused by
        /// default.
        #[arg(long)]
        allow_unsafe_names: bool,

        /// Mask secret values in the command's output as `***`.
        ///
        /// On by default when output is not a terminal (a CI log, an agent),
        /// where what is printed is kept. Only values the classifier calls
        /// secret (by name or by value) are masked. It prevents accidents, not
        /// exfiltration: a command that transforms a value gets past it.
        #[arg(long, conflicts_with = "no_mask")]
        mask: bool,

        /// Never mask the command's output.
        #[arg(long)]
        no_mask: bool,

        /// The command to run, after `--`.
        #[arg(
            last = true,
            required = true,
            allow_hyphen_values = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
    },

    /// Every command this build has, and which features it was compiled with.
    ///
    /// ⚠️ "This build" is the point. `cloud`, `migrate` and `backup` are
    /// optional features, so two binaries of the same version can have very
    /// different command sets — and nothing else tells you which one you have.
    /// PyPI shipped five releases with no cloud commands at all and nobody
    /// noticed, because every check anyone ran read `--version` and stopped.
    ///
    /// `-q` has no effect: the list is the answer.
    ///
    /// ⓘ Named `commands`; `surface` still works, because tooling calls it that.
    #[command(name = "commands", visible_alias = "surface", after_help = docs::COMMANDS.after_help)]
    Surface {
        /// The whole command tree as JSON, for tooling.
        ///
        /// Versioned by a `schema` field, so the shape can change without
        /// breaking anyone. The plain output carries no such promise — do not
        /// parse it.
        #[arg(long)]
        json: bool,

        /// One line instead of indented JSON. Requires `--json`.
        #[arg(long)]
        compact: bool,
    },

    /// Generate shell completions.
    ///
    /// The script is written to stdout and covers the commands this build has.
    /// `-q` has no effect: the script is the answer.
    #[command(after_help = docs::COMPLETIONS.after_help)]
    Completions {
        /// The shell to write the script for, in any case.
        #[arg(value_enum, ignore_case = true)]
        shell: clap_complete::Shell,
    },

    /// Show how to upgrade, and optionally whether a newer release exists.
    ///
    /// Without `--check` this contacts nothing: it reads the path of the running
    /// binary, and the files pip leaves beside it, to work out which channel
    /// installed it — cargo, Homebrew, Scoop, winget, npm, pip or the install
    /// script — and prints that channel's upgrade command. Where the install
    /// script and pip share a folder and nothing tells them apart, it prints
    /// both; where it cannot tell at all, every channel's.
    ///
    /// ⚠️ evnx never replaces its own binary. Each package manager owns the file
    /// it installed, and a self-updating binary fights it — Homebrew and winget
    /// would then report a version they did not install.
    #[command(after_help = docs::UPDATE.after_help)]
    Update {
        /// Ask api.github.com for the latest release.
        ///
        /// The only network request this command makes, and it is made only when
        /// you pass this flag. Nothing is sent but the request itself.
        ///
        /// Needs a build with network support (`cloud`, `migrate` or `net`); a bare
        /// `cargo install evnx` has none, and says so.
        #[arg(long)]
        check: bool,

        /// With `--check`: the answer as JSON on stdout, for scripts and CI.
        ///
        /// `schema` (1), `installed`, `latest`, `status` (`current`, `behind` or
        /// `ahead`), `channel`, `command` (null when evnx cannot name one) and
        /// `release_url`. The exit code is the same as without it.
        #[arg(long, requires = "check")]
        json: bool,
    },
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
    /// Every command describes itself, and no two siblings describe themselves
    /// the same way.
    ///
    /// ⚠️ This exists because of a real mistake that every other gate missed. A
    /// variant was inserted between a doc comment and the variant that comment
    /// belonged to, so `rotate-master-password` inherited "Delete your account
    /// and everything only you can reach" and `delete-account` was left blank.
    /// It compiled, passed clippy, passed 663 tests and `debug_assert()` — and
    /// `evnx auth --help` told people the wrong thing about the most destructive
    /// command in the tool. Nothing but running it would have shown it, so this
    /// runs it.
    #[test]
    fn every_command_describes_itself_distinctly() {
        fn walk(cmd: &clap::Command, path: &str) {
            let mut seen: std::collections::HashMap<String, String> = Default::default();
            for sub in cmd.get_subcommands() {
                let name = sub.get_name();
                let here = if path.is_empty() {
                    name.to_string()
                } else {
                    format!("{path} {name}")
                };

                // `help` is clap's own, and carries no description of ours.
                if name == "help" {
                    continue;
                }

                let about = sub
                    .get_about()
                    .map(|a| a.to_string())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                assert!(
                    !about.is_empty(),
                    "`evnx {here}` has no description — a doc comment is probably \
                     attached to the wrong variant"
                );
                if let Some(other) = seen.insert(about.clone(), here.clone()) {
                    panic!(
                        "`evnx {here}` and `evnx {other}` share a description ({about:?}), \
                         which means one of them has the other's doc comment"
                    );
                }
                walk(sub, &here);
            }
        }
        walk(&Cli::command(), "");
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

/// `evnx spec` subcommands.
#[derive(Subcommand, Debug)]
pub enum SpecAction {
    /// Generate a first spec from this project, or from named components.
    #[command(after_help = SPEC_INIT_EXIT_CODES)]
    Init {
        /// Build from the component catalogue instead of the project's files.
        ///
        /// Example: --with django,postgres,redis
        #[arg(long, value_delimiter = ',', conflicts_with_all = ["env", "env_name", "example"])]
        with: Option<Vec<String>>,

        /// The environment file to read sample values from.
        ///
        /// Defaults to `.env`, or `[defaults] env_name` from `.evnx.toml`. A
        /// file named here must exist.
        #[arg(long)]
        env: Option<String>,

        /// Read sample values from `.env.<NAME>` instead of `.env`.
        ///
        /// `--env-name production` reads `.env.production`. A name whose file
        /// does not exist is an error listing the environments that do.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,

        /// The template that decides which variables are in the contract.
        ///
        /// Defaults to `.env.example`, or `[defaults] example` from
        /// `.evnx.toml`. A file named here must exist.
        #[arg(long)]
        example: Option<String>,

        /// Print the generated block instead of writing it.
        ///
        /// Only TOML goes to stdout, so `> contract.toml` gives a file every
        /// command can read; notes and warnings go to stderr.
        #[arg(long)]
        stdout: bool,

        /// Add only the variables `[vars]` does not declare yet.
        ///
        /// Every existing entry, and everything else in `.evnx.toml`, is kept
        /// byte for byte. Nothing is written unless the result parses.
        #[arg(long, conflicts_with = "stdout")]
        merge: bool,

        /// Removed (I-178): it appended a second `[vars]` that TOML refuses.
        /// Kept hidden so the error can point at `--merge`.
        #[arg(long, hide = true)]
        force: bool,
    },

    /// Check the contract against `.env.example`, and `.env` when there is
    /// one. Read-only; suited to a pre-commit hook and to CI.
    ///
    /// Every variable `[vars]` declares for every environment must be in the
    /// template, and every name in the template must be declared. When `.env`
    /// exists, every required variable must be in it. A project without
    /// `[vars]` is skipped, and says so.
    #[command(after_help = SPEC_CHECK_EXIT_CODES)]
    Check {
        /// The template to check. Defaults to `.env.example`, or
        /// `[defaults] example` from `.evnx.toml`.
        #[arg(long)]
        example: Option<String>,

        /// The environment file to check. Defaults to `.env`, or
        /// `[defaults] env_name`. Skipped when it does not exist, unless named.
        #[arg(long)]
        env: Option<String>,

        /// Check `.env.<NAME>` instead of `.env`.
        #[arg(long, value_name = "NAME", conflicts_with = "env")]
        env_name: Option<String>,
    },
}

/// `spec check --help`: what each exit code means (RM-41).
const SPEC_CHECK_EXIT_CODES: &str = "\
Exit codes:
  0  the template and .env agree with [vars], or there is no [vars] (skipped)
  1  they disagree: a name missing from the template, one it has that [vars]
     does not declare, or a required variable missing from .env
  2  no verdict: the template is missing or does not parse, or a file named
     by --env or --env-name does not exist";

/// `spec init --help`: what each exit code means (D-131).
const SPEC_INIT_EXIT_CODES: &str = "\
Exit codes:
  0  the contract was written, printed, or every variable was already declared
  2  nothing was written: no variables to read, nothing to write, a file named
     by --env or --example that does not exist, a file that does not parse,
     [vars] already present without --merge, or an unknown component

The project's one .evnx.toml is written: the one in force (found at or above
this directory), else a new one at the git root.";

/// `--mode` for `template`: an octal file mode, at most 0777 (DEC-T6).
fn parse_mode(s: &str) -> Result<u32, String> {
    let digits = s.strip_prefix("0o").unwrap_or(s);
    match u32::from_str_radix(digits, 8) {
        Ok(m) if m <= 0o777 => Ok(m),
        _ => Err(format!(
            "'{s}' is not a file mode; use octal digits such as 0600 or 0640"
        )),
    }
}
