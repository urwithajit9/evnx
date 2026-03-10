//! File filtering and collection utilities.
//!
//! This module handles:
//! - Collecting files from paths (files and directories)
//! - Excluding files based on patterns (glob and substring)
//! - Determining if a file is scannable (text-based)
//!
//! # Example
//!
//! ```
//! use evnx::commands::scan::filters::FileFilter;
//!
//! let filter = FileFilter::new(&["node_modules".to_string(), "*.log".to_string()]);
//! let files = filter.collect_files(&vec!["./src".to_string()]).unwrap();
//! println!("Found {} files to scan", files.len());
//! ```

use anyhow::Result;
use glob::Pattern;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// File filter for collecting scannable files.
///
/// Manages exclusion patterns and file type filtering to determine
/// which files should be included in the scan.
///
/// # Exclusion Patterns
///
/// Supports two types of patterns:
/// - **Glob patterns**: `*.log`, `test*`, `**/node_modules/**`
/// - **Substring patterns**: `node_modules`, `.git`, `vendor`
///
/// # Always Excluded
///
/// The following are always excluded regardless of configuration:
/// - `.git/`, `node_modules/`, `target/`, `dist/`, `build/`
/// - `.env.example`, `.env.sample`, `.env.template`
///
/// # Example
///
/// ```no_run
/// # use evnx::commands::scan::filters::FileFilter;
/// let filter = FileFilter::new(&["*.test.js".to_string()]);
/// let files = filter.collect_files(&vec!["./src".to_string()]).unwrap();
/// println!("Found {} files to scan", files.len());
/// ```
pub struct FileFilter {
    exclude_patterns: Vec<String>,
    compiled_globs: Vec<Pattern>,
}

impl FileFilter {
    /// Create a new FileFilter with exclusion patterns.
    ///
    /// # Arguments
    ///
    /// * `exclude` - Vector of exclusion patterns (glob or substring)
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::filters::FileFilter;
    /// let filter = FileFilter::new(&[
    ///     "node_modules".to_string(),
    ///     "*.log".to_string(),
    ///     "test_".to_string(),
    /// ]);
    /// ```
    pub fn new(exclude: &[String]) -> Self {
        let compiled_globs = exclude
            .iter()
            .filter(|p| p.contains('*'))
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        Self {
            exclude_patterns: exclude.to_vec(),
            compiled_globs,
        }
    }

