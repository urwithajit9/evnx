use anyhow::{Context, Result};
use std::path::Path;

/// Ensure directory exists, creating it if necessary
pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

/// Check if a file contains a specific substring
pub fn file_contains(path: &Path, substring: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    Ok(content.contains(substring))
}

/// Append content to a file if it doesn't already contain a marker
pub fn append_if_missing(path: &Path, content: &str, marker: &str) -> Result<bool> {
    if file_contains(path, marker)? {
        return Ok(false);
    }

    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("Failed to open file for appending: {}", path.display()))?;

    writeln!(file, "{}", content)
        .with_context(|| format!("Failed to append to file: {}", path.display()))?;

    Ok(true)
}
