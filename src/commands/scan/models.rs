//! Data models for secret scanning results.
//!
//! This module defines the core data structures used throughout the scan command:
//! - [`Finding`]: Represents a single detected secret with metadata
//! - [`ScanResults`]: Aggregates all findings from a scan operation
//! - [`Confidence`]: Enum for confidence levels (High/Medium/Low)
//!
//! # Example
//!
//! ```rust
//! # use evnx::commands::scan::{Finding, ScanResults, Confidence};
//! # use evnx::core::secretscan::Severity;
//! # use evnx::commands::scan::models::Status;
//!
//! let finding = Finding {
//!     // The rule's stable id, and how bad a match is if real (C2).
//!     rule_id: "aws-access-key".to_string(),
//!     // New, or accepted by a marker or the baseline (C5).
//!     status: Status::New,
//!     fingerprint: String::new(),
//!     reason: None,
//!     reason_code: None,
//!     encoded: None,
//!     context: Vec::new(),
//!     confidence_before_context: None,
//!     pattern: "AWS Access Key".to_string(),
//!     confidence: Confidence::High,
//!     severity: Severity::Critical,
//!     value_preview: "AKIA...XYZ12".to_string(),
//!     location: ".env:15 (AWS_ACCESS_KEY_ID)".to_string(),
//!     file: ".env".to_string(),
//!     key_path: None,
//!     variable: Some("AWS_ACCESS_KEY_ID".to_string()),
//!     action_url: Some("https://console.aws.amazon.com/iam".to_string()),
//!     // An `AKIA` prefix is a value match, so the report may say so.
//!     judged_value: true,
//!     // `Some("VITE_")` when the name carries a build-time public prefix, which
//!     // changes the remedy: the variable has to be renamed, not just rotated.
//!     public_prefix: None,
//!     // What git says about the file, inside a repository (I-285).
//!     git: None,
//!     repo_path: None,
//!     in_commit: None,
//!     note: None,
//!     line: 15,
//!     commit: None,
//!     blob: None,
//!     commit_time: None,
//! };
//!
//! let mut results = ScanResults::new(100);
//! results.add_finding(finding);
//! ```

use serde::{Deserialize, Serialize};

/// How sure a detection is. Defined with the rule engine in
/// [`crate::core::secretscan`]; re-exported here, where it has always been.
pub use crate::core::secretscan::{Confidence, Severity};

