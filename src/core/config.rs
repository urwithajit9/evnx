//! `.evnx.toml` — per-project policy.
//!
//! # What belongs here
//!
//! A setting belongs in `.evnx.toml` if it is **true of the project** and would
//! otherwise be repeated on every invocation and in every CI job. It does not
//! belong here if it expresses **what you want from this particular run**.
//!
//! So `scan.severity` (this repo's floor) is configurable and `--exit-zero` is
//! not; `validate.strict` is and `--fix` is not. That rule is what keeps the
//! schema decidable as commands are added, rather than a matter of taste each
//! time.
//!
//! # Where it is found
//!
//! Walking up from the working directory, **stopping once a directory containing
//! `.git` has been examined**, and never reaching `$HOME`.
//!
//! ⚠️ The old implementation walked all the way up and then tried `$HOME`. That
//! is wrong for the same reason `cloud::binding` documents: a setting in your
//! home directory would silently apply to *every project on the machine*, and
//! `scan.exclude` is exactly the setting you would least want to leak that way.
//!
//! Bare `evnx.toml` is no longer accepted. Two spellings needs a rule about which
//! wins, for no gain. ⚠️ Not silently, though (I-040, DEC-SP15): a bare file
//! the walk meets with no `.evnx.toml` beside it stops every command with exit 2
//! and the rename to make; beside a `.evnx.toml` it is one warning.
//!
//! # Where it is written
//!
//! [`project_file`] picks the file and [`save`] writes it, for every command
//! that records into it — `spec init`, the `[vars]` writeback `add` uses, and
//! `cloud link`. One place decides, so a contract cannot end up in a second
//! config that shadows the first (I-177).
//!
//! # Evolution
//!
//! **Additive only.** Keys are added, never repurposed or removed, so a file
//! written for a later evnx keeps working on an earlier one — which matters
//! because this file is committed and teams run mixed versions.
//!
//! **Unknown keys warn; they never fail**, for the same reason. But a typo must
//! not be silent either, so the warning names the key.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The one accepted filename.
pub const PROJECT_FILE: &str = ".evnx.toml";

/// Unknown keys captured alongside a section's known ones.
type Extra = BTreeMap<String, toml::Value>;

/// Settings that cross commands.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Defaults {
    /// Which `.env.<name>` this project means when none is given.
    pub env_name: Option<String>,
    /// The template path, when it is not the conventional `.env.example`.
    pub example: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

/// One secret format this project recognises that evnx does not ship with.
///
/// The built-in detectors know AWS, Stripe, GitHub and the rest. This is how a
/// team adds the format only it knows about — an internal token, a vendor key
/// shape — and has every clone of the repository scan for it.
///
/// ```toml
/// [[scan.patterns]]
/// name = "Acme API key"
/// regex = "ACME-[A-Z0-9]{32}"
/// confidence = "high"                        # optional, defaults to high
/// url = "https://acme.example/settings/keys" # optional: where to revoke it
/// ```
///
/// `name` and `regex` are required. The expression is compiled by the scan
/// command rather than here, so a typo in a scan-only rule does not stop
/// `evnx convert` — see `commands::scan::patternset::PatternSet::compile`,
/// which turns a bad expression into exit 2 rather than a clean scan.
///
/// ⚠️ This is deliberately different from `[vars] format`, which **is**
/// compiled at load: those expressions decide what `validate` does, so a broken
/// one there is a broken contract rather than a broken search.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct PatternRule {
    /// What to call it in output, and in the SARIF rule id.
    pub name: String,
    /// The expression. Anchored by the author if they want it anchored.
    pub regex: String,
    /// `high`, `medium` or `low`. Unset means high — you wrote the rule.
    pub confidence: Option<String>,
    /// Where to go and revoke a match, when there is such a place.
    pub url: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

impl PatternRule {
    /// A rule from a bare `--pattern` expression, named by position.
    ///
    /// `--pattern` takes an expression and nothing else. `NAME=REGEX` was
    /// considered and rejected: `=` is legal inside a character class, so
    /// `[A-Za-z0-9+/=]{40}` — an AWS-shaped pattern — would split into a
    /// nonsense name and a broken expression. Any rule that resolves that
    /// ambiguity is a rule someone has to remember.
    ///
    /// The cost is a positional name that moves if the flags are reordered,
    /// which is the nudge toward declaring the rule in `.evnx.toml` when the
    /// name needs to mean something.
    /// A rule evnx ships with.
    ///
    /// ⚠️ Built-in detectors are the **same type** as `[[scan.patterns]]`, on
    /// purpose. They used to be a hand-written `if` chain in `utils::patterns`,
    /// which made them strictly weaker than a rule a user could write: custom
    /// rules are matched against whole lines, built-ins only against tokens
    /// longer than 20 characters — so `-----BEGIN RSA PRIVATE KEY-----`, a
    /// phrase that can never be one token, was undetectable in any file.
    ///
    /// One type means one engine, one precedence rule, and a user can disable
    /// or re-rate anything evnx ships.
    pub fn builtin(name: &str, regex: &str, confidence: &str, url: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            regex: regex.to_string(),
            confidence: Some(confidence.to_string()),
            url: url.map(str::to_string),
            ..Default::default()
        }
    }

    pub fn anonymous(position: usize, regex: String) -> Self {
        Self {
            name: format!("Custom pattern {position}"),
            regex,
            ..Default::default()
        }
    }
}

