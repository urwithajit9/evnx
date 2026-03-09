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
            IssueType::InvalidUrl => "invalid_url",
            IssueType::InvalidPort => "invalid_port",
            IssueType::InvalidEmail => "invalid_email",
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
    pub old_value: Option<String>,
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
    pub validate_formats: bool,
    pub ignore_issues: std::collections::HashSet<String>,
    pub env_pattern: Option<String>,
}
