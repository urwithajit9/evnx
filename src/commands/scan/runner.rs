//! Scan orchestration and execution logic.
//!
//! This module coordinates the scanning process:
//! 1. Collect files using [`FileFilter`](super::filters::FileFilter)
//! 2. Scan each file using [`DetectorRegistry`](super::detector::DetectorRegistry)
//! 3. Aggregate results into [`ScanResults`](super::models::ScanResults)
//! 4. Output results using [`render()`](super::output::render)
//!
//! # Example
//!
//! ```
//! use evnx::commands::scan::ScanRunner;
//! use evnx::commands::scan::OutputFormat;
//!
//! let runner = ScanRunner::new(&[], false, false);
//! runner.run(vec!["./src".to_string()], OutputFormat::Pretty).unwrap();
//! ```

use super::walk::{
    Decoded, STDIN_LABEL, decode, decode_bytes, is_notebook, is_pkcs12, named_like_dotenv,
    structured_format,
};

/// The id of a PKCS#12 keystore finding (C3). Found by file content, so it is
/// not a regex rule in `rules.toml`.
pub const PKCS12_ID: &str = "pkcs12-keystore";
use super::{
    detector::DetectorRegistry,
    filters::FileFilter,
    models::UnreadableFile,
    models::{Confidence, Finding, ScanResults},
    output::{OutputFormat, render},
};
use crate::utils::fs::display_portable;
use crate::utils::ui;
use anyhow::Result;
// use colored::*;
use std::path::{Path, PathBuf};

/// A value shows characters only when a built-in format rule matched it and it
/// is at least this long: a known public prefix is then most of what is shown.
const MIN_PREVIEW_LEN: usize = 20;

/// Characters of the end shown after the prefix.
const PREVIEW_TAIL: usize = 2;

/// Prefixes that name a provider and are public by design (`ghp_`, `AKIA`).
const PUBLIC_VALUE_PREFIXES: &[&str] = &[
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "AKIA",
    "ASIA",
    "sk_live_",
    "sk_test_",
    "rk_live_",
    "sk-ant-",
    "sk-proj-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xoxs-",
    "xapp-",
    "AIza",
    "glpat-",
    "npm_",
    "hf_",
    "dckr_pat_",
    "whsec_",
    "sntrys_",
    "sntryu_",
    "SG.",
];

/// Main orchestrator for secret scanning operations.
///
/// Holds configuration and coordinates all scanning components.
///
/// # Fields
///
/// * `registry` - Detector registry with all active detection strategies
/// * `filter` - File filter for collecting scannable files
/// * `ignore_placeholders` - Whether to skip placeholder values
/// * `verbose` - Whether to print verbose progress information
///
/// # Example
///
/// ```
/// use evnx::commands::scan::ScanRunner;
/// let runner = ScanRunner::new(
///     &["node_modules".to_string()],
///     true,  // ignore placeholders
///     false, // not verbose
/// );
/// ```
pub struct ScanRunner {
    registry: DetectorRegistry,
    filter: FileFilter,
    ignore_placeholders: bool,
    verbose: bool,
    /// Lowest confidence worth reporting. `Low` reports everything.
    min_confidence: Confidence,
    /// Lowest severity worth reporting (C2). `Low` reports everything.
    min_severity: crate::core::secretscan::Severity,
    /// Findings accepted earlier (C5): reported as `known`, not counted.
    baseline: super::suppress::Baseline,
    /// `--update-baseline`: write this run's findings here instead of
    /// reporting them (C5).
    update_baseline: Option<std::path::PathBuf>,
    /// `--update-baseline --reason <code>`: given to the entries added now.
    baseline_reason: Option<String>,
    /// `(built-in, custom)` rule counts and every rule's name, for `-v`
    /// (I-058). The rules themselves live in the registry.
    rule_counts: (usize, usize),
    rule_names: Vec<String>,
    /// Where an extension list came from, for the run's announcement.
    extensions_from: &'static str,
    /// Report findings in walked files git ignores and does not track
    /// (`--include-ignored`). Off: they are summarised and not counted (I-285).
    include_ignored: bool,
    /// What this run was set to leave out, recorded in its report.
    effective_config: super::models::EffectiveConfig,
    /// How many of evnx's own files under `.evnx/` were left out
    /// ([`super::filters::is_evnx_own_file`]), for the one note that says so.
    evnx_own: std::sync::atomic::AtomicUsize,
}

impl ScanRunner {
    /// Record the settings that narrow this scan, for the report.
    pub fn with_effective_config(mut self, effective: super::models::EffectiveConfig) -> Self {
        self.effective_config = effective;
        self
    }

    /// Read symlinked files in a walk when their target is inside the
    /// scanned folder (`--follow-symlinks`).
    pub fn with_include_dirs(mut self, names: &[String]) -> Self {
        self.filter = self.filter.with_include_dirs(names);
        self
    }

    /// Report short values under strong secret names (WP-13, D5); see
    /// [`DetectorRegistry::with_short_values`].
    pub fn with_short_values(mut self, on: bool) -> Self {
        self.registry = std::mem::take(&mut self.registry).with_short_values(on);
        self
    }

    pub fn with_follow_symlinks(mut self, follow: bool) -> Self {
        self.filter = self.filter.with_follow_symlinks(follow);
        self
    }

    /// What `[scan] disable` turned off beyond the rule list (I-056): see
    /// [`DetectorRegistry::with_disabled`].
    pub fn with_disabled(
        mut self,
        formats: super::patternset::PatternSet,
        names: &[String],
    ) -> Self {
        self.registry = self.registry.with_disabled(formats, names);
        self
    }

    /// Count findings in files git ignores, as `--include-ignored` asks.
    pub fn with_include_ignored(mut self, include_ignored: bool) -> Self {
        self.include_ignored = include_ignored;
        self
    }

    /// Skip walked files larger than `bytes` (`--max-file-size`,
    /// `[scan] max_file_size`); `None` keeps [`super::filters::MAX_WALK_BYTES`].
    /// The baseline to compare against, and where `--update-baseline`
    /// writes (C5).
    pub fn with_baseline(
        mut self,
        baseline: super::suppress::Baseline,
        update: Option<std::path::PathBuf>,
    ) -> Self {
        self.baseline = baseline;
        self.update_baseline = update;
        self
    }

    /// The reason code `--update-baseline` gives to the entries it adds
    /// (`--reason`, SCAN-REASONS).
    pub fn with_baseline_reason(mut self, reason: Option<String>) -> Self {
        self.baseline_reason = reason;
        self
    }

    /// Report only findings at or above `severity` (`--severity`, C2).
    pub fn with_min_severity(mut self, severity: crate::core::secretscan::Severity) -> Self {
        self.min_severity = severity;
        self
    }

    pub fn with_max_file_size(mut self, bytes: Option<u64>) -> Self {
        if let Some(bytes) = bytes {
            self.filter = self.filter.with_max_bytes(bytes);
        }
        self
    }