/// ⚠️ Both `severity` and `exclude` can *weaken* what the scanner reports, and
/// this file is committed — so a change here affects everyone who clones the
/// repository, with nothing on the command line to show it.
///
/// That is allowed, because a team genuinely needs to exclude its fixtures.
/// What is not allowed is doing it quietly: see [`Config::security_overrides`],
/// which the CLI announces on every run that loads a config.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ScanPolicy {
    /// Lowest confidence worth reporting: `high`, `medium` or `low`.
    pub severity: Option<String>,
    /// Paths and globs this project never scans.
    pub exclude: Option<Vec<String>>,
    /// Skip values that look like filler.
    pub ignore_placeholders: Option<bool>,
    /// Built-in detectors this project does not want, by name.
    ///
    /// ⚠️ This *weakens* scanning, so it is listed by
    /// [`Config::security_overrides`] and announced on every run that loads a
    /// config. A team with fixtures that trip a provider pattern has a real need
    /// for it; doing it silently would not be acceptable.
    pub disable: Option<Vec<String>>,
    /// Set `false` to scan with **only** this project's rules.
    ///
    /// For a team that scans for its own internal token formats and finds the
    /// shipped provider patterns noisy. Also announced as a security override.
    pub builtins: Option<bool>,
    /// Secret formats this project recognises beyond the built-in ones.
    ///
    /// ⚠️ Absent from [`Config::security_overrides`] on purpose. `severity` and
    /// `exclude` are announced because they *weaken* what the scanner reports;
    /// a pattern can only add a finding, so a committed rule makes the scan
    /// stricter for everyone who clones the repository. There is nothing to
    /// warn about in that direction.
    pub patterns: Option<Vec<PatternRule>>,
    /// Read only files with these extensions in folder walks. Unset: every
    /// file (I-047, DEC-U8-2). ⚠️ Narrows the scan, so it is announced.
    pub extensions: Option<Vec<String>>,
    /// Skip walked files larger than this: `"5MB"`, `"512K"`, or a number of
    /// bytes (I-061, DEC-SC4). ⚠️ Below the 1 MiB default it narrows the scan,
    /// so it is announced.
    pub max_file_size: Option<SizeSetting>,
    #[serde(flatten)]
    extra: Extra,
}

/// A size in `.evnx.toml`: `max_file_size = "5MB"` or `= 5000000`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SizeSetting {
    Bytes(u64),
    Text(String),
}

