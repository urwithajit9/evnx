//! Shared types for the doctor command.
//!
//! These types are used across checks, output formatters, and the runner.
//! They are designed to be serializable (serde) for JSON output support.

use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Severity level for diagnostic results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Critical issue that should be fixed
    Error,
    /// Potential problem or best practice violation
    Warning,
    /// Informational message
    Info,
    /// Check passed successfully
    Ok,
}

impl Severity {
    /// Get the icon symbol for this severity
    pub fn icon(self) -> &'static str {
        match self {
            Severity::Error => "✗",
            Severity::Warning => "⚠️",
            Severity::Info => "ℹ️",
            Severity::Ok => "✓",
        }
    }
    /// Get the colored icon for terminal output
    pub fn colored_icon(self) -> ColoredString {
        // ← Add this method
        match self {
            Severity::Error => self.icon().red(),
            Severity::Warning => self.icon().yellow(),
            Severity::Info => self.icon().cyan(),
            Severity::Ok => self.icon().green(),
        }
    }
}

/// Result of a single diagnostic check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub description: String,
    pub severity: Severity,
    pub details: Option<String>,
    pub fixable: bool,
    pub fixed: bool,
    #[serde(skip)]
    pub fix_action: Option<FixFn>,
}

/// Function type for auto-fix operations
pub type FixFn = fn(&Path, bool) -> Result<bool>;

/// Aggregated diagnostic report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub project_path: String,
    pub checks: Vec<CheckResult>,
    pub summary: Summary,
    pub timestamp: String,
}

/// Summary statistics for a diagnostic report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub total: usize,
    pub errors: usize,
    pub warnings: usize,
    pub passed: usize,
    pub info: usize,
    pub fixable: usize,
}

impl Default for Summary {
    fn default() -> Self {
        Self::new()
    }
}
impl Summary {
    /// Create a new empty summary
    pub fn new() -> Self {
        Self {
            total: 0,
            errors: 0,
            warnings: 0,
            passed: 0,
            info: 0,
            fixable: 0,
        }
    }

    /// Add a check result to the summary statistics
    pub fn add(&mut self, result: &CheckResult) {
        self.total += 1;
        match result.severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
            Severity::Ok => self.passed += 1,
            Severity::Info => self.info += 1,
        }
        if result.fixable && !result.fixed {
            self.fixable += 1;
        }
    }
}
