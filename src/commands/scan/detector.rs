//! Secret detection strategies and registry.
//!
//! This module implements the strategy pattern for secret detection, allowing
//! multiple detection approaches to be registered and executed. Each detector
//! implements the [`SecretDetector`] trait.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────┐
//! │  DetectorRegistry   │
//! │  (manages detectors)│
//! └─────────┬───────────┘
//!           │
//!   ┌───────┼───────┐
//!   │       │       │
//!   ▼       ▼       ▼
//! ┌─────┐ ┌─────┐ ┌──────────┐
//! │Pattern│ │Entropy│ │  Custom  │
//! │Detector│ │Detector│ │Detector  │
//! └─────┘ └─────┘ └──────────┘
//! ```
//!
//! # Adding a New Detector
//!
//! Follow these steps to add a new detection strategy:
//!
//! ## Step 1: Create a new struct
//!
//! ```rust
//! # use evnx::commands::scan::detector::{SecretDetector, Detection};
//! # use evnx::commands::scan::Confidence;
//! # use std::path::Path;
//! pub struct MyNewDetector {
//!     min_length: usize,
//! }
//! ```
//!
//! ## Step 2: Implement the `SecretDetector` trait
//!
//! ```rust
//! # use evnx::commands::scan::{SecretDetector, Detection};
//! # use evnx::commands::scan::Confidence;
//! # use std::path::Path;
//! # pub struct MyNewDetector { min_length: usize }
//! impl SecretDetector for MyNewDetector {
//!     fn name(&self) -> &str {
//!         "my-detector"
//!     }
//!
//!     fn scan_kv(&self, key: &str, value: &str, _location: &str) -> Option<Detection> {
//!         if value.starts_with("SECRET_") {
//!             Some(Detection {
//!                 pattern: "My Pattern".to_string(),
//!                 confidence: Confidence::High,
//!                 action_url: None,
//!                 matched_value: value.to_string(),
//!             })
//!         } else {
//!             None
//!         }
//!     }
//!
//!     fn scan_token(&self, token: &str, _location: &str) -> Option<Detection> {
//!         self.scan_kv("", token, "")
//!     }
//! }
//! ```
//!
//! ## Step 3: Register in `DetectorRegistry::new()`
//!
//! ```rust
//! # use evnx::commands::scan::{DetectorRegistry, SecretDetector, Detection, Confidence};//
//! # struct MyDetector;
//! # impl SecretDetector for MyDetector {
//! #   fn name(&self) -> &str { "test" }
//! #   fn scan_kv(&self, _: &str, _: &str, _: &str) -> Option<evnx::commands::scan::Detection> { None }
//! #   fn scan_token(&self, _: &str, _: &str) -> Option<evnx::commands::scan::Detection> { None }
//! # }
//! let mut registry = DetectorRegistry::new();
//! registry.register(MyDetector);
//! ```
//!
//! # Example
//!
//! ```no_run
//! # use evnx::commands::scan::DetectorRegistry;
//! let registry = DetectorRegistry::new();
//! let detections = registry.scan_kv("AWS_KEY", "AKIA1234567890EXAMPLE", "test:1");
//! ```

use super::models::Confidence;
use crate::utils::patterns;
use std::path::Path;

/// Result of a single detection attempt.
///
/// Contains the full matched value (truncation happens later for display).
/// This allows detectors to return complete information while the runner
/// handles safe display formatting.
#[derive(Debug, Clone)]
pub struct Detection {
    /// Name of the detected pattern
    pub pattern: String,
    /// Confidence level of the detection
    pub confidence: Confidence,
    /// Optional URL for remediation actions
    pub action_url: Option<String>,
    /// The full matched value (will be truncated for display)
    pub matched_value: String,
}

