//! Platform-specific file permission utilities.

use anyhow::Result;
// use std::fs::File;
use std::path::Path;

/// Write content to a file with restrictive permissions (owner read/write only).
///
/// On Unix-like systems, sets mode `0o600`. On Windows, uses default ACLs
/// (which are typically user-only by default).
pub fn write_secure<P: AsRef<Path>>(path: P, content: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // rw-------
            .open(path)?;
        std::io::Write::write_all(&mut file, content)?;
    }

    #[cfg(not(unix))]
    {
        // Windows: rely on default ACLs; could add explicit ACL logic if needed
        std::fs::write(path, content)?;
    }

    Ok(())
}
