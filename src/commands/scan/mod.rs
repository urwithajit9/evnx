//! Secret scanning command module.
//!
//! Scans files for accidentally committed secrets using pattern matching
//! and entropy analysis. Outputs findings with confidence levels and
//! remediation steps.
//!
//! # Architecture
//!
//! ```text
//! mod.rs          → Public API (backward compatible run() function)
//! runner.rs       → Orchestration logic
//! detector.rs     → Detection strategies (trait + implementations)
//! patternset.rs   → Custom patterns: --pattern and [[scan.patterns]]
//! filters.rs      → File collection and filtering
//! output.rs       → Output formatters (pretty/json/sarif)
//! models.rs       → Data structures (Finding, ScanResults)
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use evnx::commands::scan;
//!
//! // CLI-compatible entry point
//! scan::run(scan::ScanOptions {
//!     paths: vec!["./src".to_string()],
//!     severity: "low".to_string(),   // lowest confidence reported
//!     ..Default::default()           // pretty output, no patterns, no spec
//! }).unwrap();
//! ```
//!
//! # Adding New Detectors
//!
//! See [`detector`](self::detector) module documentation for detailed steps.
//!
//! Quick example:
//!
//! ```no_run
//! # use evnx::commands::scan::detector::{SecretDetector, Detection};
//! # use evnx::commands::scan::models::Confidence;
//! # use std::path::Path;
//! pub struct MyDetector;
//!
//! impl SecretDetector for MyDetector {
//!     fn name(&self) -> &str { "my-detector" }
//!     fn scan_kv(&self, key: &str, value: &str, _loc: &str) -> Option<Detection> {
//!         // Your logic here
//!         None
//!     }
//!     fn scan_token(&self, token: &str, _loc: &str) -> Option<Detection> {
//!         self.scan_kv("", token, "")
//!     }
//! }
//! ```

pub mod filters;
pub mod hook;
pub mod models;
pub mod output;
pub mod packs_cmd;
pub mod rules_cmd;
pub mod runner;
pub mod staged;
pub mod suppress;
pub mod walk;
// The rule engine lives in `core::secretscan` so `sync` can ask it without
// importing a command (A-09). Its old paths keep working.
pub use crate::core::secretscan::{detector, patternset};

use crate::core::verdict::Verdict;
use crate::docs;
use crate::utils::ui;
pub use detector::{Detection, DetectorRegistry, SecretDetector};
pub use filters::FileFilter;
pub use models::{Confidence, Finding, ScanResults};
pub use output::{OutputFormat, render};
pub use patternset::{PatternMatch, PatternSet};
pub use runner::{ScanRunner, preview_value};

