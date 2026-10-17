pub mod dotenv_validation;
pub mod file_ops;
pub mod file_permissions;
pub mod fs;
pub mod git;
pub mod patterns;
pub mod prompt;
pub mod protect;
pub mod string;
pub mod term;
pub mod ui;

// Re-export commonly used items
pub use dotenv_validation::{count_dotenv_vars, looks_like_dotenv};
// `write_secure` is deprecated (C-04) and re-exported until it goes.
#[allow(deprecated)]
pub use file_permissions::write_secure;
pub use patterns::{calculate_entropy, detect_heuristic, is_placeholder, Confidence};
// `print_header_stderr` is deprecated (DEC-G1) and re-exported until it goes.
#[allow(deprecated)]
pub use ui::print_header_stderr;
pub use ui::{error, info, print_header, scanning_file_stderr, success, verbose_stderr, warning};
