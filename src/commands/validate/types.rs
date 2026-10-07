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
}

impl IssueType {
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
    pub fixed_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub status: String,
    pub required_present: usize,
    pub required_total: usize,
    pub issues: Vec<Issue>,
    pub fixed: Vec<FixApplied>,
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
