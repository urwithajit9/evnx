//! Data models for secret scanning results.
//!
//! This module defines the core data structures used throughout the scan command:
//! - [`Finding`]: Represents a single detected secret with metadata
//! - [`ScanResults`]: Aggregates all findings from a scan operation
//! - [`Confidence`]: Enum for confidence levels (High/Medium/Low)
//!
//! # Example
//!
//! ```rust
//! # use evnx::commands::scan::{Finding, ScanResults, Confidence};
//!
//! let finding = Finding {
//!     pattern: "AWS Access Key".to_string(),
//!     confidence: Confidence::High,
//!     value_preview: "AKIA...XYZ12".to_string(),
//!     location: ".env:15 (AWS_ACCESS_KEY_ID)".to_string(),
//!     variable: Some("AWS_ACCESS_KEY_ID".to_string()),
//!     action_url: Some("https://console.aws.amazon.com/iam".to_string()),
//! };
//!
//! let mut results = ScanResults::new(100);
//! results.add_finding(finding);
//! ```

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

/// Convert from `patterns::Confidence` to `models::Confidence`.
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

/// A single detected secret with contextual metadata.
///
/// # Fields
///
/// * `pattern` - Name of the detected secret pattern (e.g., "AWS Access Key")
/// * `confidence` - Confidence level of the detection
/// * `value_preview` - Truncated secret value for safe display (never shows full secret)
/// * `location` - File path and line number where secret was found
/// * `variable` - Variable name if detected in a key-value context (e.g., .env files)
/// * `action_url` - Optional URL for remediation (e.g., key rotation page)
///
/// # Security Note
///
/// The `value_preview` field should never contain the full secret value.
/// Use [`truncate_value()`](super::runner::truncate_value) to safely truncate
/// before creating a Finding.
///
/// # Example
///
/// ```rust
/// # use evnx::commands::scan::{Finding, Confidence};
/// let finding = Finding::new(
///     "GitHub Token",
///     Confidence::High,
///     "ghp_...abc12",
///     ".env:10 (GITHUB_TOKEN)",
///     Some("GITHUB_TOKEN".to_string()),
///     Some("https://github.com/settings/tokens".to_string()),
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub pattern: String,
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: Confidence,
    pub value_preview: String,
    pub location: String,
    pub variable: Option<String>,
    pub action_url: Option<String>,
}

/// Custom serializer for Confidence to output as string in JSON
fn serialize_confidence<S>(confidence: &Confidence, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&confidence.to_string())
}

impl Finding {
    /// Create a new Finding with all required fields.
    ///
    /// # Arguments
    ///
    /// * `pattern` - Name of the detected pattern
    /// * `confidence` - Confidence level
    /// * `value_preview` - Truncated secret value (max 20 chars visible)
    /// * `location` - File and line information
    /// * `variable` - Optional variable name
    /// * `action_url` - Optional remediation URL
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{Finding, Confidence};
    /// let finding = Finding::new(
    ///     "GitHub Token",
    ///     Confidence::High,
    ///     "ghp_...abc12",
    ///     ".env:10 (GITHUB_TOKEN)",
    ///     Some("GITHUB_TOKEN".to_string()),
    ///     Some("https://github.com/settings/tokens".to_string()),
    /// );
    /// ```
    pub fn new(
        pattern: impl Into<String>,
        confidence: Confidence,
        value_preview: impl Into<String>,
        location: impl Into<String>,
        variable: Option<String>,
        action_url: Option<String>,
    ) -> Self {
        Self {
            pattern: pattern.into(),
            confidence,
            value_preview: value_preview.into(),
            location: location.into(),
            variable,
            action_url,
        }
    }
}

/// Aggregated results from a complete scan operation.
///
/// # Fields
///
/// * `files_scanned` - Total number of files processed
/// * `secrets_found` - Total number of findings
/// * `findings` - Vector of all detected secrets
/// * `high_confidence` - Count of high-confidence findings
/// * `medium_confidence` - Count of medium-confidence findings
/// * `low_confidence` - Count of low-confidence findings
///
/// # Example
///
/// ```rust
/// # use evnx::commands::scan::ScanResults;
/// let mut results = ScanResults::new(50);
/// // ... add findings ...
/// println!("Found {} secrets in {} files", results.secrets_found, results.files_scanned);
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct ScanResults {
    pub files_scanned: usize,
    pub secrets_found: usize,
    pub findings: Vec<Finding>,
    pub high_confidence: usize,
    pub medium_confidence: usize,
    pub low_confidence: usize,
}

