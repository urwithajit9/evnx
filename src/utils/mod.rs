pub mod dotenv_validation;
pub mod file_ops;
pub mod file_permissions;
pub mod fs;
pub mod git;
pub mod patterns;
pub mod string;
pub mod ui;

// Re-export commonly used items
pub use dotenv_validation::{count_dotenv_vars, looks_like_dotenv};
pub use file_permissions::write_secure;
pub use patterns::{calculate_entropy, detect_secret, is_placeholder, Confidence};
pub use ui::{
    error, info, print_header, print_header_stderr, scanning_file_stderr, success, verbose_stderr,
    warning,
};
