pub mod file_ops;
pub mod fs;
pub mod git;
pub mod patterns;
pub mod string;
pub mod ui;

// Re-export commonly used items
pub use patterns::{calculate_entropy, detect_secret, is_placeholder, Confidence};
