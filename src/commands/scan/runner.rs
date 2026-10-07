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

use super::{
    detector::DetectorRegistry,
    filters::FileFilter,
    models::UnreadableFile,
    models::{Confidence, Finding, ScanResults},
    output::{render, OutputFormat},
};
use crate::utils::ui;
use anyhow::Result;
// use colored::*;
use std::path::Path;

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
    /// How many custom patterns are active, for verbose output. The patterns
    /// themselves live in the registry.
    custom_patterns: usize,
}

impl ScanRunner {
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
        let custom_patterns = patterns.len();
        Self {
            registry: DetectorRegistry::new()
                .with_spec(spec)
                .with_patterns(patterns),
            filter: FileFilter::new(exclude),
            ignore_placeholders,
            verbose,
            min_confidence,
            custom_patterns,
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
            ui::print_header_stderr("evnx scan", Some("Checking for exposed secrets"));
        } else if self.verbose {
            // For machine formats, at least acknowledge start in verbose mode
            ui::verbose_stderr("Starting scan...");
        }

        if self.verbose {
            ui::verbose_stderr(format!("Scanning {} files...", paths.len()));
            if self.custom_patterns > 0 {
                ui::verbose_stderr(format!("{} custom pattern(s) active", self.custom_patterns));
            }
        }

