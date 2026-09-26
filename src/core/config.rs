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
//! wins, for no gain.
//!
//! # Evolution
//!
//! **Additive only.** Keys are added, never repurposed or removed, so a file
//! written for a later evnx keeps working on an earlier one — which matters
//! because this file is committed and teams run mixed versions.
//!
//! **Unknown keys warn; they never fail**, for the same reason. But a typo must
//! not be silent either, so the warning names the key.

use anyhow::{Context, Result};
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
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ValidatePolicy {
    pub strict: Option<bool>,
    pub validate_formats: Option<bool>,
    /// Issue types this project suppresses.
    pub ignore: Option<Vec<String>>,
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
                out.push(format!("scan.disable={} built-in rule(s)", names.len()));
            }
            _ => {}
        }
        if self.scan.builtins == Some(false) {
            out.push("scan.builtins=false (no built-in rules)".to_string());
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
        out.sort();
        out
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
        if candidate.is_file() {
            return Some(candidate);
        }
        // Checked *after* looking in this directory, so a repository root
        // holding the config is still found.
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent();
    }
    None
}

/// Load the config governing `start`.
///
/// A missing file is not an error — most projects have none. A **malformed** one
/// is: quietly ignoring a config that was written on purpose would mean running
/// with policy the user believes is in force.
pub fn load(start: &Path) -> Result<Loaded> {
    let Some(path) = find(start) else {
        return Ok(Loaded::default());
    };
    load_file(&path)
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
        .map(|key| format!("{}: unknown key `{key}` (ignored)", path.display()))
        .collect();

    Ok(Loaded {
        config,
        source: Some(path.to_path_buf()),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
