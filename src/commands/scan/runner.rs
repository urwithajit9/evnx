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
    decode, decode_bytes, is_notebook, is_pkcs12, named_like_dotenv, structured_format, Decoded,
    STDIN_LABEL,
};

/// The id of a PKCS#12 keystore finding (C3). Found by file content, so it is
/// not a regex rule in `rules.toml`.
pub const PKCS12_ID: &str = "pkcs12-keystore";
use super::{
    detector::DetectorRegistry,
    filters::FileFilter,
    models::UnreadableFile,
    models::{Confidence, Finding, ScanResults},
    output::{render, OutputFormat},
};
use crate::utils::fs::display_portable;
use crate::utils::ui;
use anyhow::Result;
// use colored::*;
use std::path::{Path, PathBuf};

/// Characters shown at each end of a masked secret.
const VISIBLE_EDGE: usize = 4;

/// Below this length no characters are shown at all — four of eight would be
/// half the secret.
///
/// ⚠️ There used to be a `MAX_VISIBLE = 20` here, under which a value was shown
/// whole. An AWS access key ID is exactly 20 characters, so the threshold
/// perfectly exempted the format the scanner detects most often. Length is not a
/// safety property; a short secret is not a less secret one.
const MIN_MASKABLE: usize = 8;

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
    /// `(built-in, custom)` rule counts and every rule's name, for `-v`
    /// (I-058). The rules themselves live in the registry.
    rule_counts: (usize, usize),
    rule_names: Vec<String>,
    /// Where an extension list came from, for the run's announcement.
    extensions_from: &'static str,
    /// Report findings in walked files git ignores and does not track
    /// (`--include-ignored`). Off: they are summarised and not counted (I-285).
    include_ignored: bool,
}

