//! The secret-detection engine: format rules, the detectors that run them and
//! the heuristics, and how sure each answer is (R-scan, A-09).
//!
//! `evnx scan` is its main user, and `sync` asks the same engine which
//! detector would report a value before writing it into `.env.example`. It
//! lived in `commands::scan`, so `sync` imported a command to ask; it is here
//! now, and `commands::scan` re-exports it under the old paths.
//!
//! What a scan reads (the walk), how a file is decoded and parsed, and how the
//! results are printed stay in `commands::scan`. Splitting those out too, and
//! then a crate of their own, is SC-WP1 and SC-WP9.

mod confidence;
pub mod detector;
pub mod patternset;

pub use confidence::Confidence;
pub use detector::{Detection, DetectorRegistry, SecretDetector};
pub use patternset::{PatternMatch, PatternSet};