/// A single detected secret with contextual metadata.
///
/// # Fields
///
/// * `pattern` - Name of the detected secret pattern (e.g., "AWS Access Key")
/// * `confidence` - Confidence level of the detection
/// * `value_preview` - Truncated secret value for safe display (never shows full secret)
/// * `location` - File path and line number where secret was found
/// * `variable` - Variable name if detected in a key-value context (e.g., .env files)
/// * `action_url` - Optional URL for remediation (e.g., key rotation page)
///
/// # Security Note
///
/// The `value_preview` field should never contain the full secret value.
/// Use [`preview_value()`](super::runner::preview_value) to safely truncate
/// before creating a Finding.
///
/// # Example
///
/// ```rust
/// # use evnx::commands::scan::{Finding, Confidence};
/// let finding = Finding::new(
///     "GitHub Token",
///     Confidence::High,
///     "ghp_...abc12",
///     ".env:10 (GITHUB_TOKEN)",
///     Some("GITHUB_TOKEN".to_string()),
///     Some("https://github.com/settings/tokens".to_string()),
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// The rule's stable id (C2): `aws-access-key`, `sensitive-config-key`.
    pub rule_id: String,
    /// `new`, or `known` (in the baseline) or `allowed` (an `evnx:allow`
    /// marker) (C5). Only `new` findings are in `findings` and count.
    #[serde(default)]
    pub status: Status,
    /// The finding's identity across runs (C5, S9). Never a value.
    #[serde(default)]
    pub fingerprint: String,
    /// An `allowed` finding's reason, from its marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The reason code of an `allowed` or `known` finding (SCAN-REASONS):
    /// `used-in-tests`, `false-positive`, `fix-later`, from its marker's
    /// `reason=` or its baseline entry. Absent when none was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// `base64`, `hex` or `url` when the finding is in text decoded from the
    /// value at its location (RM-71). Absent for a value as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoded: Option<String>,
    /// The hints that moved `confidence` (SCAN-CONTEXT): `usage` and/or
    /// `test-path`. Empty when none applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    /// The confidence before the hints, when they changed it. The exit code,
    /// the `--confidence` floor and the summary counts use this one (D9).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_confidence"
    )]
    pub confidence_before_context: Option<Confidence>,
    pub pattern: String,
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: Confidence,
    /// How bad it is if real (S7, C2). `--severity` filters on this.
    pub severity: Severity,
    pub value_preview: String,
    pub location: String,
    /// The file, as `location` names it (`./config.yml`, `<stdin>`; under
    /// `--history` the path in the commit).
    #[serde(default)]
    pub file: String,
    /// The line the finding is on, 1-based; `null` when it is not known.
    /// Also used for sorting (I-062).
    #[serde(
        default,
        serialize_with = "serialize_line",
        deserialize_with = "deserialize_line"
    )]
    pub line: usize,
    /// In a structured file (YAML, JSON, TOML, a notebook), the key path of
    /// the value: `app.db.password`, `rules[76].allow[0]`. Absent elsewhere,
    /// where `variable` is a variable's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    pub variable: Option<String>,
    pub action_url: Option<String>,
    /// Did a detector read the **value**, or only the variable's name?
    ///
    /// Serialized as `matched_by: "value" | "name"`. Defaults to `true`, so the
    /// many test constructors keep meaning "a real value matched".
    ///
    /// ⚠️ This was `#[serde(skip)]` when F18 first landed, on the reasoning that
    /// the distinction only affected one line of terminal output and that
    /// widening a contract CI scripts parse was not worth it. That was wrong in
    /// the one direction that matters: CI is the audience most likely to
    /// automate on "is this a real leak?", and it was the audience that could
    /// not tell. Adding a field is additive — no existing consumer breaks.
    #[serde(rename = "matched_by", serialize_with = "serialize_matched_by")]
    #[serde(default = "judged_value_default")]
    pub judged_value: bool,

    /// The build-time prefix that publishes this variable, if its name carries
    /// one — `"NEXT_PUBLIC_"`, `"VITE_"` and friends.
    ///
    /// Serialized so a CI script can act on the distinction that matters most
    /// here: a secret behind one of these is **already served to every visitor**,
    /// so rotating it is necessary but not sufficient, and scrubbing git history
    /// achieves nothing. `None` for every ordinary variable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub public_prefix: Option<String>,

    /// What git says about the file this was found in (I-285): `tracked`,
    /// `unignored` (the next `git add .` takes it) or `ignored`. Absent
    /// outside a repository, where nothing is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub git: Option<GitState>,

    /// The file's path from the repository root, for `git filter-repo
    /// --path` (I-286). Display only.
    #[serde(skip)]
    pub repo_path: Option<String>,

    /// Whether a commit holds the file (any branch or tag): `Some(false)` for
    /// a file that is staged and in no commit, which gets "unstage it", not a
    /// history rewrite. `None` when nothing is known. Display only.
    #[serde(skip)]
    pub in_commit: Option<bool>,

    /// What to say about a value match in place of the generic "matches a
    /// live key format" line — `a password in a postgres:// URL` (I-052).
    /// Display only.
    #[serde(skip)]
    pub note: Option<String>,

    /// `--history` (RM-69): the oldest commit in the range that has the blob
    /// the finding is in. Absent for every other scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// `--history`: the blob's object id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    /// `--history`: that commit's time, seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_time: Option<i64>,
}

/// Whether a finding counts (C5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    New,
    /// In the baseline: accepted earlier.
    Known,
    /// Silenced by an `evnx:allow <rule-id>` marker on its line or the line above.
    Allowed,
}

/// [`crate::core::gitignore::Exposure`], as a finding reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitState {
    /// In the index: the secret is in the repository.
    Tracked,
    /// Neither ignored nor tracked: the next `git add .` takes it.
    Unignored,
    /// Ignored and not tracked: a commit leaves it out.
    Ignored,
    /// Read from the index by `--staged` (C9): the next commit records it.
    Staged,
    /// Read from a commit's blob by `--history` (RM-69): it is in the
    /// repository's history.
    History,
}