impl SizeSetting {
    /// The size in bytes, by [`crate::core::args::parse_size`]'s rules.
    pub fn bytes(&self) -> Result<u64, String> {
        match self {
            SizeSetting::Bytes(0) => crate::core::args::parse_size("0"),
            SizeSetting::Bytes(n) => Ok(*n),
            SizeSetting::Text(t) => crate::core::args::parse_size(t),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ValidatePolicy {
    pub strict: Option<bool>,
    pub validate_formats: Option<bool>,
    /// Issue types this project suppresses.
    pub ignore: Option<Vec<String>>,
    /// `error` (the default) or `warning`: the lowest severity that fails the
    /// run. `--fail-on` overrides it.
    pub fail_on: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct SyncPolicy {
    /// `warn`, `error` or `ignore` — this project's variable-naming convention.
    pub naming_policy: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct DiffPolicy {
    /// Keys that always differ here and are not worth reporting.
    pub ignore_keys: Option<Vec<String>>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct BackupPolicy {
    /// How many previous backups to retain.
    pub keep: Option<u32>,
    #[serde(flatten)]
    extra: Extra,
}

/// Declared so that a working `[cloud] vault` binding is not reported as an
/// unknown section. It is still *read* by `cloud::binding`, which owns writing
/// this file through `toml_edit`; folding the two readers together is a separate
/// change.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct CloudPolicy {
    pub vault: Option<String>,
    /// The file push and pull use with the bound vault, relative to the
    /// folder this `.evnx.toml` is in (RM-86).
    pub file: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

/// A project's `.evnx.toml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub scan: ScanPolicy,
    #[serde(default)]
    pub validate: ValidatePolicy,
    #[serde(default)]
    pub sync: SyncPolicy,
    #[serde(default)]
    pub diff: DiffPolicy,
    #[serde(default)]
    pub backup: BackupPolicy,
    #[serde(default)]
    pub cloud: CloudPolicy,
    /// The variable contract — see [`crate::core::spec`].
    ///
    /// ⚠️ Optional, and must stay optional. Every project today has no `[vars]`
    /// section, and an absent spec means the previous behaviour exactly.
    #[serde(default)]
    pub vars: crate::core::spec::Spec,
    #[serde(flatten)]
    extra: Extra,
}

impl Config {
    /// Settings that loosen a security default, described for display.
    ///
    /// Empty when the config changes nothing security-relevant, which is the
    /// common case. The CLI prints these so that a committed config cannot
    /// silently weaken scanning for everyone who clones the repository.
    pub fn security_overrides(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(sev) = &self.scan.severity {
            out.push(format!("scan.severity={sev}"));
        }
        match self.scan.exclude.as_deref() {
            Some(patterns) if !patterns.is_empty() => {
                out.push(format!("scan.exclude={} patterns", patterns.len()));
            }
            _ => {}
        }
        if self.scan.ignore_placeholders == Some(true) {
            out.push("scan.ignore_placeholders=true".to_string());
        }
        // ⚠️ Both of these switch off detectors evnx ships, which is the most
        // direct way this file can weaken a scan. `builtins = false` turns off
        // every provider pattern at once, so it is announced even though it is
        // one line.
        match self.scan.disable.as_deref() {
            Some(names) if !names.is_empty() => {
                // "detector", not "rule": the names include heuristics such as
                // `Sensitive config key`, which are not rules (I-292).
                out.push(format!("scan.disable={} built-in detector(s)", names.len()));
            }
            _ => {}
        }
        // ⚠️ A list narrows every folder scan to those extensions (I-047): a
        // key in an unlisted file is never read.
        if let Some(list) = self.scan.extensions.as_deref() {
            out.push(format!(
                "scan.extensions={} (other files in folders are not scanned)",
                list.join(",")
            ));
        }
        if self.scan.builtins == Some(false) {
            out.push("scan.builtins=false (no built-in rules)".to_string());
        }
        // A limit below the default skips files a plain walk would read.
        if let Some(Ok(bytes)) = self.scan.max_file_size.as_ref().map(SizeSetting::bytes) {
            if bytes < 1024 * 1024 {
                out.push(format!(
                    "scan.max_file_size={} (larger files in folders are not scanned)",
                    crate::utils::string::size_label(bytes)
                ));
            }
        }
        // ⚠️ `secret = false` retracts a name-based finding, and this file is
        // committed — so one line silences a warning for everyone who clones.
        // It cannot retract a match on the value (see `SecretDetector::
        // judges_value`), but it is still a deliberate loosening and belongs in
        // the same announcement as `scan.exclude`.
        let silenced = self
            .vars
            .values()
            .filter(|v| v.secret == Some(false))
            .count();
        if silenced > 0 {
            out.push(format!("vars.secret=false on {silenced} variable(s)"));
        }
        out
    }

    /// The template a command compares against: the flag, else
    /// `[defaults] example`, else `.env.example`. One rule for validate, sync
    /// and spec init (I-189).
    pub fn example_path(&self, flag: Option<String>) -> String {
        pick(
            flag,
            self.defaults.example.clone(),
            ".env.example".to_string(),
        )
    }

    /// Dotted names of every key this version does not recognise.
    fn unknown_keys(&self) -> Vec<String> {
        let mut out: Vec<String> = self.extra.keys().cloned().collect();
        let sections: [(&str, &Extra); 7] = [
            ("defaults", &self.defaults.extra),
            ("scan", &self.scan.extra),
            ("validate", &self.validate.extra),
            ("sync", &self.sync.extra),
            ("diff", &self.diff.extra),
            ("backup", &self.backup.extra),
            ("cloud", &self.cloud.extra),
        ];
        for (name, extra) in sections {
            out.extend(extra.keys().map(|k| format!("{name}.{k}")));
        }
        // An array of tables has no `extra` of its own to walk, so each entry
        // reports its own unknown keys. A misspelled *required* key never
        // reaches here — serde refuses the file outright with "missing field".
        for (i, rule) in self.scan.patterns.iter().flatten().enumerate() {
            out.extend(rule.extra.keys().map(|k| format!("scan.patterns[{i}].{k}")));
        }
        // ⚠️ Each `[vars.X]` too (I-187, DEC-SP5). `requird = false` left the
        // variable required and `secrett = true` left it undeclared, silently.
        for (name, var) in &self.vars {
            out.extend(
                var.unknown_keys()
                    .into_iter()
                    .map(|k| format!("vars.{name}.{k}")),
            );
        }
        out.sort();
        out
    }
}

/// The templates (`.env.example`, `.env.sample`, `.env.template`) present in
/// `project_root`, in that order.
pub fn templates_in(project_root: &Path) -> Vec<String> {
    super::env_name::TEMPLATE_SUFFIXES
        .iter()
        .map(|suffix| format!(".env.{suffix}"))
        .filter(|name| project_root.join(name).is_file())
        .collect()
}

/// A project's template, by one rule (I-294, DQ-28): `[defaults] example` in
/// the project's `.evnx.toml`, else the first of `.env.example`, `.env.sample`,
/// `.env.template` present, else `.env.example`.
///
/// ⚠️ Written for doctor (I-294) and moved here in 0.10.16 (R-init) so the
/// commands that *write* a template follow it too. `init` and `add` wrote
/// `.env.example` whatever the project used: beside a `.env.sample` that was a
/// second template, and with `[defaults] example` set, `validate` right after
/// compared against a file init had not written (I-295). validate, sync, diff
/// and spec init still use [`Config::example_path`] (no fallback to the other
/// two names); switching them is recorded in their R- notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// As a person names it from the project folder.
    pub name: String,
    /// Named by `[defaults] example`, so it is the template even when missing.
    pub configured: bool,
}

impl Default for Template {
    fn default() -> Self {
        Template {
            name: ".env.example".to_string(),
            configured: false,
        }
    }
}

impl Template {
    pub fn of(project_root: &Path, config: &Config) -> Template {
        match &config.defaults.example {
            Some(name) => Template {
                name: name.clone(),
                configured: true,
            },
            None => Template {
                name: templates_in(project_root)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| ".env.example".to_string()),
                configured: false,
            },
        }
    }

    /// Relative to the project folder; an absolute name is used as written.
    pub fn path(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.name)
    }

    pub fn exists(&self, project_root: &Path) -> bool {
        self.path(project_root).is_file()
    }
}

/// Combine a flag with a config value and a built-in default.
///
/// **Precedence: flag > config > default.** The flag is `Option` precisely so
/// this can tell "the user asked for the default" from "the user asked for
/// nothing" — with a clap `default_value` the two are the same string and the
/// config could never take effect.
pub fn pick<T: Clone>(flag: Option<T>, configured: Option<T>, fallback: T) -> T {
    flag.or(configured).unwrap_or(fallback)
}

/// Combine a boolean flag with a config value.
///
/// ⚠️ Either saying `true` wins, which means a flag can **tighten** what config
/// set but never loosen it. That is deliberate rather than a limitation of clap:
/// there is no `--no-strict`, so "off" is indistinguishable from "unspecified",
/// and resolving the ambiguity toward the stricter reading is the safe direction
/// for a tool whose booleans are all guards.
pub fn any(flag: bool, configured: Option<bool>) -> bool {
    flag || configured.unwrap_or(false)
}

/// A `--x` / `--no-x` pair as one choice for [`pick`]: `Some(true)`,
/// `Some(false)`, or `None` when neither was given, so the config decides.
///
/// ⚠️ For a boolean that **loosens** a scan (I-057). [`any`] lets a flag only
/// tighten what config set, which is the safe reading for a guard; for
/// `ignore_placeholders` the safe direction is the other one, and a project's
/// `true` must be undoable for one run. clap's `overrides_with` keeps only the
/// last of the pair, so both are never set.
pub fn either(on: bool, off: bool) -> Option<bool> {
    if off {
        Some(false)
    } else if on {
        Some(true)
    } else {
        None
    }
}

/// Append configured entries to whatever the flag supplied.
///
/// Lists are additive rather than replaced: `[scan] exclude` is the project's
/// standing exclusions and `--exclude` is this run's, and a flag silently
/// dropping the project's list would be a surprising way to widen a scan.
pub fn extend(mut flag: Vec<String>, configured: Option<Vec<String>>) -> Vec<String> {
    if let Some(extra) = configured {
        for item in extra {
            if !flag.contains(&item) {
                flag.push(item);
            }
        }
    }
    flag
}

/// A config file that was found, parsed, and whatever could not be understood.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub config: Config,
    /// `None` when no `.evnx.toml` governs this directory.
    pub source: Option<PathBuf>,
    /// One entry per unrecognised key, ready to print.
    pub warnings: Vec<String>,
}

/// Locate the `.evnx.toml` governing `start`, if any.
///
/// Walks upward and **stops once a directory containing `.git` has been
/// examined** — the project boundary — so a parent repository's policy cannot
/// leak into a nested one, and `$HOME` is never reached.
pub fn find(start: &Path) -> Option<PathBuf> {
    walk(start).found
}

/// The bare spelling, read by nothing (I-040).
const BARE_FILE: &str = "evnx.toml";

/// What the upward walk met.
struct Walk {
    /// The `.evnx.toml` governing the start.
    found: Option<PathBuf>,
    /// An `evnx.toml` in the directory where the walk stopped: beside `found`,
    /// or — with `found` empty — where a `.evnx.toml` was evidently meant to be.
    bare: Option<PathBuf>,
}

fn walk(start: &Path) -> Walk {
    // ⚠️ Absolute first. The walk is `Path::parent`, and `Path::new(".").parent()`
    // is `""` — not the parent directory — so a relative start makes the loop
    // examine the working directory and stop, silently finding nothing above it.
    // `main.rs` passed `"."` and this went unnoticed because every unit test here
    // supplies an absolute `TempDir`.
    //
    // ⚠️ `absolute`, **not** `canonicalize`: this is a lexical operation that does
    // not resolve symlinks. On macOS `/var` is a symlink to `/private/var`, so
    // canonicalizing a `TempDir` path rewrites it and the returned path stops
    // matching the one the caller asked about. That failed CI on macOS while
    // passing on Linux, where `/tmp` is a real directory. It also means a
    // directory that does not exist is still answered for, rather than erroring.
    let absolute = std::path::absolute(start).ok();
    let mut dir = absolute.as_deref().or(Some(start));
    while let Some(d) = dir {
        let candidate = d.join(PROJECT_FILE);
        let bare = Some(d.join(BARE_FILE)).filter(|b| b.is_file());
        if candidate.is_file() {
            return Walk {
                found: Some(candidate),
                bare,
            };
        }
        // A bare file where the walk would otherwise pass by: the config the
        // author meant, which nothing reads (I-040). Stop here and say so.
        if bare.is_some() {
            return Walk { found: None, bare };
        }
        // Checked *after* looking in this directory, so a repository root
        // holding the config is still found.
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    Walk {
        found: None,
        bare: None,
    }
}

/// Load the config governing `start`.
///
/// A missing file is not an error — most projects have none. A **malformed** one
/// is: quietly ignoring a config that was written on purpose would mean running
/// with policy the user believes is in force. So is a bare `evnx.toml` with no
/// `.evnx.toml` beside it, for the same reason (I-040, DEC-SP15).
pub fn load(start: &Path) -> Result<Loaded> {
    let w = walk(start);
    let Some(path) = w.found else {
        if let Some(bare) = w.bare {
            bail!(
                "{} is not read: evnx reads only .evnx.toml, with the leading dot.\n\
                 \x20 rename it:  mv {BARE_FILE} {PROJECT_FILE}",
                bare.display()
            );
        }
        return Ok(Loaded::default());
    };
    let mut loaded = load_file(&path)?;
    if let (Some(example), Some(toml_dir)) = (&loaded.config.defaults.example, path.parent()) {
        loaded.config.defaults.example = Some(rebase(example, toml_dir, start));
    }
    if let Some(bare) = w.bare {
        loaded.warnings.insert(
            0,
            format!(
                "{} is ignored: {PROJECT_FILE} beside it is the config; merge the two and remove {BARE_FILE}",
                bare.display()
            ),
        );
    }
    Ok(loaded)
}

/// `[defaults] example` as seen from `start` (I-332).
///
/// A relative path in the file means "from the folder holding this file", the
/// way `.gitignore` and `docker-compose.yml` read theirs; every command reads
/// it from `start`. Until 0.11.0 it was read from `start` as written, so a
/// root `.evnx.toml` naming `deploy/compose/.env.example` sent a command run in
/// `deploy/compose` to `deploy/compose/deploy/compose/.env.example`.
/// Unchanged when the file is in `start` itself.
fn rebase(name: &str, toml_dir: &Path, start: &Path) -> String {
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let (dir, from) = (lexical(toml_dir), lexical(&start));
    if Path::new(name).is_absolute() || dir == from {
        return name.to_string();
    }
    let target = lexical(&dir.join(name));
    let (t, f): (Vec<_>, Vec<_>) = (target.components().collect(), from.components().collect());
    let common = t.iter().zip(&f).take_while(|(a, b)| a == b).count();
    if common == 0 {
        // Another drive on Windows: no relative path exists.
        return target.to_string_lossy().into_owned();
    }
    let mut rel = PathBuf::new();
    for _ in common..f.len() {
        rel.push("..");
    }
    rel.extend(&t[common..]);
    rel.to_string_lossy().into_owned()
}

/// `.` dropped and `..` applied, without touching the filesystem.
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Parse one specific file. Exposed for tests and for `--config`, later.
pub fn load_file(path: &Path) -> Result<Loaded> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let config: Config =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    let warnings = config
        .unknown_keys()
        .into_iter()
        .map(|key| {
            // A `[vars.X]` key has a short list to be a typo of (I-187).
            let hint = key
                .strip_prefix("vars.")
                .and_then(|rest| rest.rsplit('.').next())
                .and_then(crate::core::spec::known_key_like)
                .map(|k| format!(" — did you mean `{k}`?"))
                .unwrap_or_default();
            format!("{}: unknown key `{key}` (ignored){hint}", path.display())
        })
        .collect();

    Ok(Loaded {
        config,
        source: Some(path.to_path_buf()),
        warnings,
    })
}

/// The `.evnx.toml` to record into for `start` (SH-LOC; DEC-SP2, DEC-SP16).
///
/// The one [`find`] returns; when there is none, a new one at the project root
/// — the nearest directory at or above `start` holding `.git` — and, outside a
/// repository, in `start` itself.
///
/// ⚠️ Never a second file below the one in force. `spec init` used to write
/// `./.evnx.toml` while it had loaded `../.evnx.toml`, and the new file then
/// shadowed the root one — its `[scan]` policy stopped applying (I-177).
pub fn project_file(start: &Path) -> PathBuf {
    if let Some(found) = find(start) {
        return found;
    }
    let absolute = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let mut dir = Some(absolute.as_path());
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return d.join(PROJECT_FILE);
        }
        dir = d.parent();
    }
    absolute.join(PROJECT_FILE)
}