/// Trait for secret detection strategies.
///
/// Implement this trait to add new detection approaches. Each detector
/// can specialize in different types of secrets or use different algorithms.
///
/// # Methods
///
/// * `name()` - Returns human-readable identifier for reporting
/// * `applies_to()` - Optional filter for file types (default: all files)
/// * `scan_kv()` - Scan key-value pairs (optimized for .env files)
/// * `scan_token()` - Scan raw tokens (for general text files)
///
/// # Implementation Notes
///
/// - Return `None` when no secret is detected (avoid false positives)
/// - Use appropriate confidence levels based on detection certainty
/// - Provide action_url when known remediation steps exist
///
/// # Example
///
/// ```rust
/// # use evnx::commands::scan::Confidence;
/// # use evnx::commands::scan::Detection;
/// # use evnx::commands::scan::SecretDetector;
/// pub struct EntropyDetector {
///     min_entropy: f64,
/// }
///
/// impl SecretDetector for EntropyDetector {
///     fn name(&self) -> &str { "entropy-analyzer" }
///
///     fn scan_kv(&self, _key: &str, value: &str, _location: &str) -> Option<Detection> {///
///         if value.len() > 30 {
///             Some(Detection {
///                 pattern: "high-entropy-string".to_string(),
///                 confidence: Confidence::Medium,
///                 action_url: None,
///                 matched_value: value.to_string(),
///             })
///         } else {
///             None
///         }
///     }
///
///     fn scan_token(&self, token: &str, _location: &str) -> Option<Detection> {
///         self.scan_kv("", token, "")
///     }
/// }
/// ```
pub trait SecretDetector: Send + Sync {
    /// Returns the human-readable name of this detector.
    ///
    /// Used in verbose output and debugging to identify which detector
    /// found a particular secret.
    fn name(&self) -> &str;

    /// Check if this detector should run on the given file.
    ///
    /// Override this method to optimize performance by skipping
    /// detectors that aren't relevant for certain file types.
    ///
    /// # Default Implementation
    ///
    /// Returns `true` for all files (detector runs on everything).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use std::path::Path;
    /// # struct SecretDetector;
    /// # impl SecretDetector {
    /// fn applies_to(&self, _path: &Path) -> bool {
    ///     true
    /// }
    /// # }
    /// ```
    fn applies_to(&self, _path: &Path) -> bool {
        true
    }

    /// Scan a key-value pair for secrets.
    ///
    /// Used primarily for .env files and configuration files where
    /// variable names provide additional context for detection.
    ///
    /// # Arguments
    ///
    /// * `key` - The variable/key name (e.g., "AWS_ACCESS_KEY_ID")
    /// * `value` - The value to scan
    /// * `location` - File and line information for reporting
    ///
    /// # Returns
    ///
    /// `Some(Detection)` if a secret is found, `None` otherwise.
    fn scan_kv(&self, key: &str, value: &str, location: &str) -> Option<Detection>;

    /// Scan a raw token for secrets.
    ///
    /// Used for general text files where key-value structure isn't present.
    /// Tokens are typically extracted by splitting on whitespace and separators.
    ///
    /// # Arguments
    ///
    /// * `token` - The string token to analyze
    /// * `location` - File and line information for reporting
    ///
    /// # Returns
    ///
    /// `Some(Detection)` if a secret is found, `None` otherwise.
    fn scan_token(&self, token: &str, location: &str) -> Option<Detection>;
}

/// Registry that manages all active secret detectors.
///
/// The registry holds a collection of detectors and provides unified
/// methods to scan values through all registered detectors.
///
/// # Example
///
/// ```
/// # use evnx::commands::scan::detector::DetectorRegistry;
/// let registry = DetectorRegistry::new();
/// let detections = registry.scan_kv("API_KEY", "sk_live_12345", "config.yml:10");
/// for detection in detections {
///     println!("Found: {}", detection.pattern);
/// }
/// ```
pub struct DetectorRegistry {
    detectors: Vec<Box<dyn SecretDetector>>,
}

impl DetectorRegistry {
    /// Create a new registry with default detectors registered.
    ///
    /// # Default Detectors
    ///
    /// - [`PatternDetector`] - Regex-based pattern matching (always registered)
    ///
    /// # Adding Custom Detectors
    ///
    /// ```rust
    /// # use evnx::commands::scan::DetectorRegistry;
    /// let mut registry = DetectorRegistry::new();
    /// // Future:registry.register(MyCustomDetector::default());
    /// ```
    pub fn new() -> Self {
        let mut registry = Self {
            detectors: Vec::new(),
        };
        // Register default detectors
        registry.register(PatternDetector);
        // Future: registry.register(EntropyDetector::default());
        registry
    }