impl From<crate::core::gitignore::Exposure> for GitState {
    fn from(e: crate::core::gitignore::Exposure) -> Self {
        use crate::core::gitignore::Exposure;
        match e {
            Exposure::Tracked => GitState::Tracked,
            Exposure::Unignored => GitState::Unignored,
            Exposure::Protected => GitState::Ignored,
        }
    }
}

/// `#[serde(skip)]` would otherwise deserialize to `false` and make every
/// round-tripped finding claim its value was never checked.
fn judged_value_default() -> bool {
    true
}

/// `true`/`false` says nothing to someone reading the JSON; `value`/`name` says
/// exactly what the scanner looked at.
fn serialize_matched_by<S>(judged: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(if *judged { "value" } else { "name" })
}

/// `line`: a number, or `null` for 0 (not known).
fn serialize_line<S>(line: &usize, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match line {
        0 => serializer.serialize_none(),
        n => serializer.serialize_u64(*n as u64),
    }
}

fn deserialize_line<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<usize>::deserialize(deserializer)?.unwrap_or(0))
}

/// Custom serializer for Confidence to output as string in JSON
fn serialize_confidence<S>(confidence: &Confidence, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&confidence.to_string())
}

fn serialize_opt_confidence<S>(c: &Option<Confidence>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match c {
        Some(c) => serializer.serialize_str(&c.to_string()),
        None => serializer.serialize_none(),
    }
}

impl Finding {
    /// The confidence the exit code and the counts use: the one before any
    /// context hint (D9).
    #[must_use]
    pub fn decided_confidence(&self) -> Confidence {
        self.confidence_before_context.unwrap_or(self.confidence)
    }

    /// Record that this finding came from the variable's **name**, with the
    /// value unexamined.
    ///
    /// Builder-style rather than a seventh parameter to [`Finding::new`]:
    /// exactly one caller in the whole crate knows the answer, and the other
    /// thirteen construction sites are tests that mean the default.
    #[must_use]
    pub fn judged_value(mut self, judged: bool) -> Self {
        self.judged_value = judged;
        self
    }

    /// Record which rule matched and how severe it is. The runner always
    /// does; [`Finding::new`]'s defaults (a slug of the name, medium) are for
    /// the test constructors.
    #[must_use]
    pub fn rule(mut self, id: impl Into<String>, severity: Severity) -> Self {
        self.rule_id = id.into();
        self.severity = severity;
        self
    }

    /// Record the build-time prefix that publishes this variable.
    ///
    /// Builder-style for the same reason as [`Finding::judged_value`]: one caller
    /// knows, and every other construction site means `None`.
    #[must_use]
    pub fn public_prefix(mut self, prefix: Option<String>) -> Self {
        self.public_prefix = prefix;
        self
    }

    /// Create a new Finding with all required fields.
    ///
    /// # Arguments
    ///
    /// * `pattern` - Name of the detected pattern
    /// * `confidence` - Confidence level
    /// * `value_preview` - Truncated secret value (max 20 chars visible)
    /// * `location` - File and line information
    /// * `variable` - Optional variable name
    /// * `action_url` - Optional remediation URL
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{Finding, Confidence};
    /// let finding = Finding::new(
    ///     "GitHub Token",
    ///     Confidence::High,
    ///     "ghp_...abc12",
    ///     ".env:10 (GITHUB_TOKEN)",
    ///     Some("GITHUB_TOKEN".to_string()),
    ///     Some("https://github.com/settings/tokens".to_string()),
    /// );
    /// ```
    pub fn new(
        pattern: impl Into<String>,
        confidence: Confidence,
        value_preview: impl Into<String>,
        location: impl Into<String>,
        variable: Option<String>,
        action_url: Option<String>,
    ) -> Self {
        let pattern = pattern.into();
        let location = location.into();
        Self {
            file: super::output::file_of(&location).to_string(),
            status: Status::New,
            fingerprint: String::new(),
            reason: None,
            reason_code: None,
            encoded: None,
            context: Vec::new(),
            confidence_before_context: None,
            rule_id: crate::core::secretscan::patternset::slug(
                pattern.split(':').next().unwrap_or(&pattern),
            ),
            pattern,
            confidence,
            severity: Severity::Medium,
            value_preview: value_preview.into(),
            location,
            variable,
            action_url,
            judged_value: true,
            public_prefix: None,
            git: None,
            repo_path: None,
            in_commit: None,
            note: None,
            line: 0,
            key_path: None,
            commit: None,
            blob: None,
            commit_time: None,
        }
    }
}

