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

/// Rewrite `\` as `/` so a pattern written with forward slashes can match a
/// Windows path.
///
/// ⚠️ Only used for the **user's own** `--exclude` substring patterns, and only
/// as an *additional* spelling to try — never as a replacement for the path as
/// written. On Unix a backslash is a legal filename character, so normalising
/// unconditionally would make `--exclude a/b` match a single file named
/// `a\b`. Trying both spellings cannot remove a match, so no project's
/// exclusions change meaning on upgrade.
///
/// ⓘ The hardcoded directory exclusions do **not** go through this. They match
/// by path component instead, which the platform's own parser splits correctly,
/// and that is strictly better than normalising a string.
fn normalise_separators(path_str: &str) -> String {
    path_str.replace('\\', "/")
}

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

        // Check substring patterns.
        //
        // ⚠️ Matched against the path as written *and* with separators
        // normalised to `/`. On Windows `to_string_lossy` yields
        // `project\\node_modules\\pkg.json`, so `--exclude node_modules/` —
        // the spelling every tutorial and `.gitignore` uses — matched nothing
        // there. Trying both only ever *adds* a match for a pattern the user
        // typed on purpose, so no existing exclusion changes meaning.
        let slashed = normalise_separators(&path_str);
        for pattern in &self.exclude_patterns {
            if !pattern.contains('*') && (path_str.contains(pattern) || slashed.contains(pattern)) {
                return true;
            }
        }

        // Always exclude common non-secret locations.
        //
        // ⛔ These were one list of substrings — `".git/"`, `"node_modules/"`,
        // … — tested with `path_str.contains(p)`. Every directory entry ended
        // in a **forward slash**, and on Windows `to_string_lossy` returns
        // `node_modules\\pkg\\.env`. So on Windows **not one of the five
        // directory exclusions ever fired**, and `scan` walked `.git`,
        // `node_modules`, `target`, `dist` and `build` in full.
        //
        // ⓘ It over-scanned rather than under-reporting, so it was never a
        // missed secret — it was a slow walk of `node_modules` and a flood of
        // findings from lockfile hashes and vendored test fixtures, which is
        // its own route to someone deciding the tool cries wolf.
        //
        // Directories are matched **by path component** and files by name, so
        // the platform's own path parser does the separator handling and there
        // is no string to normalise.
        //
        // ⓘ A component is an exact comparison, which also closes a quieter
        // over-match in the other direction: `contains("dist/")` was true for
        // `redist/`, `contains("target/")` for `my_target/`, and
        // `contains("build/")` for `prebuild/` — all plausible directory names
        // that were silently skipped. ⚠️ Note the shape: the extra text has to
        // come **before** the name. `my-node_modules-notes/` was *not*
        // affected, because the trailing `-notes` breaks the substring.
        const ALWAYS_EXCLUDE_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build"];
        const ALWAYS_EXCLUDE_FILES: &[&str] = &[".env.example", ".env.sample", ".env.template"];

        if path
            .components()
            .any(|c| ALWAYS_EXCLUDE_DIRS.iter().any(|d| c.as_os_str() == *d))
        {
            return true;
        }

        // ⚠️ By file name, not by substring: `.env.example` as a substring also
        // matched `.env.example.bak` and `my.env.example-notes`, and a name
        // comparison is what was meant. Falls back to the whole path when a
        // path has no final component, which `components()` can produce for
        // something like `..`.
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or(std::borrow::Cow::Borrowed(""));
        if ALWAYS_EXCLUDE_FILES.iter().any(|f| name == *f) {
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
            // ⚠️ A notebook is JSON, and identical content was scanned as
            // `.json` and ignored as `.ipynb` — a filter problem, not a parsing
            // one. Notebooks are where ML work keeps keys, and a key often
            // appears **twice**: once in a `source` cell and again in an
            // `outputs` cell where it was echoed. Both are found now, because
            // the detectors are content-based and only the extension was
            // stopping them.
            "ipynb",
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
            // ⚠️ An editor's swap file is named `.env.swp` and is **binary**, so
            // `.env*` selects it. That was harmless while an unreadable file was
            // silently skipped; now that one fails the scan, a `.env` left open
            // in vim would fail CI.
            //
            // Excluded by extension rather than by "looks binary", because the
            // decision belongs here: these files are transient editor state, and
            // scanning them was never the intent.
            //
            // ⛔ **Not `.bak` and not `~`.** `evnx validate --fix` writes
            // `.env.bak` itself, and an editor's `.env~` is plain text — both
            // hold the same secrets as the file they copy, so both must stay
            // scannable. Only the binary swap formats are dropped.
            const EDITOR_SWAP: &[&str] = &[".swp", ".swo", ".swn", ".swm", ".swl"];
            if EDITOR_SWAP.iter().any(|e| name.ends_with(e)) {
                return false;
            }
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

    /// ⚠️ `.env.swp` is binary editor state and `.env*` selects it, so once an
    /// unreadable file started failing the scan, a `.env` open in vim would
    /// fail CI. `.env.bak` and `.env~` must stay in: they hold the same secrets
    /// as the file they copy, and `evnx validate --fix` writes the first one.
    #[test]
    fn editor_swap_files_are_not_scanned_but_backups_are() {
        for skip in [".env.swp", ".env.swo", ".env.swn", "secrets.env.swp"] {
            assert!(
                !FileFilter::is_scannable(Path::new(skip)),
                "{skip} should not be scanned"
            );
        }
        for keep in [".env.bak", ".env.bak.2", ".env~", ".env", ".env.production"] {
            assert!(
                FileFilter::is_scannable(Path::new(keep)),
                "{keep} must still be scanned"
            );
        }
    }

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

    /// ⚠️ **An external review asked for this exclusion to be removed, and the
    /// answer is "not yet". Measured before deciding.**
    ///
    /// The argument for removing it is good: `.env.example` is one of the few
    /// `.env*` files that actually gets committed, so a live key pasted there is
    /// exactly the leak this command exists to catch — and today it is missed.
    ///
    /// The argument was "placeholders are already suppressed". They are not. A
    /// perfectly ordinary template produces **five** findings:
    ///
    /// ```text
    /// DATABASE_URL=postgresql://user:password@localhost:5432/dbname
    ///                                      → Credentials in a connection string
    /// STRIPE_SECRET_KEY=sk_test_xxxxxxxxxx → Stripe Secret Key (test)
    /// AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE → AWS Access Key
    /// GITHUB_TOKEN=ghp_xxxxxxxxxxxxxxxxxxxx → GitHub Token
    /// API_KEY=<your-api-key>               → Sensitive config key
    /// ```
    ///
    /// and `utils::patterns::is_placeholder` recognises only two of those seven
    /// values. Adding a real key makes it six findings among five false ones.
    ///
    /// Removing the exclusion today would turn every project's pre-commit hook
    /// red on a correct template, which is how a tool teaches people to ignore
    /// it — a worse outcome than the miss it would fix.
    ///
    /// ⛔ **The prerequisite is placeholder detection that recognises template
    /// values**, which also needs `--ignore-placeholders` to stop being inert.
    /// Both are tracked. Until then this stays, deliberately.
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

    /// ⛔ **The defect, stated as the string comparison it actually was.**
    ///
    /// `ALWAYS_EXCLUDE` held `"node_modules/"` with a forward slash and was
    /// tested with `path_str.contains(p)`. On Windows the path string uses
    /// `\`, so the test was this — and it is `false`:
    #[test]
    fn a_forward_slash_substring_cannot_match_a_windows_path() {
        let windows_path = r"project\node_modules\pkg\.env";

        // What the old code asked, on every one of the five directory
        // patterns. All five answered "do not exclude".
        for pattern in [".git/", "node_modules/", "target/", "dist/", "build/"] {
            assert!(
                !windows_path.contains(pattern),
                "{pattern} cannot appear in a backslash path — this is the bug"
            );
        }

        // ⓘ This test needs no Windows machine because the defect was never
        // about Windows behaviour; it was about comparing a `/` pattern against
        // a `\` string. The fix removes the comparison rather than fixing the
        // string, so the one below is what now decides it.
    }

    /// The component walk the fix uses, exercised on this platform's own
    /// separator.
    ///
    /// ⚠️ **On Unix this test passes against the old substring code too**, so
    /// on the runners that exist today it is a regression guard and nothing
    /// more — measured, not assumed. The assertion that actually distinguishes
    /// fixed from broken is `windows_paths_hit_the_hardcoded_directory_exclusions`
    /// below, and it cannot run until `windows-latest` is in CI's matrix. ⛔ Do
    /// not read this test's green as coverage of the bug it was written for.
    #[test]
    fn hardcoded_directories_are_excluded_by_component() {
        let filter = FileFilter::new(&[]);
        for dir in [".git", "node_modules", "target", "dist", "build"] {
            let nested: PathBuf = ["project", dir, "pkg", ".env"].iter().collect();
            assert!(
                filter.should_exclude(&nested),
                "{} must be excluded",
                nested.display()
            );
        }
    }

    /// ⚠️ A component is an exact match, so a directory whose name merely
    /// *ends with* one of these is no longer skipped.
    ///
    /// ⓘ These three are the real over-matches, found by measuring rather than
    /// by guessing: `redist/` contains `dist/`, `my_target/` contains
    /// `target/`, `prebuild/` contains `build/`. Each was silently excluded
    /// from every scan. ⚠️ My first attempt at this test used
    /// `my-node_modules-notes` and `distribution`, which the old code did
    /// **not** match — the extra text has to precede the name, not follow it.
    /// The negative control is what caught that.
    #[test]
    fn a_directory_whose_name_ends_with_an_excluded_one_is_still_scanned() {
        let filter = FileFilter::new(&[]);
        for dir in ["redist", "my_target", "prebuild", "vendor_node_modules"] {
            let nested: PathBuf = ["project", dir, ".env"].iter().collect();
            assert!(
                !filter.should_exclude(&nested),
                "{} is not a build directory and must be scanned",
                nested.display()
            );
        }
    }

    /// The template family is matched by **file name**, not as a substring, so
    /// a file that merely begins with one of the names is still scanned.
    #[test]
    fn only_the_template_file_itself_is_excluded() {
        let filter = FileFilter::new(&[]);
        assert!(filter.should_exclude(Path::new(".env.example")));
        // ⚠️ `.contains(".env.example")` matched both of these too. A copy of a
        // template with a real key pasted in is exactly what should be read.
        assert!(!filter.should_exclude(Path::new(".env.example.bak")));
        assert!(!filter.should_exclude(Path::new("my.env.example-notes")));
    }

    /// A user's `--exclude node_modules/` — written the way `.gitignore` and
    /// every tutorial write it — must work on a Windows path too.
    #[test]
    fn a_user_pattern_with_forward_slashes_matches_a_backslash_path() {
        let filter = FileFilter::new(&["vendor/cache/".to_string()]);

        // A backslash path, as `to_string_lossy` produces on Windows. On Unix
        // this is one oddly-named file, and matching it is still the answer the
        // user asked for by typing that pattern.
        assert!(filter.should_exclude(Path::new(r"app\vendor\cache\.env")));

        // The forward-slash spelling keeps working, unchanged.
        assert!(filter.should_exclude(Path::new("app/vendor/cache/.env")));

        // And an unrelated path is still scanned.
        assert!(!filter.should_exclude(Path::new("app/src/.env")));
    }

    /// Windows-only, and the assertion the fix exists for. ⓘ It runs only once
    /// `windows-latest` is in CI's matrix — which it was not when this was
    /// written, despite a note claiming otherwise.
    #[test]
    #[cfg(windows)]
    fn windows_paths_hit_the_hardcoded_directory_exclusions() {
        let filter = FileFilter::new(&[]);
        assert!(filter.should_exclude(Path::new(r"project\node_modules\pkg\.env")));
        assert!(filter.should_exclude(Path::new(r"project\.git\config")));
        assert!(filter.should_exclude(Path::new(r"C:\work\app\target\debug\.env")));
        assert!(!filter.should_exclude(Path::new(r"project\src\.env")));
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
