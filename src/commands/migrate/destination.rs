//! destination.rs — Shared trait and types for all migration destinations
//!
//! Every destination (GitHub, AWS, Doppler, …) implements `MigrationDestination`.
//! The `run()` entry point in `mod.rs` resolves a boxed trait object and calls
//! `migrate()`, keeping the dispatch table trivially extensible.

use anyhow::Result;
use indexmap::IndexMap;
use std::fmt::Display;

// ─── Where each line goes ────────────────────────────────────────────────────

/// Where a destination's lines go (I-105, I-111).
///
/// ⚠️ The commands shared stdout with everything else — the load line, "Run
/// these commands:", "Requires Heroku CLI", the summary — so
/// `evnx migrate … > run.sh` saved a file `sh` could not run (I-111). Three
/// kinds of line, three rules:
///
/// | kind | default | `-q` | `--print-only` |
/// |---|---|---|---|
/// | [`command`](Out::command) | stdout | stdout | stdout |
/// | [`status`](Out::status) | stdout | hidden | stderr |
/// | [`result`](Out::result) | stdout | stdout | stderr |
///
/// With neither flag every line is where it always was.
#[derive(Debug, Clone, Copy, Default)]
pub struct Out {
    /// `-q`: status lines are hidden.
    pub quiet: bool,
    /// `--print-only`: stdout holds commands and nothing else.
    pub print_only: bool,
}

impl Out {
    /// A command, or a comment or blank line that belongs to the commands.
    pub fn command(&self, line: impl Display) {
        println!("{line}");
    }

    /// A line about the run — what is being done, what to install, what was
    /// renamed. `-q` hides it.
    pub fn status(&self, line: impl Display) {
        if !self.quiet {
            self.result(line);
        }
    }

    /// What the run came to: the summary, a dry run's preview. Shown under
    /// `-q`; on stderr with `--print-only`.
    pub fn result(&self, line: impl Display) {
        if self.print_only {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
}

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
    /// Pretty-print the summary table, as result lines (see [`Out`]).
    ///
    /// `kind` decides the wording, because "uploaded" is only true for a
    /// destination that actually uploaded.
    pub fn print_summary(&self, kind: DestinationKind, dry_run: bool, out: Out) {
        use colored::Colorize;
        out.result(format!("\n{}", "Summary:".bold()));

        // ⚠️ A dry run neither generates nor skips.
        //
        // Destinations report a preview as `skipped: secrets.len()`, because
        // nothing was uploaded — so the summary read
        //
        //     ✓  0 command(s) generated
        //        nothing has been uploaded — run the commands above
        //     ⊘  3 skipped
        //
        // directly beneath the three commands it had just printed. "0
        // generated … run the commands above" cannot both be true, and the
        // three were rendered rather than skipped. The honesty fix that removed
        // the false `✓ N uploaded` did not reach these counters.
        if dry_run {
            let covered = self.uploaded + self.skipped;
            out.result(format!("  ·  {covered} secret(s) previewed"));
            out.result(format!(
                "     {}",
                "dry run — nothing was uploaded and nothing was written".yellow()
            ));
            if self.failed > 0 {
                out.result(format!("  ✗  {} would fail", self.failed));
                for e in &self.errors {
                    out.result(format!("      • {}", e));
                }
            }
            return;
        }

        match kind {
            DestinationKind::Uploads => {
                out.result(format!("  ✓  {} uploaded", self.uploaded));
            }
            DestinationKind::EmitsCommands => {
                out.result(format!("  ✓  {} command(s) generated", self.uploaded));
                // With `--print-only` the commands went to stdout — a file,
                // usually — and this line to stderr, so they are not "above".
                let run = if out.print_only {
                    "nothing has been uploaded — run the commands written to stdout"
                } else {
                    "nothing has been uploaded — run the commands above"
                };
                out.result(format!("     {}", run.yellow()));
            }
        }
        if self.skipped > 0 {
            out.result(format!("  ⊘  {} skipped", self.skipped));
        }
        if self.failed > 0 {
            out.result(format!("  ✗  {} failed", self.failed));
            for e in &self.errors {
                out.result(format!("      • {}", e));
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
    /// `--yes`: do not ask before uploading. Never implies `overwrite`.
    pub yes: bool,
    /// Emit extra diagnostic output.
    pub verbose: bool,
    /// Where lines go: `-q` and `--print-only` (I-105, I-111).
    pub out: Out,

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

    /// Whether the platform treats names that differ only in case as one
    /// (Azure Key Vault, GitHub). Two such names are one secret there, the
    /// second overwriting the first, so the collision check folds case (I-104).
    fn names_ignore_case(&self) -> bool {
        false
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

    /// Optional: platform-specific "next steps" after a successful migration,
    /// as status lines.
    fn print_next_steps(&self, _out: Out) {}
}

/// A destination's naming, for the shared collision check
/// ([`ConvertOptions::prepare`](crate::core::converter::ConvertOptions::prepare)).
///
/// The names are compared as `migrate` will hand them over; only case folding
/// is the platform's. Azure's `_` → `-` cannot merge two variable names (they
/// hold no `-`), so folding case is the whole of its rule here.
impl crate::core::converter::Naming for dyn MigrationDestination + '_ {
    fn name(&self) -> &str {
        MigrationDestination::name(self)
    }

    fn names_ignore_case(&self) -> bool {
        MigrationDestination::names_ignore_case(self)
    }
}