/// Aggregated results from a complete scan operation.
///
/// # Fields
///
/// * `files_scanned` - Total number of files processed
/// * `secrets_found` - Total number of findings
/// * `findings` - Vector of all detected secrets
/// * `high_confidence` - Count of high-confidence findings
/// * `medium_confidence` - Count of medium-confidence findings
/// * `low_confidence` - Count of low-confidence findings
///
/// # Example
///
/// ```rust
/// # use evnx::commands::scan::ScanResults;
/// let mut results = ScanResults::new(50);
/// // ... add findings ...
/// println!("Found {} secrets in {} files", results.secrets_found, results.files_scanned);
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct ScanResults {
    /// Files actually read and searched.
    ///
    /// ⚠️ This used to be set from `files.len()` before any file was opened, so
    /// it reported "1 file scanned" about a file the scanner could not decode
    /// and had silently skipped. It is now incremented per file that was really
    /// read; anything that was not lands in `unreadable`.
    pub files_scanned: usize,
    /// Each file read, and how it was read (RM-47). Name-based checks, such as
    /// `DB_PASSWORD=…` flagged by its name, run only on files read as
    /// `dotenv`; a `text` file gets the value patterns alone.
    #[serde(default)]
    pub files: Vec<ScannedFile>,
    /// Files this scan selected and then could not read.
    ///
    /// ⛔ **Not an empty list to ignore.** A secret scanner that cannot read a
    /// file has not cleared it, and saying nothing is the failure mode that made
    /// task 23 and this defect the same bug twice.
    pub unreadable: Vec<UnreadableFile>,
    /// Files a folder walk found and did not read: not text, or larger than
    /// `filters::MAX_WALK_BYTES` (I-047). Reported, never silent; unlike
    /// `unreadable` they do not fail the scan — they are not files a scanner
    /// is expected to read.
    pub skipped: Vec<UnreadableFile>,
    /// Walked files git ignores and does not track, and how many findings
    /// each held (I-285). Not counted anywhere else: those findings are not in
    /// `findings`, the totals or the exit code. Empty with `--include-ignored`,
    /// outside a repository, and for files named on the command line.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub ignored_by_git: Vec<IgnoredFile>,
    /// Of the files read, the ones held back that way, with findings or not:
    /// the pretty output's `scanning …` line names them apart from the files
    /// that are counted.
    #[serde(skip)]
    pub summarised: Vec<std::path::PathBuf>,
    pub secrets_found: usize,
    pub findings: Vec<Finding>,
    /// Findings silenced by an `evnx:allow` marker (C5). Not counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<Finding>,
    /// Findings in the baseline (C5). Not counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known: Vec<Finding>,
    pub high_confidence: usize,
    pub medium_confidence: usize,
    pub low_confidence: usize,
    /// Lines for stderr in the order they arose: `(is_warning, text)`, else a
    /// `-v` line. Held, not printed, while files are scanned in parallel
    /// (RM-74), so stderr reads the same whatever the thread count.
    #[serde(skip)]
    pub log: Vec<(bool, String)>,
    /// What the run was configured to leave out, so a clean result can be read
    /// for what it covers. `None` until the runner sets it.
    #[serde(default)]
    pub effective_config: Option<EffectiveConfig>,
    /// Dependency folders a walk left out (`vendor (go.mod)`, `.venv`), so a
    /// clean result says what it did not cover. `--include-dir` walks one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_dirs: Vec<String>,
}

/// The settings that decide what a scan can report: rules turned off, the
/// lowest severity and confidence shown, and the baseline in force. Printed in
/// pretty output and recorded in JSON and SARIF, so "no secrets detected" is
/// never read without knowing what was not looked for.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveConfig {
    /// `[scan] disable`, as written.
    pub disabled_rules: Vec<String>,
    /// `[scan] builtins = false`: no built-in rule runs.
    #[serde(default)]
    pub builtins_disabled: bool,
    /// Lowest severity reported: `low` reports everything.
    pub severity_floor: String,
    /// Lowest confidence reported: `low` reports everything.
    pub confidence_floor: String,
    /// The baseline in force, when it holds entries.
    pub baseline: Option<EffectiveBaseline>,
    /// Configuration files read; empty with `--no-config`.
    pub config_files: Vec<String>,
    /// `[scan] exclude` patterns from the repository's configuration.
    #[serde(default)]
    pub config_excludes: Vec<String>,
    /// `[scan] ignore_placeholders = true`.
    #[serde(default)]
    pub ignore_placeholders: bool,
    /// `[scan] extensions`: folder walks read only these.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    /// `[scan] max_file_size` in bytes, when below the 1 MiB default.
    #[serde(default)]
    pub max_file_size: Option<u64>,
    /// Variables marked `secret = false` in `[vars]`.
    #[serde(default)]
    pub secret_false: Vec<String>,
    /// A hash of the rule set in force (SCAN-RESCAN): the built-in version,
    /// project rules, packs, patterns and the disabled list. Never a rule.
    #[serde(default)]
    pub rules_digest: String,
}