        let files = self.filter.collect_files(&paths)?;

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
        if files.is_empty() {
            anyhow::bail!(
                "nothing was scanned — every path given was filtered out.\n\n\
                 Directory walks skip .git, node_modules, target, dist and build \
                 by default. Name a file directly to scan it regardless, or widen \
                 the walk with --include.\n\n\
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

        for file in &files {
            if self.verbose {
                ui::scanning_file_stderr(file);
            }
            self.scan_file(file, &mut results)?;
        }

        // Render output (goes to stdout)
        render(&results, format, &files)?;

        // Return status for caller to handle exit code
        // ⛔ An unreadable file fails the scan.
        //
        // `scan` exists to answer "is it safe to push this?", and the honest
        // answer for a file it could not read is "I do not know" — which must
        // not share an exit code with "no". Exiting 0 there is what let a live
        // AWS key through a CI gate that was working exactly as configured.
        Ok(results.secrets_found > 0 || !results.unreadable.is_empty())
    }

    /// Scan a single file for secrets.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to scan
    /// * `results` - Mutable results to append findings to
    ///
    /// # Returns
    ///
    /// Ok(()) on success. Skips files that can't be read as text.
    /// Scan a `.env` through the shared parser.
    ///
    /// `Some(())` when the file parsed and was scanned; `None` when it did not
    /// parse, leaving the caller to fall back to raw line scanning.
    ///
    /// ⚠️ The line reported for a multiline value is the line it **opened** on,
    /// matching `EnvFile::lines` and the convention parse errors already use.
    /// Pointing at the fragment that happened to match would send someone to the
    /// middle of a PEM block rather than to the assignment they need to edit.
    fn scan_env_parsed(&self, path: &Path, content: &str, results: &mut ScanResults) -> Option<()> {
        use crate::core::parser::{Parser, ParserConfig};

        let parser = Parser::new(ParserConfig {
            allow_expansion: false,
            ..Default::default()
        });
        let (vars, lines) = parser.parse_content_located(content).ok()?;

        for (key, value) in &vars {
            let line_num = lines.get(key).copied().unwrap_or(1);
            let location = format!("{}:{} ({})", path.display(), line_num, key);
            if let Some(detection) = Self::best(self.registry.scan_kv(key, value, &location)) {
                self.add_finding(results, path, line_num, Some(key.clone()), detection);
            }
        }
        Some(())
    }

    fn scan_file(&self, path: &Path, results: &mut ScanResults) -> Result<()> {
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
            Decoded::Unreadable { reason } => {
                results.unreadable.push(UnreadableFile {
                    path: path.display().to_string(),
                    reason,
                });
                return Ok(());
            }
        };

        results.files_scanned += 1;

        let is_env = path.to_string_lossy().contains(".env");

        // ── `.env`: the real parser, not a second implementation ─────────────
        //
        // ⚠️ This used to split every line on `=` itself. A continuation line of
        // a multiline value has no `=`, so it was skipped entirely — and a PEM
        // private key, the most common multiline value there is, scanned clean
        // with exit 0. `scan.mdx` lists "private keys accidentally pasted as
        // values" as something this catches. D21.
        //
        // `core::parser` already handles multiline, quoting and `export`, and
        // eleven other modules use it. Three settings matter here:
        //
        // * `allow_expansion: false` — a scanner wants the literal text. With
        //   expansion on, `$OTHER` would be substituted, which can both hide a
        //   secret written literally and invent one that is not in the file.
        // * `allow_inline_comments: true` — `A=secret # note` yields `secret`,
        //   which is the value that would reach a program.
        // * `strict: false` — a lowercase key is still worth scanning.
        if is_env {
            if let Some(()) = self.scan_env_parsed(path, &content, results) {
                return Ok(());
            }
            // Fell through: the file does not parse. Carry on into the raw loop
            // below, which is what this command did for every `.env` before D21.
            //
            // ⚠️ Deliberately a fallback rather than an error. A malformed `.env`
            // is exactly where a half-pasted secret lives, and the baseline
            // showed the old path *does* find one in a file with an unterminated
            // quote. Refusing to scan it would trade a silent miss for a louder
            // one.
        }

        for (line_num, line) in content.lines().enumerate() {
            let line_num = line_num + 1;

            // Skip comments and empty lines
            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }

            if is_env {
                // Reached only when the file failed to parse — see above.
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().trim_start_matches("export").trim();
                    let value = value.trim();

                    let location = format!("{}:{} ({})", path.display(), line_num, key);
                    if let Some(detection) =
                        Self::best(self.registry.scan_kv(key, value, &location))
                    {
                        self.add_finding(results, path, line_num, Some(key.to_string()), detection);
                    }
                }
            } else {
                // Scan tokens for general text files.
                let location = format!("{}:{}", path.display(), line_num);
                let mut detections: Vec<super::detector::Detection> = Vec::new();

                for token in Self::extract_tokens(line) {
                    detections.extend(self.registry.scan_token(&token, &location));
                }

                // ...and the whole line, for what a token cannot carry.
                //
                // ⚠️ A line is not one value. Two unrelated credentials on one
                // line are two findings, whereas two detectors describing one
                // value are one — so these are reconciled by *value* below, not
                // collapsed into a single answer per line.
                //
                // `extract_tokens` also drops anything 20 characters or shorter,
                // so a format shorter than that is reachable only from here. An
                // AWS access key is `AKIA` plus sixteen characters — exactly 20
                // — and so arrives by this path alone.
                detections.extend(self.registry.scan_line(line, &location));

                for detection in Self::one_finding_per_value(detections) {
                    self.add_finding(results, path, line_num, None, detection);
                }
            }
        }

        Ok(())
    }

    /// One value, one finding.
    ///
    /// Every registered detector sees each key/value pair, and more than one can
    /// match — `STRIPE_SECRET_KEY` fires both the pattern matcher and the
    /// sensitive-key heuristic. Recording both inflated `secrets_found` (one
    /// Stripe key reported as two secrets), printed the same line twice, and
    /// emitted two GitHub annotations for it.
    ///
    /// Ranking, in order:
    ///
    /// 1. **Carries a remediation URL.** A finding that can say *where to revoke
    ///    this* is more useful than one that cannot, and only the named provider
    ///    patterns can. Confidence alone gets this wrong: the sensitive-key
    ///    heuristic scores `High` on any long value, so it beat the real
    ///    `AWS Secret Access Key` match — `Medium` on purpose, since that pattern
    ///    is just "40 base64-ish characters" — and a genuine AWS key was reported
    ///    as "Sensitive config key" with no link to IAM.
    /// 2. **Higher confidence.** So `MY_PASSWORD=<long value>`, which matches only
    ///    the key-aware heuristic and the shapeless high-entropy fallback, keeps
    ///    the key-aware answer.
    /// 3. **Earliest detector**, which is `RuleDetector` when the
    ///    project declared patterns and `PatternDetector` otherwise. A rule the
    ///    project wrote itself outranks a built-in heuristic that reached the
    ///    same confidence, because it carries a name someone chose.
    ///
    /// The winner keeps its label and URL but takes the **highest confidence any
    /// detector reported**. The two are answering different questions: the
    /// pattern's `Medium` on an AWS secret key expresses doubt about *which
    /// provider* a shapeless 40-character blob belongs to, not doubt about
    /// whether it is a secret — and `AWS_SECRET_ACCESS_KEY` as a key name settles
    /// the second question. Two independent detectors agreeing is stronger
    /// evidence than either alone.
    ///
    /// Without this, deduplication would have quietly narrowed `--severity high`:
    /// a real AWS secret key would drop to `Medium` and slip under a gate that
    /// used to catch it through the heuristic's separate `High` finding.
    fn best(detections: Vec<super::detector::Detection>) -> Option<super::detector::Detection> {
        fn rank(d: &super::detector::Detection) -> (bool, super::models::Confidence) {
            (d.action_url.is_some(), d.confidence)
        }

        let strongest = detections.iter().map(|d| d.confidence).max()?;
        let mut winner = detections.into_iter().reduce(|best, next| {
            if rank(&next) > rank(&best) {
                next
            } else {
                best
            }
        })?;
        winner.confidence = strongest;
        Some(winner)
    }

    /// Reconcile a line's detections by the value each one is about.
    ///
    /// [`best`](Self::best) settles *one* set of detections about *one* value.
    /// That is all a `.env` line needs, because `scan_kv` hands every detector
    /// the same value and there is one answer to pick. A line of source is not
    /// like that: it is scanned twice, by two paths that see different strings,
    /// and neither could see the other's result.
    ///
    /// So one Stripe key in a `.py` file was reported twice:
    ///
    /// ```text
    ///   ·  High-entropy string (possible secret)       low
    ///   ✗  Stripe Secret Key (LIVE)                    high
    /// ```
    ///
    /// The entropy heuristic reaches a non-`.env` file only through
    /// `scan_token`, and the provider patterns reach it only through
    /// `scan_line` — `RuleDetector::scan_token` returns `None` on purpose, so
    /// that it does not report its own match twice. Each path was internally
    /// consistent; nothing reconciled them with each other. One key counted as
    /// two secrets, and as one `high` **and** one `low` in the severity totals
    /// that `--severity` gates and CI reads.
    ///
    /// # Why containment rather than equality
    ///
    /// The two paths do not agree on where the value ends. `extract_tokens`
    /// splits on whitespace, `=`, `:` and quotes — not on `-`, `{`, `)` or `,`
    /// — so the token is often the credential plus whatever surrounds it, while
    /// the pattern matched only the credential:
    ///
    /// ```text
    /// c = "prefix-sk_live_51Habc…1234-suffix"
    ///     token   prefix-sk_live_51Habc…1234-suffix
    ///     pattern         sk_live_51Habc…1234
    /// ```
    ///
    /// Equality collapses the quoted case and leaves that one duplicated, which
    /// is the shape real code is full of. Two detections about the same line
    /// where one value contains the other are one finding, and `best` then keeps
    /// the specific answer — the named provider with its revocation URL, over
    /// "high-entropy string".
    ///
    /// ⚠️ **An empty `matched_value` is compared by equality only.** Every string
    /// contains `""`, so a detection that recorded no value would otherwise
    /// swallow every other finding on its line.
    ///
    /// Two genuinely different credentials on one line stay two findings:
    /// neither value contains the other.
    ///
    /// Order is the order values were first seen, so output stays stable —
    /// grouping through a `HashMap` would reorder findings between runs of the
    /// same scan.
    fn one_finding_per_value(
        detections: Vec<super::detector::Detection>,
    ) -> Vec<super::detector::Detection> {
        /// Do these two detections describe the same value?
        fn same_value(a: &str, b: &str) -> bool {
            if a.is_empty() || b.is_empty() {
                return a == b;
            }
            a.contains(b) || b.contains(a)
        }

        let mut groups: Vec<Vec<super::detector::Detection>> = Vec::new();
        for detection in detections {
            match groups
                .iter_mut()
                .find(|g| same_value(&g[0].matched_value, &detection.matched_value))
            {
                Some(group) => group.push(detection),
                None => groups.push(vec![detection]),
            }
        }

        groups.into_iter().filter_map(Self::best).collect()
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
    ) {
        // Skip placeholders if configured
        if self.ignore_placeholders
            && crate::utils::patterns::is_placeholder(&detection.matched_value)
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
            .and_then(crate::utils::patterns::public_prefix);

        let mut detection = detection;
        if public.is_some() {
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
        if detection.confidence < self.min_confidence {
            return;
        }

        let judged_value = detection.judged_value;
        let finding = Finding::new(
            detection.pattern,
            detection.confidence,
            truncate_value(&detection.matched_value),
            if let Some(ref var) = variable {
                format!("{}:{} ({})", path.display(), line, var)
            } else {
                format!("{}:{}", path.display(), line)
            },
            variable,
            detection.action_url,
        )
        .judged_value(judged_value)
        .public_prefix(public.map(|(prefix, _)| prefix.to_string()));

        results.add_finding(finding);
    }

    /// Extract tokens from a line for scanning.
    ///
    /// Splits on common separators and filters by minimum length.
    ///
    /// # Arguments
    ///
    /// * `line` - Line of text to extract tokens from
    ///
    /// # Returns
    ///
    /// Vector of token strings (min 20 chars by default)
    fn extract_tokens(line: &str) -> Vec<String> {
        line.split(|c: char| c.is_whitespace() || c == '=' || c == ':' || c == '"' || c == '\'')
            .filter(|t| t.len() > 20)
            .map(|s| s.to_string())
            .collect()
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

// ─────────────────────────────────────────────────────────────
// Decoding
// ─────────────────────────────────────────────────────────────

/// The outcome of trying to read a file as text.
enum Decoded {
    Text(String),
    Unreadable { reason: String },
}

/// Read a file as text, decoding UTF-16 rather than giving up on it.
///
/// ⚠️ **There are two ways a UTF-16 `.env` defeated a UTF-8 read, and only one
/// of them was an error.** Fixing the error alone leaves the other.
///
/// * **With a byte-order mark**, `read_to_string` fails: `0xFF` is not valid
///   UTF-8. That surfaced as `Err`, and the `Err` arm returned `Ok(())`.
/// * **Without one, `read_to_string` succeeds** — UTF-16LE ASCII is *valid
///   UTF-8*, because every byte is either an ASCII character or `0x00`, and
///   `0x00` is a legal UTF-8 code unit. So the scanner received
///   `"A\0W\0S\0_\0S\0E\0C\0R\0E\0T\0"` and no pattern matched a single
///   key or value. It genuinely scanned the file, and genuinely found nothing.
///
/// So a NUL byte is the signal, not a decoding error: no UTF-8 text file
/// contains one, and a UTF-16 one is half NULs by construction.
fn decode(path: &Path) -> Decoded {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return Decoded::Unreadable {
                // ⚠️ `e` and not the path: the caller records the path, and an
                // io::Error's Display does not include it.
                reason: format!("could not be read: {e}"),
            };
        }
    };

    // An explicit byte-order mark settles it.
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return from_utf16(rest, u16::from_le_bytes, "UTF-16LE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return from_utf16(rest, u16::from_be_bytes, "UTF-16BE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        // A UTF-8 BOM. Not an encoding problem, but left in the string it
        // becomes part of the first key's name.
        return match String::from_utf8(rest.to_vec()) {
            Ok(text) => Decoded::Text(text),
            Err(e) => Decoded::Unreadable {
                reason: format!("has a UTF-8 byte-order mark but is not valid UTF-8: {e}"),
            },
        };
    }

    // No BOM. A NUL means this is not UTF-8 text whatever `from_utf8` says.
    if bytes.contains(&0) {
        // UTF-16 without a byte-order mark. Try both parities and judge the
        // **result**, not the input.
        //
        // ⚠️ The first version of this counted NUL bytes by position — "at
        // least half the even-numbered bytes are zero, so it is UTF-16BE". A
        // six-byte binary blob, `00 FF 13 37 00 01`, satisfies that by
        // accident and was decoded into Ethiopic and Latin-Extended
        // characters, which the caller then scanned as though it were the
        // file's text. Its own test caught it.
        //
        // A ratio of input bytes cannot distinguish those cases. What can is
        // whether the decode *produced text*: a `.env` is ASCII keys, `=` and
        // newlines, with at most a few non-ASCII characters in values, and
        // never a C0 control character other than tab, newline or return.
        if bytes.len() % 2 == 0 && !bytes.is_empty() {
            for (unit, label) in [
                (
                    u16::from_le_bytes as fn([u8; 2]) -> u16,
                    "UTF-16LE (no BOM)",
                ),
                (
                    u16::from_be_bytes as fn([u8; 2]) -> u16,
                    "UTF-16BE (no BOM)",
                ),
            ] {
                if let Decoded::Text(text) = from_utf16(&bytes, unit, label) {
                    if looks_like_text(&text) {
                        return Decoded::Text(text);
                    }
                }
            }
        }
        return Decoded::Unreadable {
            reason: "contains NUL bytes and does not decode as UTF-16, so it is not text \
                     this scanner can read"
                .to_string(),
        };
    }

    match String::from_utf8(bytes) {
        Ok(text) => Decoded::Text(text),
        Err(e) => Decoded::Unreadable {
            reason: format!("is not valid UTF-8: {e}"),
        },
    }
}