/// Everything `evnx scan` was asked to do, resolved from flags and `.evnx.toml`.
///
/// `Default` is the bare command: the current folder, every confidence,
/// pretty output, built-ins only through `patterns` (empty here), no spec.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Files or folders to scan.
    pub paths: Vec<String>,
    /// Exclusion patterns for folder walks (`--exclude` plus `[scan] exclude`).
    pub exclude: Vec<String>,
    /// Secret formats in force: `--pattern` merged with `[[scan.patterns]]`
    /// and the built-ins. See [`patternset::merge`](self::patternset::merge).
    pub patterns: Vec<crate::core::config::PatternRule>,
    /// Skip values that look like filler.
    pub ignore_placeholders: bool,
    /// Lowest severity reported: `critical`, `high`, `medium` or `low` (C2;
    /// until then this filtered on confidence).
    pub severity: String,
    /// Lowest confidence reported: `high`, `medium` or `low` (C2).
    pub confidence: String,
    /// `--baseline <file>`: findings accepted earlier (C5). `None` reads
    /// `.evnx/baseline.json` when it exists.
    pub baseline: Option<String>,
    /// `--update-baseline`: write this run's findings to the baseline and
    /// exit 0 (C5).
    pub update_baseline: bool,
    /// `--reason <code>` with `--update-baseline`: why the entries added now
    /// are accepted (SCAN-REASONS).
    pub reason: Option<String>,
    /// `--rules <file>`: rule files for this run only (C6).
    pub rule_files: Vec<String>,
    /// The folder holding the `.evnx.toml` in force, else the current one:
    /// where `.evnx/rules` and `.evnx/baseline.json` are (C5, C6).
    pub project_root: std::path::PathBuf,
    /// `pretty`, `json`, `sarif` or `github`.
    pub format: OutputFormat,
    /// Exit 0 even with findings (never hides exit 2).
    pub exit_zero: bool,
    /// Progress lines on stderr.
    pub verbose: bool,
    /// `[vars]` from `.evnx.toml`, if any.
    pub spec: crate::core::spec::Spec,
    /// An extension list for folder walks and where it came from; `None`
    /// reads every file (I-047).
    pub extensions: Option<(Vec<String>, &'static str)>,
    /// Report findings in files git ignores and does not track (I-285).
    pub include_ignored: bool,
    /// Skip walked files larger than this many bytes; `None` is 1 MiB (I-061).
    pub max_file_size: Option<u64>,
    /// `[scan] disable`, as written: built-in rule names and
    /// [`detector::HEURISTIC_NAMES`] (I-056). The rules themselves are already
    /// out of `patterns`; this also silences the heuristics for their values.
    pub disable: Vec<String>,
    /// `--staged` (C9): scan the git index, not the files on disk.
    pub staged: bool,
    /// `--follow-symlinks`: read a symlinked file in a walk when its target is
    /// inside the scanned folder.
    pub follow_symlinks: bool,
    /// `--include-dir`: default-skipped folder names to walk.
    pub include_dirs: Vec<String>,
    /// `--no-config`: repository configuration was not read, so the default
    /// baseline is not either (a named `--baseline` still is).
    pub no_config: bool,
    /// The configuration files read, as shown to the user; recorded in the
    /// report.
    pub config_files: Vec<String>,
    /// `[scan] builtins = false`, recorded in the report.
    pub builtins_disabled: bool,
    /// Other repository settings that hide findings, recorded in the report.
    pub repo_settings: models::RepoSettings,
    /// The confidence floor was set to `low` on purpose (`--confidence low` or
    /// `[scan] confidence = "low"`): report short values under strong secret
    /// names too (WP-13, D5).
    pub short_values: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            paths: vec![".".to_string()],
            exclude: Vec::new(),
            patterns: Vec::new(),
            ignore_placeholders: false,
            severity: "low".to_string(),
            confidence: "low".to_string(),
            baseline: None,
            update_baseline: false,
            reason: None,
            rule_files: Vec::new(),
            project_root: std::path::PathBuf::from("."),
            format: OutputFormat::Pretty,
            exit_zero: false,
            verbose: false,
            spec: Default::default(),
            extensions: None,
            include_ignored: false,
            max_file_size: None,
            disable: Vec::new(),
            staged: false,
            follow_symlinks: false,
            include_dirs: Vec::new(),
            no_config: false,
            config_files: Vec::new(),
            builtins_disabled: false,
            repo_settings: Default::default(),
            short_values: false,
        }
    }
}