    /// Register a new detector with the registry.
    ///
    /// # Arguments
    ///
    /// * `detector` - Any type implementing `SecretDetector + 'static`
    ///
    /// # Example
    ///
    /// ```ignore
    /// # use evnx::commands::scan::DetectorRegistry;
    /// # struct MyDetector;
    /// let mut registry = DetectorRegistry::new();
    /// registry.register(MyDetector);
    ///
    /// ```
    pub fn register(&mut self, detector: impl SecretDetector + 'static) {
        self.detectors.push(Box::new(detector));
    }

    /// Scan a key-value pair through all registered detectors.
    ///
    /// Returns all detections from all detectors (multiple detectors
    /// may flag the same value for different reasons).
    ///
    /// # Arguments
    ///
    /// * `key` - Variable/key name
    /// * `value` - Value to scan
    /// * `location` - File and line context
    ///
    /// # Returns
    ///
    /// Vector of all detections (may be empty if no secrets found).
    pub fn scan_kv(&self, key: &str, value: &str, location: &str) -> Vec<Detection> {
        self.detectors
            .iter()
            .filter_map(|d| d.scan_kv(key, value, location))
            .collect()
    }

    /// Scan a raw token through all registered detectors.
    ///
    /// Similar to [`scan_kv()`](Self::scan_kv) but for token-based scanning
    /// where no key-name context is available.
    ///
    /// # Arguments
    ///
    /// * `token` - Token string to analyze
    /// * `location` - File and line context
    ///
    /// # Returns
    ///
    /// Vector of all detections (may be empty if no secrets found).
    pub fn scan_token(&self, token: &str, location: &str) -> Vec<Detection> {
        self.detectors
            .iter()
            .filter_map(|d| d.scan_token(token, location))
            .collect()
    }

    /// Get the number of registered detectors.
    ///
    /// Useful for debugging and verbose output.
    pub fn detector_count(&self) -> usize {
        self.detectors.len()
    }
}

impl Default for DetectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Default pattern-based secret detector.
///
/// Uses regex patterns from `crate::utils::patterns` to detect
/// known secret formats (API keys, tokens, credentials, etc.).
///
/// This is the primary detector and is always registered by default.
pub struct PatternDetector;

impl SecretDetector for PatternDetector {
    fn name(&self) -> &str {
        "pattern-matcher"
    }

    fn scan_kv(&self, key: &str, value: &str, _location: &str) -> Option<Detection> {
        patterns::detect_secret(value, key).map(|(pattern, confidence, action_url)| Detection {
            pattern: pattern.clone(),
            confidence: confidence.into(), // Convert patterns::Confidence → models::Confidence
            action_url,
            matched_value: value.to_string(),
        })
    }

    fn scan_token(&self, token: &str, _location: &str) -> Option<Detection> {
        patterns::detect_secret(token, "").map(|(pattern, confidence, action_url)| Detection {
            pattern: pattern.clone(),
            confidence: confidence.into(), // Convert patterns::Confidence → models::Confidence
            action_url,
            matched_value: token.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_has_default_detectors() {
        let registry = DetectorRegistry::new();
        assert_eq!(registry.detector_count(), 1); // PatternDetector
    }

    #[test]
    fn test_registry_register() {
        let mut registry = DetectorRegistry::new();
        registry.register(PatternDetector); // Register again for test
        assert_eq!(registry.detector_count(), 2);
    }

    #[test]
    fn test_pattern_detector_name() {
        let detector = PatternDetector;
        assert_eq!(detector.name(), "pattern-matcher");
    }

    // Note: Full detection tests depend on utils::patterns implementation
    // These tests verify the detector structure works correctly
    #[test]
    fn test_detection_struct() {
        let detection = Detection {
            pattern: "Test Pattern".to_string(),
            confidence: Confidence::High,
            action_url: Some("https://example.com".to_string()),
            matched_value: "secret_value_123".to_string(),
        };
        assert_eq!(detection.pattern, "Test Pattern");
        assert_eq!(detection.confidence, Confidence::High);
        assert_eq!(detection.matched_value, "secret_value_123");
    }
}