/// Did a candidate decode actually produce text?
///
/// Two conditions, both cheap and both decisive on the cases that matter:
///
/// * **No C0 control characters** beyond tab, newline and carriage return.
///   Binary interpreted as UTF-16 lands on these almost immediately.
/// * **Mostly ASCII.** A `.env` is ASCII keys, `=`, and newlines; a non-ASCII
///   value is possible but cannot dominate the file. Binary decoded as UTF-16
///   produces characters spread across the whole BMP, so it fails this by a
///   wide margin rather than a narrow one.
fn looks_like_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut ascii = 0usize;
    let mut total = 0usize;
    for c in text.chars() {
        if c.is_control() && !matches!(c, '\t' | '\n' | '\r') {
            return false;
        }
        if c.is_ascii() {
            ascii += 1;
        }
        total += 1;
    }
    // Half, not all: a password with an accent in it is still a .env file.
    ascii * 2 >= total
}

/// Decode UTF-16 code units into a `String`.
fn from_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16, label: &str) -> Decoded {
    if bytes.len() % 2 != 0 {
        return Decoded::Unreadable {
            reason: format!("looks like {label} but has an odd number of bytes"),
        };
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    match String::from_utf16(&units) {
        Ok(text) => Decoded::Text(text),
        Err(e) => Decoded::Unreadable {
            reason: format!("looks like {label} but does not decode: {e}"),
        },
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
    fn test_extract_tokens() {
        let line = "API_KEY=sk_live_1234567890abcdefghijklmnop short";
        let tokens = ScanRunner::extract_tokens(line);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].len() > 20);
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