/// Does a `.evnx.toml`'s text declare `[vars]` already, in any form — `[vars]`,
/// `[vars.NAME]`, an inline `vars = { … }`, a dotted `vars.A.required = …`
/// (DEC-SP7)?
///
/// ⚠️ Text that does not parse is read line by line instead: refusing to add to
/// a file nobody can read matters more than being precise about it.
pub fn has_vars_section(text: &str) -> bool {
    match text.parse::<toml::Table>() {
        Ok(table) => table.contains_key("vars"),
        Err(_) => text.lines().map(str::trim).any(|l| {
            l == "[vars]"
                || l.starts_with("[vars.")
                || l.strip_prefix("vars")
                    .is_some_and(|rest| rest.trim_start().starts_with(['=', '.']))
        }),
    }
}

/// Write a `.evnx.toml` — the one writer, for `spec init`, the `[vars]`
/// writeback and `cloud link` (A-31).
///
/// ⛔ Text that does not parse is never written: every command reads this file
/// first, so one bad write stops all of them (I-178, I-179). The file is left as
/// it was and the error says why.
///
/// Otherwise `core::safe_write`'s contract: atomic, an existing file keeps its
/// mode, a new one is 0644 (the file is committed), and a symlink is written
/// through to its target (DEC-SP13). The caller decides whether to print
/// [`super::safe_write::report`].
pub fn save(path: &Path, text: &str) -> Result<super::safe_write::Written> {
    if let Err(e) = text.parse::<toml::Table>() {
        let inline = text.lines().any(|l| {
            l.trim_start()
                .strip_prefix("vars")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        });
        bail!(
            "changing {} would leave a file TOML refuses, so nothing was written:\n  {}{}",
            crate::utils::ui::short_path(path),
            e.to_string().lines().next().unwrap_or_default(),
            if inline {
                "\n  [vars] is an inline `vars = { … }` table, which cannot be extended:\n  \
                 write one [vars.NAME] table per variable, then run this again."
            } else {
                ""
            }
        );
    }
    super::safe_write::write_shared(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// DQ-28: `[defaults] example`, else the first template present, else
    /// `.env.example`.
    #[test]
    fn t_i295_the_template_rule() {
        let d = tempfile::TempDir::new().unwrap();
        let none = Config::default();
        assert_eq!(Template::of(d.path(), &none), Template::default());
        std::fs::write(d.path().join(".env.template"), "").unwrap();
        std::fs::write(d.path().join(".env.sample"), "").unwrap();
        assert_eq!(Template::of(d.path(), &none).name, ".env.sample");
        let named: Config = toml::from_str("[defaults]\nexample = \"config/env.tpl\"\n").unwrap();
        let t = Template::of(d.path(), &named);
        assert_eq!((t.name.as_str(), t.configured), ("config/env.tpl", true));
        assert!(!t.exists(d.path()));
        assert_eq!(t.path(d.path()), d.path().join("config/env.tpl"));
    }

    /// I-332: from the toml's folder, re-expressed from `start`.
    #[test]
    fn i332_rebase_reads_example_from_the_config_folder() {
        let root = std::path::absolute(Path::new("proj")).unwrap();
        let sub = root.join("deploy/compose");
        let r = |name: &str, start: &Path| rebase(name, &root, start).replace('\\', "/");
        assert_eq!(r("deploy/compose/.env.example", &sub), ".env.example");
        assert_eq!(
            r("deploy/compose/.env.example", &root),
            "deploy/compose/.env.example"
        );
        assert_eq!(r("./template.env", &root), "./template.env");
        assert_eq!(r("template.env", &sub), "../../template.env");
        assert_eq!(
            r("../shared/.env.example", &sub),
            "../../../shared/.env.example"
        );
        assert_eq!(r("other/.env.example", &sub), "../../other/.env.example");
    }

    fn write(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join(PROJECT_FILE);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_project_without_a_config_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let loaded = load(dir.path()).unwrap();

        assert!(loaded.source.is_none());
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn every_section_round_trips() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            r#"
[defaults]
env_name = "development"
example  = "template.env"

[scan]
severity            = "high"
exclude             = ["fixtures/", "*.test.js"]
ignore_placeholders = true

[validate]
strict           = true
validate_formats = true
ignore           = ["extra_variable"]

[sync]
naming_policy = "error"

[diff]
ignore_keys = ["DATABASE_URL"]

[backup]
keep = 5

[cloud]
vault = "my-api/production"
"#,
        );

        let c = load(dir.path()).unwrap().config;

        assert_eq!(c.defaults.env_name.as_deref(), Some("development"));
        assert_eq!(c.defaults.example.as_deref(), Some("template.env"));
        assert_eq!(c.scan.severity.as_deref(), Some("high"));
        assert_eq!(c.scan.exclude.as_deref().unwrap().len(), 2);
        assert_eq!(c.scan.ignore_placeholders, Some(true));
        assert_eq!(c.validate.strict, Some(true));
        assert_eq!(c.validate.ignore.as_deref().unwrap(), ["extra_variable"]);
        assert_eq!(c.sync.naming_policy.as_deref(), Some("error"));
        assert_eq!(c.diff.ignore_keys.as_deref().unwrap(), ["DATABASE_URL"]);
        assert_eq!(c.backup.keep, Some(5));
        assert_eq!(c.cloud.vault.as_deref(), Some("my-api/production"));
    }

    /// ⚠️ `Option` throughout is not decoration. Precedence needs to tell "the
    /// user set this to the default" from "the user set nothing", or a config
    /// would override a flag with a value nobody wrote.
    #[test]
    fn an_absent_key_is_none_not_a_default() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "[scan]\nseverity = \"high\"\n");

        let c = load(dir.path()).unwrap().config;
        assert_eq!(c.scan.severity.as_deref(), Some("high"));
        assert_eq!(c.scan.ignore_placeholders, None, "unset must stay None");
        assert_eq!(c.validate.strict, None);
    }

    /// A config from a newer evnx must not break an older one — the file is
    /// committed and teams run mixed versions.
    #[test]
    fn unknown_keys_warn_rather_than_fail() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "[scan]\nseverity = \"high\"\nentropy_threshold = 4.5\n\n[nonesuch]\nx = 1\n",
        );

        let loaded = load(dir.path()).unwrap();

        assert_eq!(
            loaded.config.scan.severity.as_deref(),
            Some("high"),
            "the keys it does understand must still apply"
        );
        let joined = loaded.warnings.join("\n");
        assert!(joined.contains("scan.entropy_threshold"), "{joined}");
        assert!(joined.contains("nonesuch"), "{joined}");
    }

    /// The old documented schema is the realistic case: anyone who copied it gets
    /// told exactly which of their settings do nothing, instead of eight sections
    /// silently activating on upgrade.
    #[test]
    fn the_previously_documented_schema_is_reported_key_by_key() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "[project]\nname = \"my-app\"\n\n[output]\nformat = \"json\"\n\n[scan]\nmin_severity = \"high\"\n",
        );

        let joined = load(dir.path()).unwrap().warnings.join("\n");
        for expected in ["project", "output", "scan.min_severity"] {
            assert!(
                joined.contains(expected),
                "expected {expected} in:\n{joined}"
            );
        }
    }

    #[test]
    fn a_malformed_config_is_an_error_not_a_silent_default() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "[scan\nseverity = ");

        let err = load(dir.path()).unwrap_err();
        assert!(format!("{err:#}").contains(PROJECT_FILE), "{err:#}");
    }

    #[test]
    fn the_search_walks_up_to_the_project_root() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        write(dir.path(), "[scan]\nseverity = \"high\"\n");

        let found = find(&dir.path().join("a/b/c")).expect("should walk up");
        assert_eq!(found, dir.path().join(PROJECT_FILE));
    }

    /// ⚠️ The boundary that stops one repository's policy reaching another.
    #[test]
    fn the_search_stops_at_a_git_boundary() {
        let outer = TempDir::new().unwrap();
        write(outer.path(), "[scan]\nseverity = \"high\"\n");

        let inner = outer.path().join("vendored");
        fs::create_dir_all(inner.join(".git")).unwrap();

        assert!(
            find(&inner).is_none(),
            "a nested repository must not inherit the outer project's policy"
        );

        // But a config *at* the repository root is still found.
        write(&inner, "[scan]\nseverity = \"low\"\n");
        assert_eq!(find(&inner), Some(inner.join(PROJECT_FILE)));
    }

    /// ⚠️ The path handed back must be the one the caller asked about.
    ///
    /// Reproduces on any platform the shape macOS has natively: `/var` is a
    /// symlink to `/private/var`, so a `TempDir` there has two valid spellings.
    /// `find` resolving symlinks would return the other one, and the returned
    /// path would stop matching what the caller passed in — which is exactly how
    /// this failed CI on macOS while passing on Linux.
    #[test]
    fn the_returned_path_keeps_the_callers_spelling_through_a_symlink() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("real");
        fs::create_dir_all(real.join("a/b")).unwrap();
        write(&real, "[scan]\nseverity = \"high\"\n");

        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &link).unwrap();

        // Asked about via the symlink, answered about via the symlink.
        let found = find(&link.join("a/b")).expect("should walk up");
        assert_eq!(
            found,
            link.join(PROJECT_FILE),
            "find must not rewrite the caller's path through the symlink"
        );
    }

    /// Bare `evnx.toml` is not accepted: two spellings needs a precedence rule
    /// for no gain.
    #[test]
    fn only_the_dotted_name_is_read() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("evnx.toml"),
            "[scan]\nseverity = \"high\"\n",
        )
        .unwrap();

        assert!(find(dir.path()).is_none());
    }

    /// I-040, DEC-SP15: not silently. Alone, it stops the command and names
    /// the rename; beside `.evnx.toml`, one warning.
    #[test]
    fn t_i040_a_bare_evnx_toml_is_an_error_alone_and_a_warning_beside() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join("evnx.toml"), "[scan]\n").unwrap();
        let err = format!("{:#}", load(dir.path()).unwrap_err());
        assert!(err.contains("mv evnx.toml .evnx.toml"), "{err}");

        // From below too: the walk meets it before anything else.
        fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(load(&dir.path().join("sub")).is_err());

        write(dir.path(), "[scan]\nseverity = \"high\"\n");
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.config.scan.severity.as_deref(), Some("high"));
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(loaded.warnings[0].contains("evnx.toml is ignored"));
    }

    /// I-187, DEC-SP5: a typo under `[vars.X]` is named, with the key meant.
    #[test]
    fn t_i187_unknown_vars_keys_warn_with_the_key_meant() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "[vars.A]\nrequird = false\n\n[vars.B]\nsecrett = true\nzzz = 1\n",
        );
        let joined = load(dir.path()).unwrap().warnings.join("\n");
        assert!(
            joined.contains("`vars.A.requird` (ignored) — did you mean `required`?"),
            "{joined}"
        );
        assert!(joined.contains("did you mean `secret`?"), "{joined}");
        // Nothing close: no guess.
        assert!(
            joined
                .lines()
                .any(|l| l.ends_with("`vars.B.zzz` (ignored)")),
            "{joined}"
        );
    }

    /// SH-LOC (I-177, DEC-SP2, DEC-SP16): the config in force, else the git
    /// root, else the start.
    #[test]
    fn t_i177_project_file_is_the_config_in_force_or_the_git_root() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("a/b");
        fs::create_dir_all(&sub).unwrap();
        // No repository: the start itself.
        assert_eq!(project_file(&sub), sub.join(PROJECT_FILE));

        fs::create_dir(dir.path().join(".git")).unwrap();
        assert_eq!(project_file(&sub), dir.path().join(PROJECT_FILE));

        // A config in between is the one in force.
        write(&dir.path().join("a"), "[scan]\n");
        assert_eq!(project_file(&sub), dir.path().join("a").join(PROJECT_FILE));
    }

    /// DEC-SP7: one answer, from the parsed file, however `[vars]` is written.
    #[test]
    fn has_vars_section_sees_every_form() {
        for yes in [
            "[vars.A]\n",
            "  [vars.DATABASE_URL]  \n",
            "[vars]\n",
            "vars = { A = { required = false } }\n",
            "vars.A.required = false\n",
            // Does not parse: read line by line.
            "[vars.A]\nrequired = \n",
            "vars = { A = \n",
        ] {
            assert!(has_vars_section(yes), "{yes:?}");
        }
        for no in [
            "[scan]\nseverity = \"high\"\n",
            "# [vars.A] was here\n",
            "[varsomething]\n",
            "[scan]\nnote = \"[vars.A]\"\n",
            "",
        ] {
            assert!(!has_vars_section(no), "{no:?}");
        }
    }

    /// The one writer refuses text that does not parse, and leaves the file.
    #[test]
    fn save_writes_only_what_parses() {
        let dir = TempDir::new().unwrap();
        let p = write(dir.path(), "vars = { A = { required = false } }\n");
        let bad = "vars = { A = { required = false } }\n\n[vars.B]\nrequired = true\n";
        let err = format!("{:#}", save(&p, bad).unwrap_err());
        assert!(err.contains("nothing was written"), "{err}");
        assert!(err.contains("inline"), "{err}");
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "vars = { A = { required = false } }\n"
        );

        let w = save(&p, "[vars.B]\nrequired = true\n").unwrap();
        assert_eq!(w.target, p);
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "[vars.B]\nrequired = true\n"
        );
    }

    #[test]
    fn security_overrides_are_listed_for_announcement() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "[scan]\nseverity = \"high\"\nexclude = [\"a\", \"b\"]\n\n[validate]\nstrict = true\n",
        );

        let overrides = load(dir.path()).unwrap().config.security_overrides();
        assert!(overrides.iter().any(|o| o.contains("scan.severity=high")));
        assert!(overrides.iter().any(|o| o.contains("2 patterns")));
        assert!(
            !overrides.iter().any(|o| o.contains("validate")),
            "validate.strict tightens; only loosening is announced: {overrides:?}"
        );
    }

    /// I-291: the size reads as the skip note says it, and I-292: a disabled
    /// heuristic is a detector, not a rule.
    #[test]
    fn i291_i292_overrides_say_sizes_and_detectors_plainly() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "[scan]\nmax_file_size = 524288\ndisable = [\"Sensitive config key\"]\n",
        );
        let overrides = load(dir.path()).unwrap().config.security_overrides();
        assert!(
            overrides
                .iter()
                .any(|o| o.starts_with("scan.max_file_size=512 KiB")),
            "{overrides:?}"
        );
        assert!(
            overrides
                .iter()
                .any(|o| o == "scan.disable=1 built-in detector(s)"),
            "{overrides:?}"
        );
    }

    #[test]
    fn a_config_that_changes_nothing_security_relevant_announces_nothing() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "[defaults]\nenv_name = \"production\"\n");

        assert!(load(dir.path())
            .unwrap()
            .config
            .security_overrides()
            .is_empty());
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    #[test]
    fn a_flag_beats_config_which_beats_the_default() {
        assert_eq!(pick(Some("flag"), Some("config"), "default"), "flag");
        assert_eq!(pick(None, Some("config"), "default"), "config");
        assert_eq!(pick(None, None, "default"), "default");
    }

    /// ⚠️ The reason the flags became `Option`. With a clap `default_value`,
    /// "the user typed --severity low" and "the user typed nothing" arrive as the
    /// same string, so config could never apply.
    #[test]
    fn asking_for_the_default_explicitly_still_beats_config() {
        assert_eq!(pick(Some("low"), Some("high"), "low"), "low");
    }

    #[test]
    fn a_boolean_flag_tightens_but_never_loosens() {
        assert!(any(true, Some(false)), "the flag can turn it on");
        assert!(any(false, Some(true)), "config can turn it on");
        assert!(any(true, None));
        assert!(!any(false, Some(false)));
        assert!(!any(false, None));
    }

    #[test]
    fn lists_combine_without_duplicating() {
        let got = extend(
            vec!["a".into(), "b".into()],
            Some(vec!["b".into(), "c".into()]),
        );
        assert_eq!(got, vec!["a", "b", "c"]);
    }

    #[test]
    fn an_absent_list_leaves_the_flag_alone() {
        assert_eq!(extend(vec!["a".into()], None), vec!["a"]);
    }
}