/// The repository settings, beyond rules and floors, that hide findings:
/// what a run records in [`EffectiveConfig`].
#[derive(Debug, Clone, Default)]
pub struct RepoSettings {
    pub exclude: Vec<String>,
    pub ignore_placeholders: bool,
    pub extensions: Option<Vec<String>>,
    pub max_file_size: Option<u64>,
    pub secret_false: Vec<String>,
}

/// A baseline that sets findings aside.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveBaseline {
    pub path: String,
    /// Findings it accepts.
    pub entries: usize,
}

impl EffectiveConfig {
    /// Does anything here narrow what the scan reports?
    pub fn narrows(&self) -> bool {
        !self.disabled_rules.is_empty()
            || self.builtins_disabled
            || self.severity_floor != "low"
            || self.confidence_floor != "low"
            || self.baseline.is_some()
            || !self.config_excludes.is_empty()
            || self.ignore_placeholders
            || self.extensions.is_some()
            || self.max_file_size.is_some()
            || !self.secret_false.is_empty()
    }
}

/// Aggregate counts, as a nested object.
///
/// `evnx validate` and `evnx doctor` both close their JSON with a `summary`, and
/// `scan` was the one machine-readable command without one — so a pipeline had to
/// special-case it. Worse, `evnx.dev` documented `scan` as if it *did* have one:
/// `jq '.summary.errors + .summary.warnings'` returned `null`, and
/// `jq -e '.summary.errors == 0'` failed a build that was actually clean.
///
/// ⚠️ The fields are **confidence** levels, not severities. A high-confidence
/// detection means "this really looks like an AWS key", not "this is the worst
/// problem here" — the two are different questions and the docs conflated them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    /// Findings reported, after any `--severity` and `--confidence` filtering.
    pub total: usize,
    /// By **confidence**, as before C2.
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    /// By severity (C2).
    pub severity: SeverityCounts,
    pub files_scanned: usize,
    /// Files that were selected and could not be read.
    ///
    /// ⚠️ A pipeline asserting `summary.total == 0` is asserting "no secrets
    /// were found", which is not "there are no secrets" when this is non-zero.
    /// Gate on both.
    pub files_unreadable: usize,
    /// Files a folder walk passed over (binary, or over the size limit).
    pub files_skipped: usize,
    /// Findings not counted because git ignores their file (I-285). Absent
    /// when there are none, so a scan outside a repository prints the same
    /// summary as before.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ignored_by_git: usize,
    /// Findings an `evnx:allow` marker silenced (C5).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub allowed: usize,
    /// Findings in the baseline (C5).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub known: usize,
    /// Allowed and known findings by reason code (SCAN-REASONS), for the ones
    /// that have one. Absent when none does.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub by_reason: std::collections::BTreeMap<String, usize>,
}

/// Findings per severity, in the JSON `summary` (C2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A walked file git ignores and does not track, with the number of findings
/// it held (I-285).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnoredFile {
    pub path: String,
    pub findings: usize,
}

/// A file the scan read, and which detectors ran on it (RM-47).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: String,
    pub read_as: ReadAs,
    /// Why a file was not read the way its name suggests: `did not parse as
    /// yaml` for a template read as text instead (C8). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// How a file was read, which decides the detectors that ran on it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReadAs {
    /// Named like a `.env`: assignments, checked by name and by value.
    Dotenv,
    /// YAML, JSON or TOML that parsed: every string value checked by its key
    /// and its value, plus the line rules (C8).
    Structured,
    /// Anything else: tokens and lines, checked against value patterns only.
    Text,
}