impl ScanResults {
    /// Create new ScanResults with initialized counters.
    ///
    /// # Arguments
    ///
    /// * `files_scanned` - Number of files that will be/were scanned
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::ScanResults;
    /// let results = ScanResults::new(100);
    /// assert_eq!(results.files_scanned, 100);
    /// assert_eq!(results.secrets_found, 0);
    /// ```
    pub fn new(files_scanned: usize) -> Self {
        Self {
            files_scanned,
            secrets_found: 0,
            findings: Vec::new(),
            high_confidence: 0,
            medium_confidence: 0,
            low_confidence: 0,
        }
    }

    /// Add a finding and update confidence counters.
    ///
    /// # Arguments
    ///
    /// * `finding` - The detected secret to add
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{ScanResults, Finding, Confidence};
    /// let mut results = ScanResults::new(10);
    /// let finding = Finding::new("API Key", Confidence::High, "sk_...123", "config.yml:5", None, None);
    /// results.add_finding(finding);
    /// assert_eq!(results.secrets_found, 1);
    /// assert_eq!(results.high_confidence, 1);
    /// ```
    pub fn add_finding(&mut self, finding: Finding) {
        match finding.confidence {
            Confidence::High => self.high_confidence += 1,
            Confidence::Medium => self.medium_confidence += 1,
            Confidence::Low => self.low_confidence += 1,
        }
        self.secrets_found += 1;
        self.findings.push(finding);
    }

    /// Check if any high or medium confidence secrets were found.
    ///
    /// Useful for determining if immediate action is required.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{ScanResults, Finding, Confidence};
    /// let mut results = ScanResults::new(10);
    /// results.add_finding(Finding::new("Key", Confidence::High, "val", "loc", None, None));
    /// if results.has_critical_findings() {
    ///     println!("⚠️  Critical secrets detected!");
    /// }
    /// ```
    pub fn has_critical_findings(&self) -> bool {
        self.high_confidence > 0 || self.medium_confidence > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_display() {
        assert_eq!(Confidence::High.to_string(), "high");
        assert_eq!(Confidence::Medium.to_string(), "medium");
        assert_eq!(Confidence::Low.to_string(), "low");
    }

    #[test]
    fn test_confidence_from_patterns() {
        // Test conversion from patterns::Confidence
        assert_eq!(
            Confidence::from(crate::utils::patterns::Confidence::High),
            Confidence::High
        );
        assert_eq!(
            Confidence::from(crate::utils::patterns::Confidence::Medium),
            Confidence::Medium
        );
        assert_eq!(
            Confidence::from(crate::utils::patterns::Confidence::Low),
            Confidence::Low
        );
    }

    #[test]
    fn test_finding_new() {
        let finding = Finding::new(
            "AWS Key",
            Confidence::High,
            "AKIA...XYZ",
            ".env:10",
            Some("AWS_KEY".to_string()),
            None,
        );
        assert_eq!(finding.pattern, "AWS Key");
        assert_eq!(finding.confidence, Confidence::High);
        assert_eq!(finding.variable, Some("AWS_KEY".to_string()));
    }

    #[test]
    fn test_scan_results_new() {
        let results = ScanResults::new(50);
        assert_eq!(results.files_scanned, 50);
        assert_eq!(results.secrets_found, 0);
        assert!(results.findings.is_empty());
    }

    #[test]
    fn test_scan_results_add_finding() {
        let mut results = ScanResults::new(10);
        results.add_finding(Finding::new(
            "Key1",
            Confidence::High,
            "val",
            "loc:1",
            None,
            None,
        ));
        results.add_finding(Finding::new(
            "Key2",
            Confidence::Medium,
            "val",
            "loc:2",
            None,
            None,
        ));
        results.add_finding(Finding::new(
            "Key3",
            Confidence::Low,
            "val",
            "loc:3",
            None,
            None,
        ));

        assert_eq!(results.secrets_found, 3);
        assert_eq!(results.high_confidence, 1);
        assert_eq!(results.medium_confidence, 1);
        assert_eq!(results.low_confidence, 1);
    }

    #[test]
    fn test_has_critical_findings() {
        let mut results = ScanResults::new(10);
        assert!(!results.has_critical_findings());

        results.add_finding(Finding::new(
            "Key",
            Confidence::Low,
            "val",
            "loc",
            None,
            None,
        ));
        assert!(!results.has_critical_findings());

        results.add_finding(Finding::new(
            "Key",
            Confidence::High,
            "val",
            "loc",
            None,
            None,
        ));
        assert!(results.has_critical_findings());
    }
}
