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
    /// Plain `--exclude` values, as path components (I-053): `test` is
    /// `["test"]`, `vendor/cache/` is `["vendor", "cache"]`.
    exclude_components: Vec<Vec<String>>,
    compiled_globs: Vec<Pattern>,
    /// The opt-in extension list (`--extensions`, `[scan] extensions`).
    /// `None`, the default, reads every file a walk finds (I-047, DEC-U8-2).
    extensions: Option<Vec<String>>,
    /// A walked file larger than this is skipped and said (I-061).
    max_bytes: u64,
}

/// The default limit on a walked file, which `--max-file-size` and
/// `[scan] max_file_size` change (I-061, DEC-SC4). A `.env` or config file is a
/// few kilobytes; a megabyte of text is a log, a bundle or data — and named on
/// the command line it is still read.
pub const MAX_WALK_BYTES: u64 = 1024 * 1024;

/// Does `value` hold a glob metacharacter? Then `--exclude` reads it as a glob.
fn is_glob(value: &str) -> bool {
    value.contains(['*', '?', '['])
}

/// `1024 KiB`, `5120 KiB`, `500 bytes`: the limit as the skip note says it.
pub fn size_label(bytes: u64) -> String {
    if bytes >= 1024 && bytes % 1024 == 0 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

/// `path`'s names, without `.` and the root — what a plain `--exclude` value
/// is compared against, one whole component at a time.
fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// What a walk found: the files to read, and the ones it passed over and why.
#[derive(Debug, Default)]
pub struct Collected {
    pub files: Vec<PathBuf>,
    /// `(path, reason)` for files a walk did not read.
    pub skipped: Vec<(PathBuf, String)>,
    /// Files named on the command line, read whatever the rules say.
    pub named: std::collections::HashSet<PathBuf>,
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
            .filter(|p| is_glob(p))
            .filter_map(|p| Pattern::new(p).ok())
            .collect();
        // ⚠️ Split on `/` only, like the patterns people copy from
        // `.gitignore`. A `\` stays part of a name, so on Unix — where it is a
        // legal filename character — `vendor\cache` names one odd component
        // rather than widening into two.
        let exclude_components = exclude
            .iter()
            .filter(|p| !is_glob(p))
            .map(|p| {
                p.split('/')
                    .filter(|part| !part.is_empty() && *part != ".")
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|parts| !parts.is_empty())
            .collect();

        Self {
            exclude_components,
            compiled_globs,
            extensions: None,
            max_bytes: MAX_WALK_BYTES,
        }
    }

    /// Skip walked files larger than `bytes` (I-061). Named files are read
    /// whatever their size.
    pub fn with_max_bytes(mut self, bytes: u64) -> Self {
        self.max_bytes = bytes;
        self
    }

    /// The size limit in force for walked files.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Limit a folder walk to these extensions (without the dot, any case).
    /// `None` reads every file. Named files are always read.
    pub fn with_extensions(mut self, extensions: Option<Vec<String>>) -> Self {
        self.extensions = extensions.map(|v| {
            v.iter()
                .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
                .collect()
        });
        self
    }

    /// The extension list in force, if any.
    pub fn extensions(&self) -> Option<&[String]> {
        self.extensions.as_deref()
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
        Ok(self.collect(paths)?.files)
    }

    /// [`FileFilter::collect_files`], keeping what a walk skipped and which
    /// files were named, so the scan can say what it did not read.
    pub fn collect(&self, paths: &[String]) -> Result<Collected> {
        let mut collected = Collected::default();
        let mut files: Vec<PathBuf> = Vec::new();
        // ⚠️ Collected rather than bailed on immediately, so `evnx scan a b c`
        // names every bad path in one run instead of one per invocation.
        let mut unreadable = Vec::new();

        // I-054: one entry per file however it was reached — `scan . .`,
        // `scan . .env`, `scan a a/b`. Keyed on the canonical path; the first
        // spelling is the one shown.
        let mut seen: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
        let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());

        for path_str in paths {
            let path = Path::new(path_str);

            if path.is_file() {
                // Already reached by a walk: it is now a named file too, which
                // decides how it is reported (I-285), but it is read once.
                match seen.get(&canonical(path)).copied() {
                    // A walk set it aside for size; named, it is read anyway.
                    Some(usize::MAX) => {
                        let key = canonical(path);
                        collected.skipped.retain(|(f, _)| canonical(f) != key);
                    }
                    Some(at) => {
                        collected.named.insert(files[at].clone());
                        continue;
                    }
                    None => {}
                }
                seen.insert(canonical(path), files.len());
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
                collected.named.insert(path.to_path_buf());
            } else if path.is_dir() {
                // ⚠️ Sorted by name. Without it the walk follows the order the
                // filesystem returns, which differs between machines (ext4
                // hashes directory entries), so one project printed a different
                // file list and findings order in CI than locally (I-243, G14).
                for entry in WalkDir::new(path)
                    .follow_links(false)
                    .sort_by_file_name()
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let entry_path = entry.path();
                    if !entry_path.is_file()
                        || self.should_exclude(entry_path)
                        || !self.walk_selects(entry_path)
                    {
                        continue;
                    }
                    // ⚠️ The default reads every file (I-047, DEC-U8-2): the old
                    // extension allow-list never opened `config/id_rsa.pem` or a
                    // key in `app.swift`, and the scan reported clean. What it
                    // does not read, it says — see `Collected::skipped`.
                    if seen.contains_key(&canonical(entry_path)) {
                        continue;
                    }
                    if self.extensions.is_none() {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        if size > self.max_bytes {
                            seen.insert(canonical(entry_path), usize::MAX);
                            collected.skipped.push((
                                entry_path.to_path_buf(),
                                format!("larger than {}", size_label(self.max_bytes)),
                            ));
                            continue;
                        }
                    }
                    seen.insert(canonical(entry_path), files.len());
                    files.push(entry_path.to_path_buf());
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

        // I-062: by path, so output does not depend on the order paths were
        // given. Compared name by name, which is the order a walk already
        // produces — `a/x` before `a.txt` — so a single walk is unchanged.
        files.sort_by_key(|f| names(f));
        collected.skipped.sort_by_key(|(f, _)| names(f));
        collected.files = files;
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

        Ok(collected)
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
        // ⛔ **Normalised once, here, and every user pattern matches against
        // this rather than the raw string.**
        //
        // `glob::Pattern` treats `/` as its separator and nothing else, so on
        // Windows — where the walker yields `.\fixtures\.env` — a pattern like
        // `fixtures/**` could never match. `--exclude` was simply inert there.
        // That is the second half of the same defect as the hardcoded
        // directories below: #146 fixed those and the substring branch and left
        // the glob branch untouched.
        //
        // ⚠️ `display_portable` is **Windows-only by construction**, and that
        // matters more here than it does for output. On Unix `\` is a legal
        // filename character, so normalising unconditionally would let
        // `--exclude a/b` match a single file named `a\b` — **excluding a file
        // the user never named**. For a scanner an over-exclusion is a silent
        // miss, the one failure mode this command cannot afford.
        //
        // ⓘ The hardcoded directory list does not go through this at all. It
        // matches by path component, which the platform's own parser splits
        // correctly, and that is strictly better than normalising a string.
        let path_str = crate::utils::fs::display_portable(path);

        // Check compiled glob patterns
        if !self.compiled_globs.is_empty() {
            // ⚠️ After normalisation this strips on both platforms. Against the
            // raw Windows string it never did — the walker produces `.\…`, not
            // `./…`, so a pattern lost to the leading `.\` even when its
            // separators were right.
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

        // Plain values: whole path components (I-053).
        //
        // ⛔ This was a **substring of the whole path**: `--exclude env` dropped
        // `.env`, and `--exclude test` dropped `latest/` and `contest.py` — a
        // safe-looking exclusion hiding the file most worth scanning. A plain
        // name now matches a whole component, and `vendor/cache` the two
        // consecutive components, wherever they sit.
        //
        // ⓘ Components come from the platform's own parser, so `\` separates
        // on Windows and is part of a name on Unix — the distinction #146 and
        // the test below it pin.
        if !self.exclude_components.is_empty() {
            let have = names(path);
            if self
                .exclude_components
                .iter()
                .any(|want| have.windows(want.len()).any(|w| w == want.as_slice()))
            {
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
        // The template family, by the one list (`core::env_name`).
        if crate::core::env_name::kind(&name) == crate::core::env_name::FileKind::Template {
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

    /// Whether a folder walk reads `path`: with an extension list, the list
    /// (plus `.env*`, `Dockerfile`, `Makefile`); without one, every file but an
    /// editor's binary swap file.
    fn walk_selects(&self, path: &Path) -> bool {
        match &self.extensions {
            Some(list) => {
                let list: Vec<&str> = list.iter().map(String::as_str).collect();
                Self::matches(path, &list)
            }
            None => !Self::is_editor_swap(path),
        }
    }

    fn is_editor_swap(path: &Path) -> bool {
        const EDITOR_SWAP: &[&str] = &[".swp", ".swo", ".swn", ".swm", ".swl"];
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| EDITOR_SWAP.iter().any(|e| n.ends_with(e)))
    }

    /// The built-in text extensions — the list a walk was limited to before
    /// I-047. Still the answer to "is this a file we expect to be text?": a
    /// walked file on it that does not decode fails the scan; one off it is
    /// skipped as not text.
    pub fn is_scannable(path: &Path) -> bool {
        Self::matches(path, Self::TEXT_EXTENSIONS)
    }

    const TEXT_EXTENSIONS: &'static [&'static str] = &[
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

    /// `path` has one of `list`'s extensions, or a name that is always text
    /// (`.env*`, `Dockerfile`, `Makefile`) — never an editor swap file.
    fn matches(path: &Path, list: &[&str]) -> bool {
        // Check extension
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if list.contains(&ext_str.as_str()) && !Self::is_editor_swap(path) {
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
        assert_eq!(filter.exclude_components, vec![vec!["test".to_string()]]);
        assert_eq!(filter.compiled_globs.len(), 1); // Only *.log is a glob
    }

    #[test]
    fn test_should_exclude_substring() {
        let filter = FileFilter::new(&["node_modules".to_string()]);
        assert!(filter.should_exclude(Path::new("project/node_modules/package.json")));
        assert!(!filter.should_exclude(Path::new("project/src/main.rs")));
    }

    /// I-053: a plain value is a whole component, never a substring.
    #[test]
    fn t_i053_a_plain_value_is_a_component_not_a_substring() {
        let filter = FileFilter::new(&["env".to_string(), "test".to_string()]);
        for kept in [".env", "latest/a.py", "contest.py", "src/.env.test"] {
            assert!(!filter.should_exclude(Path::new(kept)), "{kept}");
        }
        for gone in ["test/a.py", "src/test", "a/env/b.txt", "./test/x"] {
            assert!(filter.should_exclude(Path::new(gone)), "{gone}");
        }
        let filter = FileFilter::new(&["./fixtures/".to_string()]);
        assert!(filter.should_exclude(Path::new("./fixtures/.env")));
        assert!(!filter.should_exclude(Path::new("./my-fixtures/.env")));
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

    /// A user's `--exclude vendor/cache/` — written the way `.gitignore` and
    /// every tutorial write it — works on both platforms, and the forward-slash
    /// spelling is unchanged everywhere.
    #[test]
    fn a_user_pattern_with_forward_slashes_still_matches_a_normal_path() {
        let filter = FileFilter::new(&["vendor/cache/".to_string()]);
        assert!(filter.should_exclude(Path::new("app/vendor/cache/.env")));
        assert!(!filter.should_exclude(Path::new("app/src/.env")));
    }

    /// ⛔ **A backslash means different things on the two platforms, and this
    /// pins both.**
    ///
    /// ⚠️ **This assertion was the other way round until 2026-10-13**, and the
    /// change is deliberate. #146 normalised separators *unconditionally*,
    /// which fixed Windows and quietly widened Unix: a file genuinely named
    /// `app\vendor\cache\.env` — one filename, backslashes and all, which is
    /// legal on Unix — started matching `--exclude vendor/cache/`.
    ///
    /// ⛔ That is an **over-exclusion**, and for a scanner an over-exclusion is
    /// a silent miss: the file is never read, nothing is reported, and the
    /// output says "no secrets detected". `SCANNER-REWORK-NOTES.md` names that
    /// as the asymmetric failure this command cannot afford. Normalising only
    /// where `\` cannot appear in a filename keeps Unix exactly as it was.
    #[test]
    fn a_backslash_path_is_a_filename_on_unix_and_a_path_on_windows() {
        let filter = FileFilter::new(&["vendor/cache/".to_string()]);
        let weird = Path::new(r"app\vendor\cache\.env");

        #[cfg(windows)]
        assert!(
            filter.should_exclude(weird),
            "on Windows this is a path and the pattern names it"
        );

        #[cfg(not(windows))]
        assert!(
            !filter.should_exclude(weird),
            "on Unix this is ONE file with backslashes in its name — excluding \
             it would skip a file the user never named, and a skipped file in a \
             secret scanner is a silent miss"
        );
    }

    /// ⛔ **The glob branch, which #146 left behind.**
    ///
    /// `glob::Pattern` treats `/` as its separator and nothing else, so against
    /// a raw `.\fixtures\.env` a pattern like `fixtures/**` matched nothing and
    /// `--exclude` was inert on Windows. Found by `scan_integration`'s
    /// `exclude_globs_match_the_forms_people_write` on the fourth round of
    /// bringing this platform up.
    ///
    /// ⓘ The leading `.\` mattered too: `strip_prefix("./")` could not strip
    /// it, so even a correctly-separated pattern lost to the prefix.
    #[test]
    #[cfg(windows)]
    fn windows_paths_hit_user_glob_patterns() {
        let filter = FileFilter::new(&["fixtures/**".to_string()]);
        assert!(filter.should_exclude(Path::new(r".\fixtures\.env")));
        assert!(filter.should_exclude(Path::new(r"fixtures\nested\.env")));
        assert!(!filter.should_exclude(Path::new(r".\src\.env")));
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

        let dir = [temp_dir.path().to_string_lossy().to_string()];

        // I-047 (DEC-U8-2): by default every file is collected; a binary one
        // is set aside when it is read, not by its name.
        let files = FileFilter::new(&[]).collect_files(&dir).unwrap();
        assert_eq!(files.len(), 2);

        // The extension list is opt-in, and then it is what decides.
        let files = FileFilter::new(&[])
            .with_extensions(Some(vec![".PY".into()]))
            .collect_files(&dir)
            .unwrap();
        assert_eq!(files, vec![file1]);
    }
}