impl ReadAs {
    /// What `-v` says about a file read this way.
    pub fn describe(self) -> &'static str {
        match self {
            ReadAs::Dotenv => "read as .env: names and values checked",
            ReadAs::Structured => "read as structured data: names and values checked",
            ReadAs::Text => "read as text: value patterns only, names not checked",
        }
    }

    /// [`describe`](Self::describe) for the file at `path`: a Dockerfile, shell
    /// script or Python module is read as text but also has its assignments
    /// checked by name (SCAN-NAMES). Decided by file name only; an extensionless
    /// script found by its `#!` line keeps the plain text label.
    pub fn describe_file(self, path: impl AsRef<std::path::Path>) -> &'static str {
        let named = self == ReadAs::Text
            && crate::core::secretscan::assign::Shape::of(&path.as_ref().to_string_lossy(), "")
                .is_some();
        if named {
            "read as text: value patterns, and names in assignments"
        } else {
            self.describe()
        }
    }
}

/// A file the scan selected and could not read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnreadableFile {
    pub path: String,
    pub reason: String,
}

impl ScanResults {
    /// Aggregate counts for machine-readable output.
    pub fn summary(&self) -> Summary {
        Summary {
            total: self.secrets_found,
            high: self.high_confidence,
            medium: self.medium_confidence,
            low: self.low_confidence,
            severity: self.severity_counts(),
            files_scanned: self.files_scanned,
            files_unreadable: self.unreadable.len(),
            files_skipped: self.skipped.len(),
            ignored_by_git: self.ignored_by_git.iter().map(|f| f.findings).sum(),
            allowed: self.allowed.len(),
            known: self.known.len(),
            by_reason: self.reason_counts(self.allowed.iter().chain(&self.known)),
        }
    }

    /// `findings` counted by reason code, those without one left out.
    pub fn reason_counts<'a>(
        &self,
        findings: impl Iterator<Item = &'a Finding>,
    ) -> std::collections::BTreeMap<String, usize> {
        let mut out = std::collections::BTreeMap::new();
        for code in findings.filter_map(|f| f.reason_code.as_deref()) {
            *out.entry(code.to_string()).or_insert(0) += 1;
        }
        out
    }
}

impl ScanResults {
    /// Create new ScanResults with initialized counters.
    ///
    /// # Arguments
    ///
    /// * `files_scanned` - Number of files that will be/were scanned
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::ScanResults;
    /// let results = ScanResults::new(100);
    /// assert_eq!(results.files_scanned, 100);
    /// assert_eq!(results.secrets_found, 0);
    /// ```
    pub fn new(files_scanned: usize) -> Self {
        Self {
            files_scanned,
            files: Vec::new(),
            unreadable: Vec::new(),
            skipped: Vec::new(),
            ignored_by_git: Vec::new(),
            summarised: Vec::new(),
            secrets_found: 0,
            findings: Vec::new(),
            allowed: Vec::new(),
            known: Vec::new(),
            high_confidence: 0,
            medium_confidence: 0,
            low_confidence: 0,
            log: Vec::new(),
            effective_config: None,
            skipped_dirs: Vec::new(),
        }
    }

    /// Append what another (one file's) scan produced, after what is here.
    pub fn merge(&mut self, other: ScanResults) {
        self.files_scanned += other.files_scanned;
        self.files.extend(other.files);
        self.unreadable.extend(other.unreadable);
        self.skipped.extend(other.skipped);
        self.ignored_by_git.extend(other.ignored_by_git);
        self.summarised.extend(other.summarised);
        self.secrets_found += other.secrets_found;
        self.findings.extend(other.findings);
        self.allowed.extend(other.allowed);
        self.known.extend(other.known);
        self.high_confidence += other.high_confidence;
        self.medium_confidence += other.medium_confidence;
        self.low_confidence += other.low_confidence;
        self.log.extend(other.log);
        self.skipped_dirs.extend(other.skipped_dirs);
    }

