//! destination.rs — Shared trait and types for all migration destinations
//!
//! Every destination (GitHub, AWS, Doppler, …) implements `MigrationDestination`.
//! The `run()` entry point in `mod.rs` resolves a boxed trait object and calls
//! `migrate()`, keeping the dispatch table trivially extensible.

use anyhow::Result;
use indexmap::IndexMap;

// ─── Result type returned by every destination ───────────────────────────────

/// Summary produced after a migration run.
#[derive(Debug, Default)]
pub struct MigrationResult {
    /// Number of secrets successfully uploaded / applied.
    pub uploaded: usize,
    /// Number of secrets intentionally skipped (already existed, excluded, etc.).
    pub skipped: usize,
    /// Number of secrets that failed to upload.
    pub failed: usize,
    /// Human-readable error messages, one per failed secret.
    pub errors: Vec<String>,
}

/// Whether a destination performs the transfer itself, or writes out the
/// commands for the operator to run.
///
/// ⚠️ This exists because the summary line used to lie. Eight of the nine
/// destinations never contact the platform at all — they print
/// `aws secretsmanager create-secret …` or `doppler secrets set …` for you to
/// run — and every one of them still reported `✓ N uploaded`. A count of
/// secrets that were *not* uploaded, labelled "uploaded", is the kind of thing
/// somebody acts on by deleting their `.env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationKind {
    /// evnx talks to the platform and the secrets are there when it returns.
    Uploads,
    /// evnx prints commands. Nothing has left the machine.
    EmitsCommands,
}

impl MigrationResult {
    /// Pretty-print the summary table.
    ///
    /// `kind` decides the wording, because "uploaded" is only true for a
    /// destination that actually uploaded.
    pub fn print_summary(&self, kind: DestinationKind) {
        use colored::Colorize;
        println!("\n{}", "Summary:".bold());
        match kind {
            DestinationKind::Uploads => {
                println!("  ✓  {} uploaded", self.uploaded);
            }
            DestinationKind::EmitsCommands => {
                println!("  ✓  {} command(s) generated", self.uploaded);
                println!(
                    "     {}",
                    "nothing has been uploaded — run the commands above".yellow()
                );
            }
        }
        if self.skipped > 0 {
            println!("  ⊘  {} skipped", self.skipped);
        }
        if self.failed > 0 {
            println!("  ✗  {} failed", self.failed);
            for e in &self.errors {
                println!("      • {}", e);
            }
        }
    }
}

// ─── Per-run options passed into every destination ───────────────────────────

/// Runtime options threaded through from CLI flags into every destination.
#[derive(Debug, Clone, Default)]
pub struct MigrationOptions {
    /// Print what would happen without making any changes.
    pub dry_run: bool,
    /// Skip secrets that already exist at the destination.
    pub skip_existing: bool,
    /// Silently overwrite secrets that already exist.
    pub overwrite: bool,
    /// Emit extra diagnostic output.
    pub verbose: bool,

    // ── Filtering ──────────────────────────────────────────────────────────
    /// Glob patterns; only secrets whose key matches are uploaded.
    /// Example: `["DB_*", "AWS_*"]`
    pub include: Option<Vec<String>>,
    /// Glob patterns; secrets whose key matches are excluded.
    /// Example: `["*_LOCAL", "*_TEST"]`
    pub exclude: Option<Vec<String>>,
    /// Strip this prefix from every key before uploading.
    /// Example: `"APP_"` → `APP_DB_URL` becomes `DB_URL`.
    pub strip_prefix: Option<String>,
    /// Add this prefix to every key before uploading.
    pub add_prefix: Option<String>,

    // ── Destination-specific overrides ─────────────────────────────────────
    /// GitHub repository `owner/repo`.
    pub repo: Option<String>,
    /// GitHub Personal Access Token.
    pub github_token: Option<String>,

    /// AWS Secrets Manager secret name, e.g. `prod/myapp/config`.
    pub secret_name: Option<String>,
    /// AWS CLI named profile.
    pub aws_profile: Option<String>,

    /// Doppler / Infisical project slug.
    pub project: Option<String>,
    /// Doppler config (e.g. `dev`, `staging`, `prd`).
    pub doppler_config: Option<String>,
    /// Infisical environment (e.g. `dev`, `staging`, `prod`).
    /// Separate from `doppler_config` so each can be set independently.
    pub infisical_env: Option<String>,

    /// Azure Key Vault name.
    pub vault_name: Option<String>,

    /// Heroku app name.
    pub heroku_app: Option<String>,

    /// Vercel project ID or name.
    pub vercel_project: Option<String>,
    /// Vercel API token.
    pub vercel_token: Option<String>,

    /// Railway project ID (for CLI pass-through).
    pub railway_project: Option<String>,
}

// ─── The core trait ──────────────────────────────────────────────────────────

/// Every migration destination must implement this trait.
///
/// # Example
///
/// ```rust,no_run
/// use indexmap::IndexMap;
/// use anyhow::Result;
/// use evnx::commands::migrate::destination::{
///     MigrationDestination, MigrationOptions, MigrationResult,
/// };
///
/// struct MyDestination;
///
/// impl MigrationDestination for MyDestination {
///     fn name(&self) -> &str { "my-destination" }
///
///     fn migrate(
///         &self,
///         secrets: &IndexMap<String, String>,
///         opts: &MigrationOptions,
///     ) -> Result<MigrationResult> {
///         // upload logic …
///         Ok(MigrationResult { uploaded: secrets.len(), ..Default::default() })
///     }
/// }
/// ```
pub trait MigrationDestination {
    /// Short display name shown in progress messages.
    fn name(&self) -> &str;

    /// Does this destination upload, or print commands?
    ///
    /// ⚠️ Defaults to `EmitsCommands`, which is both the majority (eight of
    /// nine) and the safe direction to be wrong in: a destination that forgets
    /// to override this under-claims rather than reporting an upload that never
    /// happened.
    fn kind(&self) -> DestinationKind {
        DestinationKind::EmitsCommands
    }

    /// Perform (or simulate) the migration.
    ///
    /// Implementations must honour `opts.dry_run`: when true they must not
    /// make any external mutations, only print what would happen.
    fn migrate(
        &self,
        secrets: &IndexMap<String, String>,
        opts: &MigrationOptions,
    ) -> Result<MigrationResult>;

    /// Optional: print platform-specific "next steps" after a successful migration.
    fn print_next_steps(&self) {}
}