    /// Collect all scannable files from the given paths.
    ///
    /// Recursively walks directories and applies exclusion filters.
    ///
    /// # Arguments
    ///
    /// * `paths` - Vector of file or directory paths to scan
    ///
    /// # Returns
    ///
    /// Vector of PathBuf for all files that should be scanned.
    ///
    /// # Errors
    ///
    /// Returns error if a path cannot be read (permissions, etc.)
    pub fn collect_files(&self, paths: &[String]) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        for path_str in paths {
            let path = Path::new(path_str);

            if path.is_file() {
                if !self.should_exclude(path) && Self::is_scannable(path) {
                    files.push(path.to_path_buf());
                }
            } else if path.is_dir() {
                for entry in WalkDir::new(path)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let entry_path = entry.path();
                    if entry_path.is_file()
                        && !self.should_exclude(entry_path)
                        && Self::is_scannable(entry_path)
                    {
                        files.push(entry_path.to_path_buf());
                    }
                }
            }
        }

        Ok(files)
    }

    /// Check if a file should be excluded from scanning.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to check
    ///
    /// # Returns
    ///
    /// `true` if the file should be excluded, `false` otherwise.
    ///
    /// # Pattern Matching
    ///
    /// 1. Glob patterns (compiled at initialization)
    /// 2. Substring patterns (simple contains check)
    /// 3. Always-excluded patterns (hardcoded)
    pub fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Check compiled glob patterns
        for glob in &self.compiled_globs {
            if glob.matches(&path_str) {
                return true;
            }
        }

        // Check substring patterns
        for pattern in &self.exclude_patterns {
            if !pattern.contains('*') && path_str.contains(pattern) {
                return true;
            }
        }

        // Always exclude common non-secret files
        const ALWAYS_EXCLUDE: &[&str] = &[
            ".git/",
            "node_modules/",
            "target/",
            "dist/",
            "build/",
            ".env.example",
            ".env.sample",
            ".env.template",
        ];

        ALWAYS_EXCLUDE.iter().any(|p| path_str.contains(p))
    }

    /// Check if a file is scannable (text-based).
    ///
    /// # Arguments
    ///
    /// * `path` - Path to check
    ///
    /// # Returns
    ///
    /// `true` if the file should be scanned, `false` for binary files.
    ///
    /// # Scannable Extensions
    ///
    /// Common text file extensions: env, txt, sh, py, js, ts, rs, go,
    /// java, rb, php, yml, yaml, json, toml, xml, conf, config, ini, properties
    ///
    /// # Special Cases
    ///
    /// Files without extensions are checked by name:
    /// - `.env*` files
    /// - `Dockerfile`
    /// - `Makefile`
    pub fn is_scannable(path: &Path) -> bool {
        const SCANNABLE_EXTENSIONS: &[&str] = &[
            "env",
            "txt",
            "sh",
            "bash",
            "zsh",
            "py",
            "js",
            "ts",
            "rs",
            "go",
            "java",
            "rb",
            "php",
            "yml",
            "yaml",
            "json",
            "toml",
            "xml",
            "conf",
            "config",
            "ini",
            "properties",
        ];

        // Check extension
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if SCANNABLE_EXTENSIONS.contains(&ext_str.as_str()) {
                return true;
            }
        }

        // Files without extensions
        if path.extension().is_none() {
            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();
                if name_str.starts_with(".env")
                    || name_str == "Dockerfile"
                    || name_str == "Makefile"
                {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_file_filter_new() {
        let filter = FileFilter::new(&["test".to_string(), "*.log".to_string()]);
        assert_eq!(filter.exclude_patterns.len(), 2);
        assert_eq!(filter.compiled_globs.len(), 1); // Only *.log is a glob
    }

    #[test]
    fn test_should_exclude_substring() {
        let filter = FileFilter::new(&["node_modules".to_string()]);
        assert!(filter.should_exclude(Path::new("project/node_modules/package.json")));
        assert!(!filter.should_exclude(Path::new("project/src/main.rs")));
    }

    #[test]
    fn test_should_exclude_always() {
        let filter = FileFilter::new(&[]);
        assert!(filter.should_exclude(Path::new(".git/config")));
        assert!(filter.should_exclude(Path::new(".env.example")));
        assert!(!filter.should_exclude(Path::new(".env")));
    }

    #[test]
    fn test_is_scannable_extensions() {
        assert!(FileFilter::is_scannable(Path::new("config.py")));
        assert!(FileFilter::is_scannable(Path::new("app.js")));
        assert!(FileFilter::is_scannable(Path::new("data.json")));
        assert!(!FileFilter::is_scannable(Path::new("image.png")));
        assert!(!FileFilter::is_scannable(Path::new("binary.exe")));
    }

    #[test]
    fn test_is_scannable_special_files() {
        assert!(FileFilter::is_scannable(Path::new(".env")));
        assert!(FileFilter::is_scannable(Path::new("Dockerfile")));
        assert!(FileFilter::is_scannable(Path::new("Makefile")));
    }

    #[test]
    fn test_collect_files() {
        let temp_dir = TempDir::new().unwrap();
        let file1 = temp_dir.path().join("test.py");
        let file2 = temp_dir.path().join("skip.png");
        fs::write(&file1, "print('hello')").unwrap();
        fs::write(&file2, "binary").unwrap();

        let filter = FileFilter::new(&[]);
        let files = filter
            .collect_files(&[temp_dir.path().to_string_lossy().to_string()])
            .unwrap();

        assert_eq!(files.len(), 1);
        assert!(files.contains(&file1));
    }
}