impl ScanRunner {
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
            rule_counts,
            rule_names,
            extensions_from: "",
            include_ignored: false,
        }
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
                 {why}Folder walks always skip .git, node_modules, target, dist and build, \
                 the .env.example family, and what --exclude or [scan] exclude names. \
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
        for (path, reason) in &collected.skipped {
            results.skipped.push(UnreadableFile {
                path: display_portable(path),
                reason: reason.clone(),
            });
        }

        // What git says about each file (I-285): asked once per repository.
        // `None` everywhere outside one, which leaves the report as it was.
        let git = crate::core::gitignore::exposures(&files);

        // RM-74: files are scanned in parallel, each into a results of its own,
        // and merged in `files` order (sorted by path), so the report is the
        // same whatever the thread count. `RAYON_NUM_THREADS` caps the pool.
        //
        // ⚠️ Nothing a file's scan does is shared: its findings, notes and
        // warnings land in its own results. Notes are printed by `finish`, in
        // order, so stderr is as deterministic as stdout.
        use rayon::prelude::*;
        let scanned: Vec<(ScanResults, bool)> = files
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
                let outcome = self.scan_file(file, walked, &mut local);
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
                if walked && ignored && !self.include_ignored {
                    local.hold_back(0, display_portable(file));
                } else if let Some(view) = view {
                    for finding in &mut local.findings {
                        finding.git = Some(view.exposure.into());
                        finding.repo_path = Some(view.repo_path.clone());
                    }
                }
                outcome.map(|()| (local, read))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<Result<_>>()?;

        // The files actually read: `scanning …` must not name one that was
        // then set aside as binary.
        let mut read = Vec::with_capacity(files.len());
        for (file, (local, was_read)) in files.iter().zip(scanned) {
            if was_read {
                read.push(file.clone());
            }
            results.merge(local);
        }

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
            anyhow::bail!("--staged reads the git index; it cannot be combined with `-` (stdin)");
        }
        let cwd = std::env::current_dir()?;
        super::staged::toplevel(&cwd)?;
        let paths: Vec<String> = paths.into_iter().filter(|p| p != ".").collect();
        let staged = super::staged::staged(&cwd, &paths, self.filter.max_bytes())?;
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
        let mut read = Vec::with_capacity(staged.len());
        for file in staged {
            let label = PathBuf::from(&file.path);
            let Some(bytes) = file.bytes else {
                results.skipped.push(UnreadableFile {
                    path: file.path.clone(),
                    reason: format!(
                        "larger than {} ({})",
                        super::filters::size_label(self.filter.max_bytes()),
                        super::filters::size_label(file.size)
                    ),
                });
                continue;
            };
            if self.verbose {
                ui::scanning_file_stderr(&label, Self::read_as(&label).describe_file(&label));
            }
            let first = results.findings.len();
            if super::walk::is_pkcs12_bytes(&bytes) {
                self.add_pkcs12(&label, &mut results);
            } else {
                match decode_bytes(bytes) {
                    Decoded::Text(c) => {
                        self.scan_text(&label, &c, Self::read_as(&label), &mut results)?
                    }
                    // Binary handling as for a walk (I-047).
                    Decoded::Unreadable { reason } if !FileFilter::is_scannable(&label) => {
                        results.skipped.push(UnreadableFile {
                            path: file.path.clone(),
                            reason: format!("not text: {reason}"),
                        });
                        continue;
                    }
                    Decoded::Unreadable { reason } => {
                        results.unreadable.push(UnreadableFile {
                            path: file.path.clone(),
                            reason,
                        });
                        continue;
                    }
                }
            }
            read.push(label);
            results.findings[first..]
                .sort_by(|a, b| (a.line, &a.pattern).cmp(&(b.line, &b.pattern)));
            // ⚠️ No git hold-back: a staged file is committed whatever
            // `.gitignore` says.
            for finding in &mut results.findings[first..] {
                finding.git = Some(super::models::GitState::Staged);
                finding.repo_path = Some(file.path.clone());
            }
        }
        self.finish(results, format, &read)
    }

    /// Report `results`, or write them as the baseline: shared by a walk and
    /// `--staged`.
    fn finish(
        &self,
        mut results: ScanResults,
        format: OutputFormat,
        read: &[PathBuf],
    ) -> Result<bool> {
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
                    let file = super::output::file_of(&f.location).to_string();
                    (f.fingerprint.clone(), f.rule_id.clone(), file)
                })
                .collect();
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let next = self.baseline.rewrite(&current, &today);
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
                let why = match (f.status, &f.reason) {
                    (super::models::Status::Allowed, Some(r)) => format!("allowed: {r}"),
                    (super::models::Status::Allowed, None) => "allowed, no reason given".into(),
                    _ => "known (baseline)".into(),
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
        self.add_finding(results, path, 1, None, detection, &[]);
    }

    fn scan_file(&self, path: &Path, walked: bool, results: &mut ScanResults) -> Result<()> {
        // C3: a keystore is binary, so it is found by its content, not read.
        if is_pkcs12(path) {
            self.add_pkcs12(path, results);
            return Ok(());
        }
        let content = match decode(path) {
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
        };
        // Detection is the engine's; what a detection becomes is the report's.
        let lines: Vec<&str> = content.lines().collect();
        for f in engine::scan_text(content, &ctx) {
            self.add_finding(results, path, f.line, f.variable, f.detection, &lines);
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
            };
            let lines: Vec<&str> = chunk.text.lines().collect();
            for f in engine::scan_text(&chunk.text, &ctx) {
                self.add_finding(
                    results,
                    path,
                    f.line,
                    Some(chunk.path.clone()),
                    f.detection,
                    &lines,
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
        line: usize,
        variable: Option<String>,
        detection: super::detector::Detection,
        lines: &[&str],
    ) {
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
                .unwrap_or_else(|| truncate_value(&detection.matched_value))
        };
        let rule = (detection.rule_id.clone(), detection.severity);
        let mut finding = Finding::new(
            detection.pattern,
            detection.confidence,
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
        finding.line = line;

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
            if m.reason.is_none() {
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
        if self.baseline.contains(&finding.fingerprint) {
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

/// The built-in rule for a password inside a URL (`core::secretscan::rules`).
const CONNECTION_STRING_RULE: &str = "Credentials in a connection string";

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

/// Truncate a value for safe display.
///
/// ⚠️ **Masks by policy, never by length.** This used to return the value
/// unchanged when it was `<= MAX_VISIBLE` (20) characters — and an AWS access key
/// ID is `AKIA` plus sixteen characters, **exactly 20**. So the single
/// most-detected credential format was printed in full, into terminal scrollback
/// and, through `scan --format json`, into CI artefacts.
///
/// # What is shown, and why so little
///
/// At most four leading and four trailing characters, and never more than half
/// the value. The preview exists so you can *recognise* which secret was found —
/// `AKIA…LKEY` is plainly an AWS key — not so you can verify it. The finding
/// already prints the variable name and the file and line, which is what actually
/// identifies it; the value adds recognition, not identity.
///
/// A value under [`MIN_MASKABLE`] characters shows no characters at all. Four of
/// eight is half a secret; there is no prefix short enough to be safe on a short
/// value, so it reports the length instead.
///
/// # Example
///
/// ```
/// # use evnx::commands::scan::runner::truncate_value;
/// assert_eq!(truncate_value("AKIA4OZRMFJ3VREALKEY"), "AKIA…LKEY");
/// assert_eq!(truncate_value("tiny"), "<4 chars>");
/// ```
pub fn truncate_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();

    if n < MIN_MASKABLE {
        // Nothing can be revealed safely, so reveal nothing.
        return format!("<{n} chars>");
    }

    // Never more than a quarter from each end, so at most half the value.
    let keep = VISIBLE_EDGE.min(n / 4);
    if keep == 0 {
        return format!("<{n} chars>");
    }

    let prefix: String = chars[..keep].iter().collect();
    let suffix: String = chars[n - keep..].iter().collect();
    format!("{prefix}…{suffix}")
}

/// Say what a walk did not read (I-047, DEC-U8-2): a count always, each file
/// with `-v`. On stderr, so `--format json` stays one document.
fn report_skipped(results: &ScanResults, verbose: bool, max_bytes: u64) {
    if results.skipped.is_empty() {
        return;
    }
    eprintln!(
        "  skipped {} file(s) that are not text or are over {}{}",
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

    /// ⚠️ These two tests asserted the bug. `truncate_value("short") == "short"`
    /// and `truncate_value("exactly20characters!")` returning its input whole
    /// were written as the specification, so the suite passed for as long as the
    /// scanner printed AWS keys in full.
    #[test]
    fn no_detected_value_is_ever_shown_whole() {
        for secret in [
            "AKIA4OZRMFJ3VREALKEY",                          // 20 — the bug
            "exactly20characters!",                          // 20 — the old test's own case
            "short",                                         // 5
            "sk_live_51Habc123def456",                       // 24
            "this_is_a_very_long_secret_key_value_12345678", // 45
        ] {
            let masked = truncate_value(secret);
            assert!(
                !masked.contains(secret),
                "{secret} was shown whole as {masked}"
            );
        }
    }

    /// Enough to recognise the provider, never enough to use.
    #[test]
    fn a_mask_keeps_the_shape_and_drops_the_secret() {
        assert_eq!(truncate_value("AKIA4OZRMFJ3VREALKEY"), "AKIA…LKEY");
        assert_eq!(
            truncate_value("this_is_a_very_long_secret_key_value_12345678"),
            "this…5678"
        );
    }

    /// Below `MIN_MASKABLE` there is no prefix short enough to be safe, so the
    /// length is reported instead of any characters.
    #[test]
    fn a_short_value_reveals_nothing_at_all() {
        assert_eq!(truncate_value("short"), "<5 chars>");
        assert_eq!(truncate_value("tiny"), "<4 chars>");
        assert_eq!(truncate_value(""), "<0 chars>");
    }

    /// Never more than half the value, whatever its length.
    #[test]
    fn at_most_half_the_characters_survive() {
        for n in 8..64 {
            let secret: String = std::iter::repeat_n('x', n).collect();
            let shown = truncate_value(&secret)
                .chars()
                .filter(|c| *c == 'x')
                .count();
            assert!(shown * 2 <= n, "{n}-char value revealed {shown} characters");
        }
    }

    /// Multi-byte input must not panic on a character boundary.
    #[test]
    fn masking_is_utf8_safe() {
        let masked = truncate_value("🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑🔑");
        assert!(masked.contains('…'), "{masked}");
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
