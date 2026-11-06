//! How sure a detection is: `high`, `medium` or `low`.
//!
//! Moved here from `commands::scan::models` with the rule engine (R-scan), so
//! `core` code — and `sync`, which names the detector behind an `allow_actual`
//! refusal — can use it without importing a command.

use serde::{Deserialize, Serialize};

/// Confidence level for a detected secret.
///
/// Used to prioritize remediation efforts. High confidence findings
/// should be addressed immediately as they likely represent real credentials.
///
/// The only one: `utils::patterns` had a second, joined by `From`, until C1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    /// Highly likely to be a real secret (e.g., matches known pattern + context)
    High,
    /// Possibly a secret, needs manual verification
    Medium,
    /// Low likelihood, could be false positive
    Low,
}

impl Confidence {
    /// Rank for threshold comparisons — higher means more confident.
    ///
    /// Declared explicitly rather than derived. `derive(Ord)` follows *declaration*
    /// order, which here reads `High, Medium, Low` and would therefore make `High`
    /// the **smallest** value — a `--severity high` filter that silently kept
    /// everything. Naming the ranks makes the intent unmissable and survives
    /// someone reordering the variants.
    pub fn rank(self) -> u8 {
        match self {
            Confidence::Low => 0,
            Confidence::Medium => 1,
            Confidence::High => 2,
        }
    }
}

impl PartialOrd for Confidence {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Confidence {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl std::str::FromStr for Confidence {
    type Err = anyhow::Error;

    /// Parses the `--confidence` threshold (it was `--severity` until C2).
    /// Unknown values are an **error**, not a silent default: a typo in a CI
    /// flag must not quietly widen or narrow what the scanner reports.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::core::args::one_of(
            "confidence",
            s,
            &[
                ("high", Confidence::High),
                ("medium", Confidence::Medium),
                ("med", Confidence::Medium),
                ("low", Confidence::Low),
                ("all", Confidence::Low),
            ],
            &["high", "medium", "low"],
        )
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Confidence::High => "high",
                Confidence::Medium => "medium",
                Confidence::Low => "low",
            }
        )
    }
}

/// How bad a finding is if it is real: `critical`, `high`, `medium` or `low`
/// (S7, C2). Separate from [`Confidence`], which is how **sure** evnx is.
///
/// A Google Maps browser key is the case that needed both: its shape is
/// unmistakable (high confidence) and it is designed to ship in web pages
/// (low severity). One field could only say one of those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Grants direct access to money, cloud accounts or signing keys.
    Critical,
    High,
    Medium,
    Low,
}

impl Severity {
    /// Ranked explicitly, as [`Confidence::rank`] is, so reordering the
    /// variants cannot invert a `--severity` filter.
    pub fn rank(self) -> u8 {
        match self {
            Severity::Low => 0,
            Severity::Medium => 1,
            Severity::High => 2,
            Severity::Critical => 3,
        }
    }
}

impl PartialOrd for Severity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Severity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl std::str::FromStr for Severity {
    type Err = anyhow::Error;

    /// Unknown values are an error, for the same reason as [`Confidence`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::core::args::one_of(
            "severity",
            s,
            &[
                ("critical", Severity::Critical),
                ("high", Severity::High),
                ("medium", Severity::Medium),
                ("med", Severity::Medium),
                ("low", Severity::Low),
                ("all", Severity::Low),
            ],
            &["critical", "high", "medium", "low"],
        )
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        })
    }
}