/// Run the scan command (CLI entry point).
///
/// # Exit codes
///
/// | code | meaning |
/// |------|---------|
/// | 0 | clean — nothing at or above `--severity` |
/// | 1 | secrets found (suppressed by `--exit-zero`) |
/// | 2 | the scan could not be completed |
///
/// ⚠️ **2 is not a louder 1.** "I found nothing" and "I could not look" are
/// different answers, and collapsing them is what let `evnx scan ./typo` report
/// success. A CI gate written as `evnx scan .` still fails on either, but one
/// written to branch on the code can now tell a finding from a broken setup.
///
/// Files that were selected and could not be read (an unreadable folder, a
/// broken followed link) are exit 1, the same class as a finding, so
/// `--exit-zero` lets them pass; they are always listed and counted in the output.
///
/// `--exit-zero` suppresses 1 only. Trouble still exits 2: the flag means "do
/// not fail my build over findings", not "never tell me the scan was impossible".
///
/// ⚠️ **A custom pattern that does not compile is trouble, not a clean scan.**
/// `--pattern '('` exits 2 and says so. A scanner that shrugged off a broken
/// rule and printed "no secrets found" would be reporting a result for a search
/// it never ran — and in CI that reads as a pass.
///
/// # Example
///
/// ```no_run
/// use evnx::commands::scan;
///
/// scan::run(scan::ScanOptions {
///     paths: vec!["./src".to_string()],
///     exclude: vec!["node_modules".to_string()],
///     ..Default::default()
/// }).unwrap();
/// ```
pub fn run(opts: ScanOptions) -> Result<Verdict, anyhow::Error> {
    // Trouble prints here and exits 2 rather than propagating to `main`, which
    // would format it as a generic error and collapse it onto 1 — the code that
    // already means "secrets found".
    let scan = || -> Result<bool, anyhow::Error> {
        // An exclusion that does not parse would otherwise be dropped and the
        // run would read files the user believes are left out.
        filters::validate_excludes(&opts.exclude)?;
        // `--no-config` starts from an empty baseline; writing it back would
        // drop every accepted finding and its reason.
        if opts.no_config && opts.update_baseline {
            anyhow::bail!(
                "--no-config cannot be combined with --update-baseline: it would overwrite \
                 the baseline from an empty one and lose its entries. Run --update-baseline \
                 without --no-config."
            );
        }
        // SCAN-REASONS: a code outside the list is kept, and said so.
        if let Some(code) = opts
            .reason
            .as_deref()
            .filter(|c| !suppress::is_known_reason(c))
        {
            crate::utils::ui::warning(format!(
                "--reason: {}",
                suppress::unknown_reason_warning(code)
            ));
        }
        let min_severity: crate::core::secretscan::Severity = opts.severity.parse()?;
        let min_confidence: Confidence = opts.confidence.parse()?;
        // C5: a named baseline must exist (unless it is about to be written);
        // the default one is optional.
        let baseline_path = match opts.baseline.as_deref() {
            Some(p) => std::path::PathBuf::from(p),
            None => opts.project_root.join(suppress::DEFAULT_BASELINE),
        };
        // `--no-config` leaves out the repository's default baseline as well:
        // it suppresses findings, which is what the flag is there to rule out.
        let baseline = if opts.no_config && opts.baseline.is_none() {
            suppress::Baseline::default()
        } else {
            suppress::Baseline::load(
                &baseline_path,
                opts.baseline.is_some() && !opts.update_baseline,
            )?
        };
        let effective = models::EffectiveConfig {
            disabled_rules: opts.disable.clone(),
            builtins_disabled: opts.builtins_disabled,
            severity_floor: min_severity.to_string(),
            confidence_floor: min_confidence.to_string(),
            baseline: (!baseline.findings.is_empty()).then(|| models::EffectiveBaseline {
                path: ui::short_path(&baseline_path),
                entries: baseline.findings.len(),
            }),
            config_files: opts.config_files.clone(),
            config_excludes: opts.repo_settings.exclude.clone(),
            ignore_placeholders: opts.repo_settings.ignore_placeholders,
            extensions: opts.repo_settings.extensions.clone(),
            max_file_size: opts.repo_settings.max_file_size,
            secret_false: opts.repo_settings.secret_false.clone(),
        };
        let output_format: OutputFormat = opts.format;
        // Compiled before anything is printed, so a broken expression cannot be
        // mistaken for a finished scan.
        // C6: the project's, the user's and this run's rule files, after the
        // core. Personal rules are always announced (S3), on stderr so a
        // machine format's stdout is untouched.
        let extra: Vec<std::path::PathBuf> = opts
            .rule_files
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        let loaded = crate::core::secretscan::rules::load(
            &opts.project_root,
            crate::core::secretscan::rules::personal_dir().as_deref(),
            &extra,
        )?;
        for w in &loaded.warnings {
            ui::warning(w);
        }
        if loaded.personal > 0 {
            eprintln!(
                "  + {} personal rule(s) from {}",
                loaded.personal,
                crate::core::secretscan::rules::personal_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default()
            );
        }
        if opts.verbose {
            for (source, n) in &loaded.sources {
                ui::verbose_stderr(format!("{n} rule(s) from {source}"));
            }
        }
        let mut all = opts.patterns.clone();
        all.extend(loaded.rules.into_iter().map(|s| s.rule));
        let compiled = PatternSet::compile(&all)?;
        let off: Vec<String> = opts.disable.iter().map(|n| n.to_lowercase()).collect();
        let disabled = PatternSet::compile(
            &crate::core::secretscan::rules::builtin_rules()
                .into_iter()
                .filter(|r| crate::core::secretscan::patternset::is_named(r, &off))
                .collect::<Vec<_>>(),
        )?;
        let runner = ScanRunner::with_spec_and_patterns(
            &opts.exclude,
            opts.ignore_placeholders,
            opts.verbose,
            min_confidence,
            opts.spec.clone(),
            compiled,
        )
        .with_extensions(opts.extensions.clone())
        .with_include_ignored(opts.include_ignored)
        .with_max_file_size(opts.max_file_size)
        .with_min_severity(min_severity)
        .with_baseline(
            baseline,
            opts.update_baseline.then(|| baseline_path.clone()),
        )
        .with_baseline_reason(opts.reason.clone())
        .with_disabled(disabled, &opts.disable)
        .with_follow_symlinks(opts.follow_symlinks)
        .with_include_dirs(&opts.include_dirs)
        .with_short_values(opts.short_values)
        .with_effective_config(effective);
        if opts.staged {
            runner.run_staged(opts.paths.clone(), output_format)
        } else {
            runner.run(opts.paths.clone(), output_format)
        }
    };

    let secrets_found = scan().map_err(|e| {
        crate::core::verdict::footed(
            Some(format!("{e:#}")),
            "No verdict: evnx did not scan anything. This is not a clean result.",
        )
    })?;

    // Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::SCAN);

    Ok(if secrets_found && !opts.exit_zero {
        Verdict::Findings
    } else {
        Verdict::Clean
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Verify all public exports are accessible
        let _ = Confidence::High;
        let _ = Finding::new("test", Confidence::Low, "val", "loc", None, None);
        let _ = ScanResults::new(10);
        let _ = OutputFormat::Pretty;
        let _ = ScanRunner::new(&[], false, false);
    }
}
