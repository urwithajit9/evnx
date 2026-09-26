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
        // ⚠️ Collected rather than bailed on immediately, so `evnx scan a b c`
        // names every bad path in one run instead of one per invocation.
        let mut unreadable = Vec::new();

        for path_str in paths {
            let path = Path::new(path_str);

            if path.is_file() {
                // ⚠️ A path the user **named** is always scanned. Neither the
                // default exclusions nor the extension allowlist apply to it.
                //
                // They used to. `dist/`, `build/`, `node_modules/` and `target/`
                // are excluded by default, so naming a file inside one was
                // silently dropped — and the run then finished with
                //
                //     ✓  No secrets detected
                //     0 files scanned
                //
                // and exit 0, on a file holding a live key. A CI gate pointed at
                // a build artifact passed. `--exclude ''` did not override it,
                // because the defaults are not the `--exclude` list.
                //
                // The extension allowlist had the same effect for anything it
                // does not know — `evnx scan key.pem` scanned nothing and called
                // it clean.
                //
                // Exclusions exist to keep a *directory walk* from reading a
                // million vendored files. They are not a veto over an argument.
                files.push(path.to_path_buf());
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
            } else {
                // ⚠️ Neither a file nor a directory. Previously this branch did
                // not exist, so a path that could not be scanned was skipped in
                // silence and the run finished with "✓ No secrets detected" and
                // exit 0 — a scanner reporting success over ground it never
                // looked at. A mistyped path in CI turned the gate into a no-op
                // that passed.
                unreadable.push(path_str.clone());
            }
        }

        if !unreadable.is_empty() {
            let detail = unreadable
                .iter()
                .map(|p| {
                    // `exists` distinguishes a typo from a path that is there
                    // but cannot be scanned — a broken symlink, a socket, a
                    // directory that denies traversal.
                    if Path::new(p).exists() {
                        format!("  {p} (not a regular file or directory)")
                    } else {
                        format!("  {p} (does not exist)")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            anyhow::bail!(
                "cannot scan {} of the {} path(s) given:\n{detail}",
                unreadable.len(),
                paths.len()
            );
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
    ///
    /// ⚠️ A glob is matched against **three** spellings of the same path: as
    /// walked (`./fixtures/.env`), with a leading `./` removed
    /// (`fixtures/.env`), and the file name alone (`.env`). `glob::Pattern`
    /// anchors at both ends, so matching only the first meant `"fixtures/**"`
    /// and `"*.log"` — the two forms people actually write — matched nothing at
    /// all, silently. A pattern that appears to be doing something and is not is
    /// the same trap `.evnx.toml` was in.
    ///
    /// The union is deliberately a union rather than a replacement: every
    /// pattern that worked before still works, so no project's exclusions change
    /// meaning on upgrade.
    pub fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Check compiled glob patterns
        if !self.compiled_globs.is_empty() {
            let relative = path_str.strip_prefix("./").unwrap_or(&path_str);
            let file_name = path.file_name().map(|n| n.to_string_lossy());

            for glob in &self.compiled_globs {
                if glob.matches(&path_str) || glob.matches(relative) {
                    return true;
                }
                if let Some(name) = &file_name {
                    if glob.matches(name) {
                        return true;
                    }
                }
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

        if ALWAYS_EXCLUDE.iter().any(|p| path_str.contains(p)) {
            return true;
        }

        // `evnx backup` writes a single base64 blob, which the high-entropy
        // detector reads as a secret. It is ciphertext by construction, so a
        // finding there is always false. This only matters since `.env*` files
        // became scannable — before that, `.env.backup` was skipped for having
        // an unrecognised extension, and the false positive never surfaced.
        Self::is_evnx_backup(path)
    }

    /// Is this an `evnx backup` artefact — `.env.backup`, `.env.production.backup`,
    /// or a rotated `.env.backup.2`?
    ///
    /// Deliberately narrow: it matches only files that *this* filter newly admits,
    /// i.e. ones whose name starts with `.env`. A plain `config.backup` is
    /// unaffected and stays outside the scanner's reach exactly as before, rather
    /// than being newly excluded on the strength of its extension.
    fn is_evnx_backup(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        if !name.starts_with(".env") {
            return false;
        }
        // Strip a rotation suffix (`.1`, `.2`, …) before looking for `.backup`,
        // so `.env.backup.2` is recognised as readily as `.env.backup`.
        let stem = name
            .rsplit_once('.')
            .filter(|(_, last)| last.chars().all(|c| c.is_ascii_digit()) && !last.is_empty())
            .map_or(name, |(head, _)| head);
        stem.ends_with(".backup")
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

        // Name-based matches.
        //
        // ⚠️ This must NOT sit behind `if path.extension().is_none()`. Rust returns
        // `Some("production")` from `Path::extension()` for `.env.production` — a
        // dotfile only has "no extension" when it has no *further* dot — so guarding
        // on that made this branch unreachable for every dotted variant:
        // `.env.production`, `.env.local`, `.env.test`, `.env.staging`,
        // `.env.development`. `evnx scan .` walked straight past the file most
        // likely to hold production credentials and reported the directory clean.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with(".env") || name == "Dockerfile" || name == "Makefile" {
                return true;
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

    /// The regression this file exists to prevent.
    ///
    /// `Path::extension()` returns `Some("production")` for `.env.production`, so
    /// an implementation that only consults the file *name* when the extension is
    /// `None` skips every dotted variant — and a secret scanner that skips
    /// `.env.production` does not merely miss it, it reports the directory clean.
    #[test]
    fn dotted_env_variants_are_scannable() {
        for name in [
            ".env",
            ".env.local",
            ".env.test",
            ".env.staging",
            ".env.production",
            ".env.development",
            ".env.production.local",
        ] {
            assert!(
                FileFilter::is_scannable(Path::new(name)),
                "{name} must be scannable"
            );
        }
    }

    #[test]
    fn env_template_family_is_still_excluded() {
        let filter = FileFilter::new(&[]);
        for name in [".env.example", ".env.sample", ".env.template"] {
            assert!(
                filter.should_exclude(Path::new(name)),
                "{name} must stay excluded"
            );
        }
    }

    /// `evnx backup` output is a single base64 blob that the high-entropy detector
    /// reads as a secret. It is ciphertext, so the finding is always false.
    #[test]
    fn evnx_backups_are_excluded() {
        let filter = FileFilter::new(&[]);
        for name in [
            ".env.backup",
            ".env.backup.1",
            ".env.backup.12",
            ".env.production.backup",
            ".env.production.backup.3",
        ] {
            assert!(
                filter.should_exclude(Path::new(name)),
                "{name} is ciphertext and must not be scanned"
            );
        }

        // Narrowly scoped: a backup that is not an `.env*` file is left exactly as
        // it was before this rule existed — unscannable by extension, not newly
        // excluded by name.
        assert!(!filter.should_exclude(Path::new("config.backup")));
        assert!(!FileFilter::is_scannable(Path::new("config.backup")));

        // And `.env` itself is not a backup.
        assert!(!filter.should_exclude(Path::new(".env")));
        assert!(!filter.should_exclude(Path::new(".env.production")));
    }

    /// End-to-end over a real directory: the shape a user actually has.
    #[test]
    fn collect_files_picks_up_every_env_variant() {
        let dir = TempDir::new().unwrap();
        for name in [
            ".env",
            ".env.local",
            ".env.test",
            ".env.staging",
            ".env.production",
            ".env.development",
            ".env.example",
            ".env.backup",
        ] {
            fs::write(
                dir.path().join(name),
                "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7REALKEY\n",
            )
            .unwrap();
        }

        let filter = FileFilter::new(&[]);
        let found: Vec<String> = filter
            .collect_files(&[dir.path().to_string_lossy().to_string()])
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        for name in [
            ".env",
            ".env.local",
            ".env.test",
            ".env.staging",
            ".env.production",
            ".env.development",
        ] {
            assert!(
                found.contains(&name.to_string()),
                "{name} was not collected"
            );
        }
        assert!(!found.contains(&".env.example".to_string()));
        assert!(!found.contains(&".env.backup".to_string()));
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
