//! Shared types for validation module

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IssueType {
    MissingVariable,
    ExtraVariable,
    PlaceholderValue,
    BooleanTrap,
    WeakSecret,
    LocalhostInDocker,
    InvalidUrl,
    InvalidPort,
    InvalidEmail,
    /// A value that does not match the format its `[vars]` entry declares.
    FormatMismatch,
    /// The same key assigned more than once in one file.
    DuplicateKey,
    /// A key under `[vars.X]` evnx does not understand — usually a typo that
    /// leaves the variable's contract other than its author meant (I-187).
    UnknownSpecKey,
    /// An `environments` name that matches no environment here, or a file
    /// named close to a listed one: the variable's requirement silently stops
    /// applying (I-042).
    UnknownEnvironment,
    /// The env file is not covered by `.gitignore`: `git add .` takes it
    /// (D-27, DEC-VA1). A warning.
    EnvNotIgnored,
    /// The env file is already tracked by git: its values are in the history
    /// (D-27, DEC-VA1). An error.
    EnvTracked,
    /// A line this project's readers (Docker Compose, python-dotenv, Node
    /// dotenv) read differently from evnx (RM-04). A warning, never fixable:
    /// fixing it would choose a reader for the user.
    ReaderMismatch,
}

impl IssueType {
    /// Every type, in the order `--ignore`'s error lists them.
    pub const ALL: [IssueType; 16] = [
        IssueType::MissingVariable,
        IssueType::ExtraVariable,
        IssueType::PlaceholderValue,
        IssueType::BooleanTrap,
        IssueType::WeakSecret,
        IssueType::LocalhostInDocker,
        IssueType::InvalidUrl,
        IssueType::InvalidPort,
        IssueType::InvalidEmail,
        IssueType::FormatMismatch,
        IssueType::DuplicateKey,
        IssueType::UnknownSpecKey,
        IssueType::UnknownEnvironment,
        IssueType::EnvNotIgnored,
        IssueType::EnvTracked,
        IssueType::ReaderMismatch,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::MissingVariable => "missing_variable",
            IssueType::ExtraVariable => "extra_variable",
            IssueType::PlaceholderValue => "placeholder_value",
            IssueType::BooleanTrap => "boolean_trap",
            IssueType::WeakSecret => "weak_secret",
            IssueType::LocalhostInDocker => "localhost_in_docker",
            IssueType::DuplicateKey => "duplicate_key",
            IssueType::InvalidUrl => "invalid_url",
            IssueType::InvalidPort => "invalid_port",
            IssueType::InvalidEmail => "invalid_email",
            IssueType::FormatMismatch => "format_mismatch",
            IssueType::UnknownSpecKey => "unknown_spec_key",
            IssueType::UnknownEnvironment => "unknown_environment",
            IssueType::EnvNotIgnored => "env_not_ignored",
            IssueType::EnvTracked => "env_tracked",
            IssueType::ReaderMismatch => "reader_mismatch",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Issue {
    pub severity: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub variable: String,
    pub message: String,
    pub location: String,
    pub suggestion: Option<String>,
    pub auto_fixable: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FixApplied {
    pub variable: String,
    pub action: String,
    /// The value that was replaced — **only when printing it is harmless**.
    ///
    /// `None` with `old_value_withheld == false` means there was no previous
    /// value: the variable was added, not repaired.
    pub old_value: Option<String>,
    /// `true` when there *was* a previous value and evnx declined to repeat it.
    ///
    /// ⛔ The redaction lives on the struct rather than in the pretty printer
    /// because `--format json` serialises this type verbatim. A printer-side
    /// mask would have left `evnx validate --fix --rotate-weak-secrets
    /// --format json` piping the old credential into whatever read it.
    ///
    /// The replaced value is in `.env.bak`, which is the right place for it:
    /// one file, 0600, not a terminal scrollback or a CI log.
    #[serde(default)]
    pub old_value_withheld: bool,
    pub new_value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub style: usize,
    /// Repairs that are complete: `fixed`'s length.
    pub fixed_count: usize,
    /// Variables `--fix` added with a placeholder: `needs_input`'s length.
    #[serde(default)]
    pub needs_input: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub status: String,
    /// `--fix --dry-run`: `fixed` and `needs_input` are what `--fix` would do,
    /// and everything else describes the file as it stands — nothing was
    /// written.
    #[serde(default)]
    pub dry_run: bool,
    pub required_present: usize,
    pub required_total: usize,
    pub issues: Vec<Issue>,
    /// Repairs `--fix` completed.
    pub fixed: Vec<FixApplied>,
    /// ⚠️ Not repairs (I-037): variables `--fix` added with a placeholder,
    /// because evnx cannot invent their values. Each still has its
    /// `placeholder_value` finding in `issues`, so `status`, the error count
    /// and the exit code agree until someone fills it in.
    #[serde(default)]
    pub needs_input: Vec<FixApplied>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Default)]
pub struct ValidationConfig {
    pub strict: bool,
    pub fix: bool,
    /// `--rotate-weak-secrets`: let `--fix` replace a weak secret that holds a
    /// **real value**, not only a placeholder.
    ///
    /// ⛔ Off by default, and that default is the whole of S1. See the flag's
    /// own documentation in `cli.rs`.
    pub rotate_weak_secrets: bool,
    pub validate_formats: bool,
    /// Issue types the caller asked to suppress, from `--ignore`.
    pub ignore_issues: std::collections::HashSet<String>,
}