    /// Add a finding and update confidence counters.
    ///
    /// # Arguments
    ///
    /// * `finding` - The detected secret to add
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{ScanResults, Finding, Confidence};
    /// let mut results = ScanResults::new(10);
    /// let finding = Finding::new("API Key", Confidence::High, "sk_...123", "config.yml:5", None, None);
    /// results.add_finding(finding);
    /// assert_eq!(results.secrets_found, 1);
    /// assert_eq!(results.high_confidence, 1);
    /// ```
    pub fn add_finding(&mut self, finding: Finding) {
        match finding.decided_confidence() {
            Confidence::High => self.high_confidence += 1,
            Confidence::Medium => self.medium_confidence += 1,
            Confidence::Low => self.low_confidence += 1,
        }
        self.secrets_found += 1;
        self.findings.push(finding);
    }

    /// Check if any high or medium confidence secrets were found.
    ///
    /// Useful for determining if immediate action is required.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use evnx::commands::scan::{ScanResults, Finding, Confidence};
    /// let mut results = ScanResults::new(10);
    /// results.add_finding(Finding::new("Key", Confidence::High, "val", "loc", None, None));
    /// if results.has_critical_findings() {
    ///     println!("⚠️  Critical secrets detected!");
    /// }
    /// ```
    /// How many findings there are at each severity.
    pub fn severity_counts(&self) -> SeverityCounts {
        let n = |s: Severity| self.findings.iter().filter(|f| f.severity == s).count();
        SeverityCounts {
            critical: n(Severity::Critical),
            high: n(Severity::High),
            medium: n(Severity::Medium),
            low: n(Severity::Low),
        }
    }

    pub fn has_critical_findings(&self) -> bool {
        self.high_confidence > 0 || self.medium_confidence > 0
    }

    /// Move the findings from `start` on — one file's — out of the report and
    /// into [`ScanResults::ignored_by_git`] under `path` (I-285). The counts
    /// go down by exactly what was added, so the totals, the JSON summary and
    /// the exit code all describe what is left.
    pub fn hold_back(&mut self, start: usize, path: String) {
        let held: Vec<Finding> = self.findings.split_off(start);
        if held.is_empty() {
            return;
        }
        for f in &held {
            match f.confidence {
                Confidence::High => self.high_confidence -= 1,
                Confidence::Medium => self.medium_confidence -= 1,
                Confidence::Low => self.low_confidence -= 1,
            }
        }
        self.secrets_found -= held.len();
        self.ignored_by_git.push(IgnoredFile {
            path,
            findings: held.len(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_display() {
        assert_eq!(Confidence::High.to_string(), "high");
        assert_eq!(Confidence::Medium.to_string(), "medium");
        assert_eq!(Confidence::Low.to_string(), "low");
    }

    #[test]
    fn test_finding_new() {
        let finding = Finding::new(
            "AWS Key",
            Confidence::High,
            "AKIA...XYZ",
            ".env:10",
            Some("AWS_KEY".to_string()),
            None,
        );
        assert_eq!(finding.pattern, "AWS Key");
        assert_eq!(finding.confidence, Confidence::High);
        assert_eq!(finding.variable, Some("AWS_KEY".to_string()));
    }

    #[test]
    fn test_scan_results_new() {
        let results = ScanResults::new(50);
        assert_eq!(results.files_scanned, 50);
        assert_eq!(results.secrets_found, 0);
        assert!(results.findings.is_empty());
    }

    #[test]
    fn test_scan_results_add_finding() {
        let mut results = ScanResults::new(10);
        results.add_finding(Finding::new(
            "Key1",
            Confidence::High,
            "val",
            "loc:1",
            None,
            None,
        ));
        results.add_finding(Finding::new(
            "Key2",
            Confidence::Medium,
            "val",
            "loc:2",
            None,
            None,
        ));
        results.add_finding(Finding::new(
            "Key3",
            Confidence::Low,
            "val",
            "loc:3",
            None,
            None,
        ));

        assert_eq!(results.secrets_found, 3);
        assert_eq!(results.high_confidence, 1);
        assert_eq!(results.medium_confidence, 1);
        assert_eq!(results.low_confidence, 1);
    }

    #[test]
    fn test_has_critical_findings() {
        let mut results = ScanResults::new(10);
        assert!(!results.has_critical_findings());

        results.add_finding(Finding::new(
            "Key",
            Confidence::Low,
            "val",
            "loc",
            None,
            None,
        ));
        assert!(!results.has_critical_findings());

        results.add_finding(Finding::new(
            "Key",
            Confidence::High,
            "val",
            "loc",
            None,
            None,
        ));
        assert!(results.has_critical_findings());
    }
}