    /// Limit folder walks to `extensions`, announcing it as coming from
    /// `from` (`--extensions`, `.evnx.toml [scan] extensions`). `None` reads
    /// every file (I-047, DEC-U8-2).
    pub fn with_extensions(mut self, extensions: Option<(Vec<String>, &'static str)>) -> Self {
        if let Some((list, from)) = extensions {
            self.filter = self.filter.with_extensions(Some(list));
            self.extensions_from = from;
        }
        self
    }

    /// Create a new ScanRunner with configuration.
    ///
    /// # Arguments
    ///
    /// * `exclude` - File exclusion patterns
    /// * `ignore_placeholders` - Skip placeholder values (e.g., "changeme", "example")
    /// * `verbose` - Enable verbose progress output
    ///
    /// # Example
    ///
    /// ```
    /// # use evnx::commands::scan::ScanRunner;
    /// let runner = ScanRunner::new(&["*.log".to_string()], false, true);
    /// ```
    pub fn new(exclude: &[String], ignore_placeholders: bool, verbose: bool) -> Self {
        Self::with_min_confidence(exclude, ignore_placeholders, verbose, Confidence::Low)
    }

    /// As [`ScanRunner::new`], but reporting only findings at or above
    /// `min_confidence` — what `--severity` sets.
    ///
    /// Filtering happens at the point a finding is recorded rather than at render
    /// time, so every count, the JSON `summary`, and the process exit code all
    /// describe the same filtered set. A scanner whose exit code disagreed with
    /// its own output would be worse than one without the flag.
    pub fn with_min_confidence(
        exclude: &[String],
        ignore_placeholders: bool,
        verbose: bool,
        min_confidence: Confidence,
    ) -> Self {
        Self::with_spec(
            exclude,
            ignore_placeholders,
            verbose,
            min_confidence,
            Default::default(),
        )
    }

    /// As [`ScanRunner::with_min_confidence`], plus the project's declared
    /// contract — `secret = true` reports a credential the heuristics cannot
    /// name, and `secret = false` retracts a guess the project has looked at.
    pub fn with_spec(
        exclude: &[String],
        ignore_placeholders: bool,
        verbose: bool,
        min_confidence: Confidence,
        spec: crate::core::spec::Spec,
    ) -> Self {
        Self::with_spec_and_patterns(
            exclude,
            ignore_placeholders,
            verbose,
            min_confidence,
            spec,
            super::patternset::PatternSet::empty(),
        )
    }

    /// As [`ScanRunner::with_spec`], plus the formats this project declared
    /// itself — `--pattern` and `[[scan.patterns]]`, already compiled.
    ///
    /// The set is compiled by the caller rather than here so that a bad
    /// expression fails before the scan starts, with nothing printed that could
    /// be read as a result. See `scan::run`.
    pub fn with_spec_and_patterns(
        exclude: &[String],
        ignore_placeholders: bool,
        verbose: bool,
        min_confidence: Confidence,
        spec: crate::core::spec::Spec,
        patterns: super::patternset::PatternSet,
    ) -> Self {
        let rule_counts = patterns.counts();
        let rule_names = patterns.names().into_iter().map(str::to_string).collect();
        Self {
            registry: DetectorRegistry::new()
                .with_spec(spec)
                .with_patterns(patterns),
            filter: FileFilter::new(exclude),
            ignore_placeholders,
            verbose,
            min_confidence,
            min_severity: crate::core::secretscan::Severity::Low,
            baseline: Default::default(),
            update_baseline: None,
            baseline_reason: None,
            rule_counts,
            rule_names,
            extensions_from: "",
            include_ignored: false,
            effective_config: super::models::EffectiveConfig {
                severity_floor: "low".to_string(),
                confidence_floor: "low".to_string(),
                ..Default::default()
            },
            evnx_own: Default::default(),
        }
    }

    /// Leave evnx's own files out of `paths` (from the repository root, as
    /// git names them), unless one was named on the command line. Counted for
    /// the note [`ScanRunner::finish`] prints.
    fn keep_project_file(&self, path: &str, named: &[String]) -> bool {
        if !super::filters::is_evnx_own_file(Path::new(path)) || named.iter().any(|n| n == path) {
            return true;
        }
        self.evnx_own
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        false
    }

    // In commands/scan/runner.rs

    /// Run the scan on the given paths.
    ///
    /// # Arguments
    ///
    /// * `paths` - Vector of file or directory paths to scan
    /// * `format` - Output format for results
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Scan completed, secrets were found
    /// * `Ok(false)` - Scan completed, no secrets found
    /// * `Err(e)` - Scan failed with error
    ///
    /// # Note
    ///
    /// This method does NOT call `std::process::exit()`. The caller
    /// is responsible for exit code handling based on configuration.
    ///
    /// # Example
    ///
    /// ```ignore
    /// # use evnx::commands::scan::ScanRunner;
    /// # use evnx::commands::scan::OutputFormat;
    /// let runner = ScanRunner::new(&[], false, false);
    /// let found_secrets = runner.run(vec!["./src".to_string()], OutputFormat::Pretty)?;
    /// if found_secrets {
    ///     println!("⚠️  Secrets detected!");
    /// }
    /// ```
    pub fn run(&self, paths: Vec<String>, format: OutputFormat) -> Result<bool> {
        // RM-48: `-` is stdin, read once and scanned in memory; nothing of it
        // is written to disk. It can sit beside paths.
        let stdin = paths.iter().any(|p| p == "-");
        let on_disk: Vec<String> = paths.iter().filter(|p| *p != "-").cloned().collect();
        let collected = if on_disk.is_empty() {
            super::filters::Collected::default()
        } else {
            self.filter.collect(&on_disk)?
        };
        let files = collected.files.clone();
        self.evnx_own
            .store(collected.evnx_own, std::sync::atomic::Ordering::Relaxed);
        // ⚠️ After the paths are checked (WP-10): a banner followed by "does
        // not exist" reads as a scan that started.
        // Only show header for human-readable output
        // Machine formats (JSON/SARIF) skip the UI header to keep stdout clean
        if matches!(format, OutputFormat::Pretty) {
            ui::print_header("evnx scan", Some("Checking for exposed secrets"));
        } else if self.verbose {
            // For machine formats, at least acknowledge start in verbose mode
            ui::verbose_stderr("Starting scan...");
        }

        // I-058: the 13 built-ins were announced as "13 custom pattern(s)", and
        // the first line counted the paths given and called them files.
        if self.verbose {
            use crate::utils::string::pluralize;
            ui::verbose_stderr(format!(
                "Scanning {}...",
                pluralize(paths.len(), "path", "paths")
            ));
            let (builtin, custom) = self.rule_counts;
            ui::verbose_stderr(format!(
                "{}, {}",
                pluralize(builtin, "built-in rule", "built-in rules"),
                pluralize(custom, "custom pattern", "custom patterns")
            ));
            if !self.rule_names.is_empty() {
                ui::verbose_stderr(format!("rules: {}", self.rule_names.join(", ")));
            }
        }

        // Said before scanning: a limited walk is a weaker scan (DEC-U8-2).
        if let Some(list) = self.filter.extensions() {
            eprintln!(
                "  scanning only files ending in {} in folders ({}); other files are not read",
                list.iter()
                    .map(|e| format!(".{e}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                self.extensions_from
            );
        }

        // ⚠️ A scan that examined nothing is not a clean scan.
        //
        // `collect_files` already refuses a path that does not exist. This is
        // the other way to end up with an empty list: every candidate was
        // filtered out. Left alone, the run printed
        //
        //     ✓  No secrets detected
        //     0 files scanned
        //
        // and exited 0 — which is what CI reads. The count was on screen and
        // the exit code contradicted it.
        //
        // Exit 2, not 1: nothing was found because nothing was looked at, which
        // is "could not run" rather than "ran and found nothing". A `--severity`
        // threshold that hides every finding is a different case and still 0.
        if files.is_empty() && !stdin {
            // ⚠️ Names what was set aside and why (I-048, I-289). This said
            // "filtered out" and advised `--include`, a flag that never
            // existed — and when the walk had skipped a file for its size, the
            // reason it gave (`.git`, `node_modules`) was not the one that
            // applied.
            let mut why = String::new();
            if !collected.unreadable.is_empty() {
                why.push_str("Could not read:\n");
                for (path, reason) in &collected.unreadable {
                    why.push_str(&format!("  {}  {reason}\n", display_portable(path)));
                }
                why.push('\n');
            }
            if !collected.skipped_dirs.is_empty() {
                why.push_str(&format!(
                    "Skipped folders: {}. --include-dir NAME walks one.\n\n",
                    collected.skipped_dirs.join(", ")
                ));
            }
            if !collected.skipped.is_empty() {
                why.push_str("Skipped:\n");
                for (path, reason) in &collected.skipped {
                    why.push_str(&format!("  {}  {reason}\n", display_portable(path)));
                }
                why.push_str(
                    "--max-file-size raises the size limit; a binary file is never read.\n\n",
                );
            }
            if let Some(list) = self.filter.extensions() {
                why.push_str(&format!(
                    "{} limits folder walks to files ending in {}.\n\n",
                    self.extensions_from,
                    list.join(", ")
                ));
            }
            anyhow::bail!(
                "nothing was scanned — every file in the paths given was skipped or excluded.\n\n\
                 {why}Folder walks always skip .git, node_modules, target, dist, build, .next, \
                 .nuxt, .turbo and __pycache__ below the folder scanned (unless git tracks files \
                 in them), \
                 Python virtualenvs, Go/PHP/Ruby vendor folders, \
                 the .env.example family, evnx's own files in .evnx/, and what --exclude or \
                 [scan] exclude names. \
                 Name a file directly to scan it regardless.\n\n\
                 Paths given: {}",
                paths.join(", ")
            );
        }

        if self.verbose {
            ui::verbose_stderr(format!("Found {} files to scan", files.len()));
        }

        // ⚠️ Zero, not `files.len()`. The count is of files **read**, and it
        // used to be the number *found* — which is how `1 file scanned` came to
        // be printed about a file that was never decoded.
        let mut results = ScanResults::new(0);
        results.skipped_dirs = collected.skipped_dirs.clone();
        for (path, reason) in &collected.unreadable {
            results.unreadable.push(UnreadableFile {
                path: display_portable(path),
                reason: reason.clone(),
            });
        }
        for (path, reason) in &collected.skipped {
            results.skipped.push(UnreadableFile {
                path: display_portable(path),
                reason: reason.clone(),
            });
        }

        // What git says about each file (I-285): asked once per repository.
        // `None` everywhere outside one, which leaves the report as it was.
        let git = crate::core::gitignore::exposures(&files);
        if matches!(format, OutputFormat::Pretty)
            && !crate::utils::ui::is_quiet()
            && !collected.skipped_dirs.is_empty()
        {
            eprintln!(
                "  · skipped dependency folders: {} (--include-dir NAME walks one)",
                collected.skipped_dirs.join(", ")
            );
        }
        // Without a repository there is no ignore list to apply: the same tree
        // reports more here than inside one. Said once, so that a different
        // result between a laptop and a build image has an explanation.
        if matches!(format, OutputFormat::Pretty)
            && !crate::utils::ui::is_quiet()
            && !files.is_empty()
            && git.iter().all(Option::is_none)
        {
            eprintln!(
                "  · not a git repository: git-ignore rules do not apply, every file is reported"
            );
        }

        // RM-74: files are scanned in parallel, each into a results of its own,
        // and merged in `files` order (sorted by path), so the report is the
        // same whatever the thread count. `RAYON_NUM_THREADS` caps the pool.
        //
        // ⚠️ Nothing a file's scan does is shared: its findings, notes and
        // warnings land in its own results. Notes are printed by `finish`, in
        // order, so stderr is as deterministic as stdout.
        use rayon::prelude::*;
        let scanned: Vec<(ScanResults, bool, bool)> = files
            .par_iter()
            .zip(git.par_iter())
            .map(|(file, view)| {
                let mut local = ScanResults::new(0);
                if self.verbose {
                    local.log.push((
                        false,
                        format!(
                            "Scanning: {} ({})",
                            file.display(),
                            Self::read_as(file).describe_file(file)
                        ),
                    ));
                }
                let walked = !collected.named.contains(file);
                let source = collected.read_through.get(file).unwrap_or(file);
                let outcome = self.scan_file(file, source, walked, &mut local);
                // I-062: line, then rule. A `.env` is read twice — parsed, then the
                // lines the parser dropped — so a commented-out key on line 1 used
                // to come after a finding on line 5.
                //
                // A notebook is already in cell order: a line is within its cell.
                if !is_notebook(file) {
                    local
                        .findings
                        .sort_by(|a, b| (a.line, &a.pattern).cmp(&(b.line, &b.pattern)));
                }
                let read = local.skipped.is_empty();

                // ⚠️ Held back only when git positively says ignored **and** not
                // tracked, and only for a walk. A tracked file is in the
                // repository whatever `.gitignore` says now (I-157); a file the
                // user named is the file they asked about; and no answer from git
                // is not "ignored". Each of those is reported exactly as before.
                let ignored = view.as_ref().map(|v| v.exposure)
                    == Some(crate::core::gitignore::Exposure::Protected);
                let held = walked && ignored && !self.include_ignored;
                if held {
                    local.hold_back(0, display_portable(file));
                } else if let Some(view) = view {
                    for finding in &mut local.findings {
                        finding.git = Some(view.exposure.into());
                        finding.repo_path = Some(view.repo_path.clone());
                        if view.exposure == crate::core::gitignore::Exposure::Tracked {
                            finding.in_commit = Some(view.committed);
                        }
                    }
                }
                outcome.map(|()| (local, read, held))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<Result<_>>()?;

        // The files actually read: `scanning …` must not name one that was
        // then set aside as binary.
        let mut read = Vec::with_capacity(files.len());
        let mut summarised = Vec::new();
        for (file, (local, was_read, held)) in files.iter().zip(scanned) {
            if was_read {
                read.push(file.clone());
                if held {
                    summarised.push(file.clone());
                }
            }
            results.merge(local);
        }
        results.summarised = summarised;

        if stdin {
            self.scan_stdin(&mut results)?;
            read.push(Path::new(STDIN_LABEL).to_path_buf());
        }
        self.finish(results, format, &read)
    }

    /// `evnx scan --staged` (C9): the index blobs a commit would record, under
    /// `paths` (pathspecs) when any are given.
    pub fn run_staged(&self, paths: Vec<String>, format: OutputFormat) -> Result<bool> {
        if paths.iter().any(|p| p == "-") {
            return Err(crate::core::verdict::usage(
                "--staged reads the git index; it cannot be combined with `-` (stdin)",
            ));
        }
        let cwd = std::env::current_dir()?;
        let top = super::git_blobs::toplevel(&cwd, "--staged")?;
        let paths: Vec<String> = paths.into_iter().filter(|p| p != ".").collect();
        let named = named_in_repo(&cwd, &paths);
        let mut staged = super::staged::staged(&cwd, &paths, self.filter.max_bytes())?;
        staged.retain(|f| self.keep_project_file(&f.path, &named));
        if matches!(format, OutputFormat::Pretty) {
            ui::print_header("evnx scan", Some("Checking what is staged for commit"));
        }
        let mut results = ScanResults::new(0);
        if staged.is_empty() {
            // The one empty case that is clean: no commit content, nothing to leak.
            if matches!(format, OutputFormat::Pretty) {
                ui::success("nothing staged, nothing scanned");
                return Ok(false);
            }
            return self.finish(results, format, &[]);
        }
        // The same rule as a walk: a file some commit holds keeps the history
        // advice, one in no commit gets "unstage it". No answer from git
        // leaves `in_commit` unknown.
        let names: Vec<String> = staged.iter().map(|f| f.path.clone()).collect();
        let committed = crate::core::gitignore::committed_among(&top, &names);
        let mut read = Vec::with_capacity(staged.len());
        for file in staged {
            let Some(first) =
                self.scan_loaded(&file.path, file.bytes, file.size, true, &mut results)?
            else {
                continue;
            };
            read.push(PathBuf::from(&file.path));
            // ⚠️ No git hold-back: a staged file is committed whatever
            // `.gitignore` says.
            for finding in &mut results.findings[first..] {
                finding.git = Some(super::models::GitState::Staged);
                finding.repo_path = Some(file.path.clone());
                finding.in_commit = committed.as_ref().map(|c| c.contains(&file.path));
            }
        }
        self.finish(results, format, &read)
    }

    /// The blobs `--history` would read: the range's, less what `--exclude` and
    /// `--extensions` leave out.
    pub fn history_entries(
        &self,
        paths: Vec<String>,
        range: &super::history::Range,
    ) -> Result<Vec<super::history::Entry>> {
        if paths.iter().any(|p| p == "-") {
            return Err(crate::core::verdict::usage(
                "--history reads git commits; it cannot be combined with `-` (stdin)",
            ));
        }
        let cwd = std::env::current_dir()?;
        super::git_blobs::toplevel(&cwd, "--history")?;
        let named = named_in_repo(&cwd, &paths);
        let mut entries = super::history::blobs(&cwd, range, &paths)?;
        entries.retain(|e| {
            self.filter.selects_in_history(Path::new(&e.path))
                && self.keep_project_file(&e.path, &named)
        });
        Ok(entries)
    }

    /// Scan `entries` (from [`ScanRunner::history_entries`]) and report.
    /// Returns whether anything counted, and how many blobs were read.
    pub fn scan_history(
        &self,
        entries: Vec<super::history::Entry>,
        range: &super::history::Range,
        format: OutputFormat,
    ) -> Result<(bool, usize)> {
        use rayon::prelude::*;
        let cwd = std::env::current_dir()?;
        if matches!(format, OutputFormat::Pretty) {
            ui::print_header(
                "evnx scan",
                Some(&format!("Checking history: {}", range.label())),
            );
        }
        let mut results = ScanResults::new(0);
        if entries.is_empty() {
            if matches!(format, OutputFormat::Pretty) {
                ui::success("no files in that range, nothing scanned");
                return Ok((false, 0));
            }
            return self.finish(results, format, &[]).map(|found| (found, 0));
        }
        let total = entries.len();
        let started = std::time::Instant::now();
        let mut told = std::time::Duration::ZERO;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut read: Vec<PathBuf> = Vec::new();
        let mut done = 0usize;
        // A chunk at a time: a long history is not held in memory whole.
        for chunk in entries.chunks(HISTORY_CHUNK) {
            let shas: Vec<String> = chunk.iter().map(|e| e.blob.clone()).collect();
            let blobs = super::git_blobs::read_blobs(&cwd, &shas, self.filter.max_bytes())?;
            let scanned: Vec<(ScanResults, bool)> = chunk
                .par_iter()
                .zip(blobs)
                .map(|(entry, blob)| {
                    let mut local = ScanResults::new(0);
                    let first =
                        self.scan_loaded(&entry.path, blob.bytes, blob.size, false, &mut local)?;
                    Ok((local, first.is_some()))
                })
                .collect::<Vec<Result<_>>>()
                .into_iter()
                .collect::<Result<_>>()?;
            for (entry, (mut local, was_read)) in chunk.iter().zip(scanned) {
                if was_read {
                    read.push(PathBuf::from(&entry.path));
                }
                annotate_history(&mut local, entry, &mut seen);
                results.merge(local);
            }
            done += chunk.len();
            // Said once the scan is slow enough to look stuck.
            if started.elapsed() >= told + std::time::Duration::from_secs(2)
                && !ui::is_quiet()
                && done < total
            {
                told = started.elapsed();
                eprintln!("  scanning history: {done} of {total} files");
            }
        }
        let n = read.len();
        self.finish(results, format, &read).map(|found| (found, n))
    }

    /// Scan one blob that git handed over (`--staged`, `--history`) as the
    /// file `path`. `bytes` is `None` for a blob over the size cap. Returns
    /// where this file's findings start in `results.findings`, or `None`
    /// when it was set aside (too large, or not text) and so was not read.
    fn scan_loaded(
        &self,
        path: &str,
        bytes: Option<Vec<u8>>,
        size: u64,
        announce: bool,
        results: &mut ScanResults,
    ) -> Result<Option<usize>> {
        let label = PathBuf::from(path);
        let Some(bytes) = bytes else {
            results.skipped.push(UnreadableFile {
                path: path.to_string(),
                reason: format!(
                    "larger than {} ({})",
                    super::filters::size_label(self.filter.max_bytes()),
                    super::filters::size_label(size)
                ),
            });
            return Ok(None);
        };
        if announce && self.verbose {
            ui::scanning_file_stderr(&label, Self::read_as(&label).describe_file(&label));
        }
        let first = results.findings.len();
        if super::walk::is_pkcs12_bytes(&bytes) {
            self.add_pkcs12(&label, results);
        } else {
            match decode_bytes(bytes) {
                Decoded::Text(c) => self.scan_text(&label, &c, Self::read_as(&label), results)?,
                // Binary handling as for a walk (I-047).
                Decoded::Unreadable { reason } if !FileFilter::is_scannable(&label) => {
                    results.skipped.push(UnreadableFile {
                        path: path.to_string(),
                        reason: format!("not text: {reason}"),
                    });
                    return Ok(None);
                }
                Decoded::Unreadable { reason } => {
                    results.unreadable.push(UnreadableFile {
                        path: path.to_string(),
                        reason,
                    });
                    return Ok(None);
                }
            }
        }
        results.findings[first..].sort_by(|a, b| (a.line, &a.pattern).cmp(&(b.line, &b.pattern)));
        Ok(Some(first))
    }

    /// Report `results`, or write them as the baseline: shared by a walk and
    /// `--staged`.
    fn finish(
        &self,
        mut results: ScanResults,
        format: OutputFormat,
        read: &[PathBuf],
    ) -> Result<bool> {
        results.effective_config = Some(self.effective_config.clone());
        if self.evnx_own.load(std::sync::atomic::Ordering::Relaxed) > 0
            && matches!(format, OutputFormat::Pretty)
            && !ui::is_quiet()
        {
            eprintln!("  · evnx's own files in .evnx/ are not scanned (name one to scan it)");
        }
        // What the scan had to say, in the order it arose (RM-74).
        for (warning, line) in std::mem::take(&mut results.log) {
            if warning {
                ui::warning(line);
            } else {
                ui::verbose_stderr(line);
            }
        }
        // `--update-baseline` (C5): this run's counted and known findings
        // become the baseline, and the run exits 0.
        if let Some(path) = &self.update_baseline {
            let current: Vec<(String, String, String)> = results
                .findings
                .iter()
                .chain(&results.known)
                .map(|f| {
                    // A history finding's location is `path@abc1234:LINE`; the
                    // baseline names the file, as a walk would.
                    let file = f
                        .commit
                        .as_ref()
                        .and(f.repo_path.clone())
                        .unwrap_or_else(|| super::output::file_of(&f.location).to_string());
                    (f.fingerprint.clone(), f.rule_id.clone(), file)
                })
                .collect();
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let next = self.baseline.rewrite(
                &current,
                &today,
                self.baseline_reason.as_deref(),
                Some(self.effective_config.rules_digest.as_str()).filter(|d| !d.is_empty()),
            );
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                crate::core::safe_write::create_dirs(dir)?;
            }
            let json = serde_json::to_string_pretty(&next)? + "\n";
            crate::core::safe_write::write_shared(path, json.as_bytes())?;
            ui::success(format!(
                "wrote {} with {} finding(s); they are reported as known from now on",
                display_portable(path),
                next.findings.len()
            ));
            return Ok(false);
        }

        // Render output (goes to stdout)
        render(&results, format, read)?;
        // After the report, so the note is not wedged between the stderr
        // header and the stdout file list.
        report_skipped(&results, self.verbose, self.filter.max_bytes());
        // C5: `-v` lists what markers and the baseline set aside.
        if self.verbose {
            for f in results.allowed.iter().chain(&results.known) {
                let code = f
                    .reason_code
                    .as_deref()
                    .map(|c| format!(" [{c}]"))
                    .unwrap_or_default();
                let why = match (f.status, &f.reason) {
                    (super::models::Status::Allowed, Some(r)) => format!("allowed{code}: {r}"),
                    (super::models::Status::Allowed, None) if !code.is_empty() => {
                        format!("allowed{code}")
                    }
                    (super::models::Status::Allowed, None) => "allowed, no reason given".into(),
                    _ => format!("known (baseline){code}"),
                };
                ui::verbose_stderr(format!("{}  {}  {why}", f.location, f.rule_id));
            }
        }

        // Return status for caller to handle exit code
        // ⛔ An unreadable file fails the scan.
        //
        // `scan` exists to answer "is it safe to push this?", and the honest
        // answer for a file it could not read is "I do not know" — which must
        // not share an exit code with "no". Exiting 0 there is what let a live
        // AWS key through a CI gate that was working exactly as configured.
        Ok(results.secrets_found > 0 || !results.unreadable.is_empty())
    }

    /// Scan a `.env` through the shared parser, read lossily: a file with a
    /// bad line is still read entry by entry around it.
    ///
    /// Which detectors a file gets is decided by its name alone (RM-47).
    fn read_as(path: &Path) -> super::models::ReadAs {
        if named_like_dotenv(path) {
            super::models::ReadAs::Dotenv
        } else if structured_format(path).is_some() || is_notebook(path) {
            super::models::ReadAs::Structured
        } else {
            super::models::ReadAs::Text
        }
    }

    /// `evnx scan -` (RM-48): what is piped in, read as a `.env` when it parses
    /// as one and as text otherwise.
    fn scan_stdin(&self, results: &mut ScanResults) -> Result<()> {
        use anyhow::Context as _;
        use std::io::{IsTerminal as _, Read as _};

        if std::io::stdin().is_terminal() {
            anyhow::bail!(
                "`evnx scan -` reads what is piped into it, and nothing is. \
                 Pipe a file, a diff or a log in: `git diff | evnx scan -`"
            );
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut bytes)
            .context("reading stdin")?;
        // ⛔ Empty is not clean: `cat missing-file | evnx scan -` pipes nothing,
        // and "No secrets detected", exit 0, would pass that CI step.
        if bytes.is_empty() {
            anyhow::bail!(
                "nothing was piped into `evnx scan -`, so nothing was scanned. \
                 Check the command before the pipe."
            );
        }
        let label = Path::new(STDIN_LABEL);
        let content = match decode_bytes(bytes) {
            Decoded::Text(c) => c,
            Decoded::Unreadable { reason } => {
                results.unreadable.push(UnreadableFile {
                    path: STDIN_LABEL.to_string(),
                    reason,
                });
                return Ok(());
            }
        };
        let read_as = if crate::core::envfile::is_dotenv(&content) {
            super::models::ReadAs::Dotenv
        } else {
            super::models::ReadAs::Text
        };
        if self.verbose {
            ui::scanning_file_stderr(label, read_as.describe_file(label));
        }
        self.scan_text(label, &content, read_as, results)
    }

    /// C3: a keystore is binary, so it is found by its content, not read.
    fn add_pkcs12(&self, path: &Path, results: &mut ScanResults) {
        results.files_scanned += 1;
        results.files.push(super::models::ScannedFile {
            path: display_portable(path),
            read_as: super::models::ReadAs::Text,
            note: Some("binary: a PKCS#12 keystore".to_string()),
        });
        let detection = super::detector::Detection {
            rule_id: PKCS12_ID.to_string(),
            pattern: "PKCS#12 Keystore".to_string(),
            confidence: Confidence::High,
            severity: crate::core::secretscan::Severity::Critical,
            matched_value: PKCS12_ID.to_string(),
            ..Default::default()
        };
        self.add_finding(
            results,
            path,
            crate::core::secretscan::engine::Found {
                line: 1,
                variable: None,
                detection,
                encoded: None,
            },
            &[],
            false,
        );
    }

    /// Scan `path`, reading its content from `source`: the same file, except
    /// for a followed symlink, which is read through the path that was checked.
    fn scan_file(
        &self,
        path: &Path,
        source: &Path,
        walked: bool,
        results: &mut ScanResults,
    ) -> Result<()> {
        // C3: a keystore is binary, so it is found by its content, not read.
        if is_pkcs12(source) {
            self.add_pkcs12(path, results);
            return Ok(());
        }
        let content = match decode(source) {
            Decoded::Text(c) => c,
            // ⛔ **A file this command chose to scan and could not read is a
            // reportable failure, not a skip.**
            //
            // This was:
            //
            // ```text
            // let content = match std::fs::read_to_string(path) {
            //     Ok(c) => c,
            //     Err(_) => return Ok(()),   // Skip binary/unreadable files
            // };
            // ```
            //
            // so a `.env` saved as UTF-16 — which is what a Windows editor
            // produces when you pick "Unicode" — reported:
            //
            // ```text
            // ✓  No secrets detected
            //    1 file scanned                            exit 0
            // ```
            //
            // over a live `AWS_SECRET_ACCESS_KEY`. The count came from
            // `files.len()`, so it said "scanned" about a file it had not read.
            //
            // This is the third time this shape has appeared: `is_scannable`
            // skipping `.env.production` (task 23) and `validate --fix`
            // reporting "All checks passed" over a credential it had
            // destroyed. CLAUDE.md's note on task 23 is the rule — a scanner
            // that misses is **worse than useless, because it reports clean**.
            // A file a walk found, of a kind not expected to be text (an image,
            // an archive), is skipped and said — not a failure (I-047). One
            // that *should* be text (`.env`, `.yml`, …) still fails, as above.
            Decoded::Unreadable { reason } if walked && !FileFilter::is_scannable(path) => {
                results.skipped.push(UnreadableFile {
                    path: display_portable(path),
                    reason: format!("not text: {reason}"),
                });
                return Ok(());
            }
            Decoded::Unreadable { reason } => {
                results.unreadable.push(UnreadableFile {
                    path: display_portable(path),
                    reason,
                });
                return Ok(());
            }
        };

        self.scan_text(path, &content, Self::read_as(path), results)
    }

    /// Scan text already read, from a file or stdin, labelled `path`.
    fn scan_text(
        &self,
        path: &Path,
        content: &str,
        read_as: super::models::ReadAs,
        results: &mut ScanResults,
    ) -> Result<()> {
        use super::models::ReadAs;
        use crate::core::secretscan::engine;
        results.files_scanned += 1;

        // F14-6: a notebook is JSON whose strings hide a secret behind
        // escapes, so it is read cell by cell, unescaped.
        if read_as == ReadAs::Structured && is_notebook(path) {
            if let Some(chunks) = engine::notebook_chunks(content) {
                return self.scan_notebook(path, chunks, results);
            }
        }

        // C8: parsed once here, so a file that does not parse is recorded as
        // read as text, and is never an error.
        let format = (read_as == ReadAs::Structured)
            .then(|| structured_format(path))
            .flatten();
        let leaves = format.and_then(|f| engine::leaves(content, f));
        let (read_as, note) = match (format, &leaves) {
            (Some(f), None) => (ReadAs::Text, Some(format!("did not parse as {f}"))),
            _ if read_as == ReadAs::Structured && is_notebook(path) => (
                ReadAs::Text,
                Some("did not parse as a Jupyter notebook".to_string()),
            ),
            _ => (read_as, None),
        };
        if self.verbose {
            if let Some(note) = &note {
                results.log.push((
                    false,
                    format!("{}: {note}, read as text", display_portable(path)),
                ));
            }
        }
        let lockfile = super::walk::is_lockfile(path);
        if lockfile && self.verbose {
            results.log.push((
                false,
                format!("{}: {}", display_portable(path), super::walk::LOCKFILE_NOTE),
            ));
        }
        let note = match (note, lockfile) {
            (Some(n), true) => Some(format!("{}; {n}", super::walk::LOCKFILE_NOTE)),
            (None, true) => Some(super::walk::LOCKFILE_NOTE.to_string()),
            (n, false) => n,
        };
        results.files.push(super::models::ScannedFile {
            path: display_portable(path),
            read_as,
            note,
        });
        let label = display_portable(path);
        let ctx = engine::Context {
            path: &label,
            kind: match (read_as, format) {
                (ReadAs::Dotenv, _) => engine::Kind::Dotenv,
                (ReadAs::Structured, Some(f)) => engine::Kind::Structured(f),
                _ => engine::Kind::Text,
            },
            document: None,
            leaves: leaves.as_deref(),
            registry: &self.registry,
            rules_only: lockfile,
        };
        // Detection is the engine's; what a detection becomes is the report's.
        let lines: Vec<&str> = content.lines().collect();
        let structured = matches!(ctx.kind, engine::Kind::Structured(_));
        for f in engine::scan_text(content, &ctx) {
            self.add_finding(results, path, f, &lines, structured);
        }

        Ok(())
    }

    /// Scan a notebook's cells (F14-6). Each chunk is read as text, so a value
    /// is found once, where it is: the location is `file:LINE (cells[N]…)`,
    /// LINE being within that cell's source or output.
    fn scan_notebook(
        &self,
        path: &Path,
        chunks: Vec<crate::core::secretscan::engine::NotebookChunk>,
        results: &mut ScanResults,
    ) -> Result<()> {
        use super::models::ReadAs;
        use crate::core::secretscan::engine;
        let label = display_portable(path);
        results.files_scanned += 1;
        results.files.push(super::models::ScannedFile {
            path: label.clone(),
            read_as: ReadAs::Structured,
            note: Some("Jupyter notebook: cells and outputs read unescaped".to_string()),
        });
        for chunk in chunks {
            let ctx = engine::Context {
                path: &label,
                kind: engine::Kind::Text,
                document: None,
                leaves: None,
                registry: &self.registry,
                rules_only: false,
            };
            let lines: Vec<&str> = chunk.text.lines().collect();
            for f in engine::scan_text(&chunk.text, &ctx) {
                self.add_finding(
                    results,
                    path,
                    crate::core::secretscan::engine::Found {
                        variable: Some(chunk.path.clone()),
                        ..f
                    },
                    &lines,
                    true,
                );
            }
        }
        Ok(())
    }

    /// Add a finding to results with proper truncation and filtering.
    ///
    /// # Arguments
    ///
    /// * `results` - Mutable results to append to
    /// * `path` - File path for location context
    /// * `line` - Line number
    /// * `variable` - Optional variable name
    /// * `detection` - Detection from a detector
    fn add_finding(
        &self,
        results: &mut ScanResults,
        path: &Path,
        found: crate::core::secretscan::engine::Found,
        lines: &[&str],
        structured: bool,
    ) {
        let crate::core::secretscan::engine::Found {
            line,
            variable,
            detection,
            encoded,
        } = found;
        // Skip placeholders if configured.
        //
        // ⚠️ `looks_like_placeholder_secret`, NOT `is_placeholder`. The latter
        // answers "is this safe to overwrite?" for `validate --fix` and was
        // anchored in #121 so it could not destroy a real value — which left
        // it unable to recognise a placeholder EMBEDDED in a detector match,
        // and made this flag inert. See the doc comment on the new function
        // for why the two must not be merged back together.
        if self.ignore_placeholders
            && crate::core::secretscan::heuristic::looks_like_placeholder_secret(
                &detection.matched_value,
            )
        {
            return;
        }

        // ── A secret behind a build-time public prefix ────────────────────────
        //
        // ⚠️ This must run **before** the `--severity` filter below, not after.
        // The case it exists for is a low-confidence value behind
        // `NEXT_PUBLIC_`: filtering first would discard it at `--severity
        // medium` and leave nothing to escalate.
        //
        // Escalation is the smaller half of this. Verified against the real
        // detectors: a Stripe key behind `VITE_` and a long value behind
        // `NEXT_PUBLIC_TOKEN` are **already** high, so what was actually wrong
        // is the advice — see `output::print_next_steps`, which told the reader
        // to rewrite git history for a key that is compiled into every bundle
        // they have already shipped.
        // ⚠️ NOT gated on `judged_value`. The first version was, and it excluded
        // the clearest case there is: `NEXT_PUBLIC_TOKEN` is a name-only match,
        // so it was skipped — a variable whose name says "token" and whose prefix
        // says "published". `judged_value` governs whether we may claim something
        // about the *value* ("matches a live key format"); the prefix is a fact
        // about the *name*, which is exactly what a name-only match has.
        let public = variable
            .as_deref()
            .and_then(crate::core::secretscan::rules::public_prefix);

        let mut detection = detection;
        if public.is_some() {
            // F14-5: a secret that ships in every bundle is worse than the
            // same secret kept on a server.
            detection.severity = detection.severity.escalated_for_public_prefix();
            detection.confidence = match detection.action_url {
                // A named provider format — this is a real key, and the prefix
                // means it is already public. Nothing about it is low risk.
                Some(_) => Confidence::High,
                // Shape alone. It may be a deliberately publishable key (a
                // PostHog or Sentry DSN is meant to ship), so `Medium` rather
                // than `High`: enough to be seen and read, not enough to fail a
                // `--severity high` gate on something that may be fine.
                None => detection.confidence.max(Confidence::Medium),
            };
        }

        // Below the --severity threshold: not recorded at all.
        if detection.confidence < self.min_confidence || detection.severity < self.min_severity {
            return;
        }

        // SCAN-CONTEXT: usage and test-path hints move the level shown, never
        // the decision. The floor above and the counts and exit code
        // (`Finding::decided_confidence`) use `detection.confidence` as it was.
        let before = detection.confidence;
        let (shown, context) = super::context::adjust(
            &detection.rule_id,
            before,
            lines
                .get(line.wrapping_sub(1))
                .is_some_and(|l| super::context::is_usage(l)),
            super::context::is_test_path(&display_portable(path)),
        );

        let judged_value = detection.judged_value;
        // I-052: a connection string is not a provider key, so say which kind
        // of URL holds the password rather than "matches a live key format".
        let note = (detection.pattern == CONNECTION_STRING_RULE)
            .then(|| detection.matched_value.split_once("://"))
            .flatten()
            .map(|(scheme, _)| format!("a password in a {scheme}:// URL, not a placeholder"));
        // I-051: `----…----` said nothing. A PEM header is a label, not key
        // material, so it is shown whole — the body never reaches a detection.
        let preview = if detection.rule_id == PKCS12_ID {
            "binary keystore (private key and certificate)".to_string()
        } else {
            pem_header(&detection.matched_value)
                .unwrap_or_else(|| preview_value(&detection.rule_id, &detection.matched_value))
        };
        let rule = (detection.rule_id.clone(), detection.severity);
        let mut finding = Finding::new(
            detection.pattern,
            shown,
            preview,
            if let Some(ref var) = variable {
                format!("{}:{} ({})", display_portable(path), line, var)
            } else {
                format!("{}:{}", display_portable(path), line)
            },
            variable,
            detection.action_url,
        )
        .rule(rule.0, rule.1)
        .judged_value(judged_value)
        .public_prefix(public.map(|(prefix, _)| prefix.to_string()));
        finding.note = note;
        finding.encoded = encoded.map(str::to_string);
        if shown != before {
            finding.confidence_before_context = Some(before);
        }
        finding.context = context.into_iter().map(str::to_string).collect();
        finding.line = line;
        // The key path a structured file's `variable` is, kept apart from a
        // `.env` variable's name so a script need not parse `location`.
        if structured {
            finding.key_path = finding.variable.clone();
        }

        // C5: what the project has looked at. The fingerprint never holds the
        // value itself: the line's text goes through BLAKE3.
        let line_text = lines.get(line.wrapping_sub(1)).copied().unwrap_or("");
        finding.fingerprint = super::suppress::fingerprint(
            &finding.rule_id,
            &display_portable(path),
            finding.variable.as_deref(),
            line_text,
        );
        let here = display_portable(path);
        if let Some(m) = super::suppress::allowing(lines, line, &finding.rule_id) {
            if let Some(code) = m
                .reason_code
                .as_deref()
                .filter(|c| !super::suppress::is_known_reason(c))
            {
                results.log.push((
                    true,
                    format!(
                        "{here}:{line}: evnx:allow {}: {}",
                        finding.rule_id,
                        super::suppress::unknown_reason_warning(code)
                    ),
                ));
            }
            if m.reason.is_none() && m.reason_code.is_none() {
                results.log.push((
                    true,
                    format!(
                        "{here}:{line}: evnx:allow {} has no reason; say why after it",
                        finding.rule_id
                    ),
                ));
            }
            finding.status = super::models::Status::Allowed;
            finding.reason = m.reason;
            finding.reason_code = m.reason_code;
            results.allowed.push(finding);
            return;
        }
        // A marker here that allows nothing: bare, or for another rule.
        for n in [line, line.saturating_sub(1)]
            .into_iter()
            .filter(|&n| n >= 1)
        {
            if let Some(m) = lines.get(n - 1).and_then(|l| super::suppress::marker_in(l)) {
                match m.rule {
                    None => results.log.push((
                        true,
                        format!(
                            "{here}:{n}: a bare evnx:allow allows nothing; name the rule: evnx:allow {}",
                            finding.rule_id
                        ),
                    )),
                    Some(other) => results.log.push((
                        true,
                        format!(
                            "{here}:{n}: evnx:allow {other} does not match this finding, which is {}",
                            finding.rule_id
                        ),
                    )),
                }
            }
        }
        if let Some(entry) = self.baseline.get(&finding.fingerprint) {
            finding.reason_code = entry.reason.clone();
            finding.status = super::models::Status::Known;
            results.known.push(finding);
            return;
        }

        results.add_finding(finding);
    }

    // /// Print the scan header UI.
    // fn print_header(&self) {
    //     println!(
    //         "\n{}",
    //         "┌─ Scanning for exposed secrets ──────────────────────┐".cyan()
    //     );
    //     println!(
    //         "{}",
    //         "│ Checking for real-looking credentials               │".cyan()
    //     );
    //     println!(
    //         "{}\n",
    //         "└──────────────────────────────────────────────────────┘".cyan()
    //     );
    // }
}

/// Blobs read from git in one batch by `--history`.
const HISTORY_CHUNK: usize = 256;

/// Mark what one blob's scan found as history (RM-69): the commit, the blob
/// and the time, `path@abc1234:LINE` for where, and the same secret seen again
/// in a later blob dropped (the file was edited, the secret stayed).
fn annotate_history(
    local: &mut ScanResults,
    entry: &super::history::Entry,
    seen: &mut std::collections::HashSet<String>,
) {
    let short: String = entry.commit.chars().take(7).collect();
    let mark = |f: &mut Finding| {
        // `path:LINE (VAR)` becomes `path@abc1234:LINE (VAR)`.
        if let Some(rest) = f
            .location
            .strip_prefix(&display_portable(Path::new(&entry.path)))
        {
            f.location = format!("{}@{short}{rest}", display_portable(Path::new(&entry.path)));
        }
        f.commit = Some(entry.commit.clone());
        f.blob = Some(entry.blob.clone());
        f.commit_time = Some(entry.time);
        f.git = Some(super::models::GitState::History);
        f.repo_path = Some(entry.path.clone());
    };
    let counted = std::mem::take(&mut local.findings);
    local.secrets_found = 0;
    local.high_confidence = 0;
    local.medium_confidence = 0;
    local.low_confidence = 0;
    for mut f in counted {
        if seen.insert(f.fingerprint.clone()) {
            mark(&mut f);
            local.add_finding(f);
        }
    }
    for list in [&mut local.known, &mut local.allowed] {
        list.retain(|f| seen.insert(f.fingerprint.clone()));
        list.iter_mut().for_each(mark);
    }
}

/// The built-in rule for a password inside a URL (`core::secretscan::rules`).
const CONNECTION_STRING_RULE: &str = "Credentials in a connection string";

/// The paths named on the command line, from the repository root and
/// `/`-separated, as git names a file in `--staged` and `--history`. Lexical:
/// a file named for `--history` may no longer be on disk.
fn named_in_repo(cwd: &Path, paths: &[String]) -> Vec<String> {
    let top = super::git_blobs::toplevel(cwd, "scan")
        .ok()
        .and_then(|t| std::fs::canonicalize(t).ok());
    let here = std::fs::canonicalize(cwd).ok();
    let prefix = match (&here, &top) {
        (Some(h), Some(t)) => h.strip_prefix(t).map(Path::to_path_buf).unwrap_or_default(),
        _ => PathBuf::new(),
    };
    paths
        .iter()
        .map(|p| {
            let mut parts: Vec<String> = Vec::new();
            for c in prefix.join(p).components() {
                match c {
                    std::path::Component::Normal(n) => parts.push(n.to_string_lossy().into_owned()),
                    std::path::Component::ParentDir => {
                        parts.pop();
                    }
                    _ => {}
                }
            }
            parts.join("/")
        })
        .collect()
}

/// `-----BEGIN RSA PRIVATE KEY-----` when `value` is exactly a PEM private-key
/// header — what the Private Key rule matches — and `None` otherwise (I-051).
fn pem_header(value: &str) -> Option<String> {
    let v = value.trim();
    (v.starts_with("-----BEGIN ")
        && v.ends_with("-----")
        && v.contains("PRIVATE KEY")
        && v.len() <= 80
        && !v.contains('\n'))
    .then(|| v.to_string())
}

/// What a finding shows of the value it found.
///
/// Nothing but its length (`<24 chars>`), unless a built-in format rule matched
/// a value of `MIN_PREVIEW_LEN` or more characters that starts with a known
/// public prefix: then that prefix and the last `PREVIEW_TAIL` characters.
/// The finding already names the file, line and variable; the value adds
/// recognition, not identity, and no length or shape of value is safe to show
/// part of by default.
///
/// # Example
///
/// ```
/// # use evnx::commands::scan::runner::preview_value;
/// assert_eq!(preview_value("aws-access-key", "AKIA4OZRMFJ3VREALKEY"), "AKIA…EY");
/// assert_eq!(preview_value("high-entropy-string", "AKIA4OZRMFJ3VREALKEY"), "<20 chars>");
/// assert_eq!(preview_value("aws-access-key", "tiny"), "<4 chars>");
/// ```
pub fn preview_value(rule_id: &str, value: &str) -> String {
    let n = value.chars().count();
    let format_rule = crate::core::secretscan::rules::builtin_rules()
        .iter()
        .any(|r| r.id.as_deref() == Some(rule_id));
    if format_rule && n >= MIN_PREVIEW_LEN {
        if let Some(prefix) = PUBLIC_VALUE_PREFIXES
            .iter()
            .find(|p| value.starts_with(**p))
        {
            let tail: String = value.chars().skip(n - PREVIEW_TAIL).collect();
            return format!("{prefix}…{tail}");
        }
    }
    format!("<{n} chars>")
}

/// Say what a walk did not read (I-047, DEC-U8-2): a count always, each file
/// with `-v`. On stderr, so `--format json` stays one document.
fn report_skipped(results: &ScanResults, verbose: bool, max_bytes: u64) {
    if results.skipped.is_empty() {
        return;
    }
    eprintln!(
        "  skipped {} file(s): not text, over {}, or symlinks{}",
        results.skipped.len(),
        super::filters::size_label(max_bytes),
        if verbose { ":" } else { " (-v lists them)" }
    );
    if verbose {
        for f in &results.skipped {
            eprintln!("    {}  {}", f.path, f.reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─────────────────────────────────────────────────────────
    // S3 — a file that was not read is not clean
    // ─────────────────────────────────────────────────────────

    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for u in text.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    fn utf16be(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFE, 0xFF] } else { Vec::new() };
        for u in text.encode_utf16() {
            out.extend_from_slice(&u.to_be_bytes());
        }
        out
    }

    /// ⛔ **A `.env` saved as UTF-16 scanned clean with exit 0.**
    ///
    /// Two different routes to the same false all-clear, which is why fixing
    /// one is not enough:
    ///
    /// * **With a BOM**, `read_to_string` fails on `0xFF`, and the `Err` arm
    ///   was `return Ok(())`.
    /// * **Without one it succeeds**, because UTF-16LE ASCII *is* valid UTF-8:
    ///   every byte is an ASCII character or `0x00`, and `0x00` is a legal
    ///   UTF-8 code unit. The scanner got `"S\0T\0R\0I\0P\0E\0"` and
    ///   genuinely found nothing in it.
    #[test]
    fn a_utf16_env_file_is_decoded_and_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let line = "STRIPE_SECRET_KEY=sk_live_51H8xQ2eZvKYlo2CpR4mN7bV9\n";

        for (name, bytes) in [
            ("bom-le", utf16le(line, true)),
            ("no-bom-le", utf16le(line, false)),
            ("bom-be", utf16be(line, true)),
            ("no-bom-be", utf16be(line, false)),
        ] {
            let path = write(dir.path(), &format!(".env.{name}"), &bytes);
            match decode(&path) {
                Decoded::Text(text) => assert!(
                    text.contains("STRIPE_SECRET_KEY") && text.contains("sk_live_"),
                    "{name} decoded to something unusable: {text:?}"
                ),
                Decoded::Unreadable { reason } => {
                    panic!("{name} was not decoded: {reason}")
                }
            }
        }
    }

    /// A UTF-8 byte-order mark is not an encoding problem, but left in the
    /// string it becomes part of the first key's name.
    #[test]
    fn a_utf8_bom_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"TOKEN=abc\n");
        let path = write(dir.path(), ".env", &bytes);
        match decode(&path) {
            Decoded::Text(t) => assert!(t.starts_with("TOKEN="), "BOM survived: {t:?}"),
            Decoded::Unreadable { reason } => panic!("{reason}"),
        }
    }

    /// ⛔ The general rule, and the one the old code broke: a file this command
    /// selected and could not read must **say so**, not return quietly.
    #[test]
    fn an_undecodable_file_is_reported_rather_than_skipped() {
        let dir = tempfile::tempdir().unwrap();

        // Binary with NULs that is not UTF-16.
        let bin = write(
            dir.path(),
            ".env.bin",
            &[0x00, 0xFF, 0x13, 0x37, 0x00, 0x01],
        );
        assert!(matches!(decode(&bin), Decoded::Unreadable { .. }));

        // Latin-1: a high byte, no NULs, so `from_utf8` rejects it.
        let latin = write(dir.path(), ".env.latin1", b"PW=caf\xe9_value\n");
        assert!(matches!(decode(&latin), Decoded::Unreadable { .. }));

        // A file that does not exist at all is an io error, not a silent pass.
        assert!(matches!(
            decode(&dir.path().join("nope")),
            Decoded::Unreadable { .. }
        ));
    }

    /// Odd-length UTF-16 cannot be decoded, and must not be truncated into
    /// something that looks scanned.
    #[test]
    fn odd_length_utf16_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), ".env", &[0xFF, 0xFE, 0x41]);
        assert!(matches!(decode(&path), Decoded::Unreadable { .. }));
    }

    #[test]
    fn a_preview_shows_a_known_prefix_and_two_characters_for_a_format_rule() {
        assert_eq!(
            preview_value("aws-access-key", "AKIA4OZRMFJ3VREALKEY"),
            "AKIA…EY"
        );
        let gh = format!("{}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        assert_eq!(preview_value("github-token", &gh), "ghp_…R8");
    }

    #[test]
    fn everything_else_is_a_length() {
        // Not a format rule: a name check, entropy, a custom rule id.
        for id in ["sensitive-config-key", "high-entropy-string", "my-rule"] {
            assert_eq!(preview_value(id, "AKIA4OZRMFJ3VREALKEY"), "<20 chars>");
        }
        // Format rule, no known prefix.
        assert_eq!(
            preview_value("jwt", "this_is_a_very_long_secret_key_value_12345678"),
            "<45 chars>"
        );
        // Format rule, known prefix, under 20 characters.
        assert_eq!(preview_value("aws-access-key", "AKIA12345"), "<9 chars>");
        assert_eq!(preview_value("aws-access-key", ""), "<0 chars>");
    }

    #[test]
    fn a_preview_is_utf8_safe() {
        assert_eq!(
            preview_value("jwt", "🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑"),
            "<12 chars>"
        );
    }

    #[test]
    fn test_scan_runner_new() {
        let runner = ScanRunner::new(&["test".to_string()], true, true);
        assert!(runner.ignore_placeholders);
        assert!(runner.verbose);
    }
    #[test]
    fn test_scan_runner_returns_secrets_found() {
        // This test would need a fixture with a known secret
        // For now, just verify the method signature works
        let _runner = ScanRunner::new(&[], false, false);
        // Note: Full integration test requires test fixtures
    }
}
