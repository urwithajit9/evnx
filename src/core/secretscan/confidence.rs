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
/// # Note
///
/// This enum mirrors `crate::utils::patterns::Confidence` for the scan module.
/// Conversion is automatic via `From` trait implementation.
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

    /// Parses the `--severity` threshold. Unknown values are an **error**, not a
    /// silent default: a typo in a CI flag must not quietly widen or narrow what
    /// the scanner reports.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::core::args::one_of(
            "severity",
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

/// Convert from `utils::patterns::Confidence`.
///
/// This allows seamless integration with the existing pattern detection logic.
impl From<crate::utils::patterns::Confidence> for Confidence {
    fn from(conf: crate::utils::patterns::Confidence) -> Self {
        match conf {
            crate::utils::patterns::Confidence::High => Confidence::High,
            crate::utils::patterns::Confidence::Medium => Confidence::Medium,
            crate::utils::patterns::Confidence::Low => Confidence::Low,
        }
    }
}
