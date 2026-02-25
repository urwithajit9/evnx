use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "evnx",
    about = "Manage .env files — validation, secret scanning, and format conversion",
    version,
    author
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Interactive project setup — generates .env.example
    Init {
        /// Project stack (python, nodejs, rust, go, php)
        #[arg(long)]
        stack: Option<String>,

        /// Services to include (comma-separated)
        #[arg(long)]
        services: Option<String>,

        /// Output path for .env.example
        #[arg(long, default_value = ".")]
        path: String,

        /// Skip all prompts and use defaults
        #[arg(short, long)]
        yes: bool,
    },

    /// Check .env against .env.example, find issues
    Validate {
        /// Path to .env file
        #[arg(long, default_value = ".env")]
        env: String,

        /// Path to .env.example file
        #[arg(long, default_value = ".env.example")]
        example: String,

        /// Fail on warnings, not just errors
        #[arg(long)]
        strict: bool,

        /// Auto-fix safe issues
        #[arg(long)]
        fix: bool,

        /// Output format (pretty, json, github-actions)
        #[arg(long, default_value = "pretty")]
        format: String,

        /// Always exit with 0 (useful in CI)
        #[arg(long)]
        exit_zero: bool,
    },

    /// Detect secrets that look real (AWS keys, tokens, etc.)
    Scan {
        /// Files/directories to scan
        #[arg(default_value = ".")]
        path: Vec<String>,

        /// Exclude files matching pattern
        #[arg(long, default_value = ".env.example")]
        exclude: Vec<String>,

        /// Only scan for specific patterns (aws, stripe, github, all)
        #[arg(long)]
        pattern: Vec<String>,

        /// Skip obvious placeholders
        #[arg(long)]
        ignore_placeholders: bool,

        /// Output format (pretty, json, sarif)
        #[arg(long, default_value = "pretty")]
        format: String,

        /// Don't fail CI even if secrets found
        #[arg(long)]
        exit_zero: bool,
    },

    /// Compare .env vs .env.example — show missing/extra vars
    Diff {
        /// Path to .env file
        #[arg(long, default_value = ".env")]
        env: String,

        /// Path to .env.example file
        #[arg(long, default_value = ".env.example")]
        example: String,

        /// Show actual values (default: hide for security)
        #[arg(long)]
        show_values: bool,

        /// Output format (pretty, json, patch)
        #[arg(long, default_value = "pretty")]
        format: String,

        /// Reverse comparison (.env.example vs .env)
        #[arg(long)]
        reverse: bool,
    },

    /// Transform to different formats (JSON, YAML, shell, etc.)
    Convert {
        /// Input .env file
        #[arg(long, default_value = ".env")]
        env: String,

        /// Target format (required in non-interactive mode)
        #[arg(long)]
        to: Option<String>,

        /// Output file (default: stdout)
        #[arg(long)]
        output: Option<String>,

        /// Include only matching vars (glob pattern)
        #[arg(long)]
        include: Option<String>,

        /// Exclude matching vars
        #[arg(long)]
        exclude: Option<String>,

        /// Base64-encode all values
        #[arg(long)]
        base64: bool,

        /// Add prefix to all keys
        #[arg(long)]
        prefix: Option<String>,

        /// Key transform (uppercase, lowercase, camelCase, snake_case)
        #[arg(long)]
        transform: Option<String>,
    },

    /// Full migration workflow to secret managers
    #[cfg(feature = "migrate")]
    Migrate {
        /// Source type (env-file, aws, gcp, github, doppler)
        #[arg(long)]
        from: Option<String>,

        /// Destination type
        #[arg(long)]
        to: Option<String>,

        /// Source .env file
        #[arg(long, default_value = ".env")]
        source_file: String,

        /// GitHub repository (OWNER/REPO)
        #[arg(long)]
        repo: Option<String>,

        /// Secret name for AWS/GCP single-secret storage
        #[arg(long)]
        secret_name: Option<String>,

        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,

        /// Skip variables that already exist
        #[arg(long)]
        skip_existing: bool,

        /// Overwrite existing secrets
        #[arg(long)]
        overwrite: bool,

        /// GitHub Personal Access Token
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,

        /// AWS profile to use
        #[arg(long)]
        aws_profile: Option<String>,
    },

    /// Keep .env and .env.example in sync
    Sync {
        /// Direction (forward: .env → .env.example, reverse: .env.example → .env)
        #[arg(long, default_value = "forward")]
        direction: String,

        /// Add with placeholder values
        #[arg(long)]
        placeholder: bool,
    },

    /// Generate config files from templates   
    Template {
        /// Input template file
        #[arg(long)]
        input: String,

        /// Output file
        #[arg(long)]
        output: String,

        /// .env file to use for values
        #[arg(long, default_value = ".env")]
        env: String,
    },

    /// Create encrypted backup of .env
    #[cfg(feature = "backup")]
    Backup {
        /// .env file to backup
        #[arg(default_value = ".env")]
        env: String,

        /// Output file
        #[arg(long)]
        output: Option<String>,
    },

    /// Restore from encrypted backup
    #[cfg(feature = "backup")]
    Restore {
        /// Backup file
        backup: String,

        /// Output file
        #[arg(long, default_value = ".env")]
        output: String,
    },

    /// Diagnose common setup issues   
    Doctor {
        /// Project directory to diagnose
        #[arg(default_value = ".")]
        path: String,
    },

    /// Generate shell completions
    Completions {
        /// Shell type (bash, zsh, fish, powershell)
        shell: String,
    },
}
