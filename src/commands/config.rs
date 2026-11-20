//! `evnx config show` — every effective setting and where it came from.
//!
//! evnx reads settings from several places and the most useful question about
//! any of them is "why is it that?". This answers it without running a command:
//! one row per setting, `key  value  source`, where the source is one of
//!
//! * `default` — nothing set it; the built-in value is shown;
//! * `.evnx.toml:LINE` — the project file, with the line of the key;
//! * `<path>:LINE` — the user's `config.toml` (cloud builds);
//! * `env NAME` — an environment variable;
//! * `flag --name` — a global flag on this command line;
//! * `policy <file>` — a scan policy file (`EVNX_POLICY`).
//!
//! ⚠️ **A value that may be a secret is never printed.** Credentials are shown
//! as `set (hidden)` or `not set`; nothing from `[vars]` is read for display
//! beyond a count; a server URL that does not validate is reported without its
//! text (a URL can carry a password). The values that are shown are choices:
//! a severity, a path, a list of globs.
//!
//! Per-command flags are not settings and are not listed: `scan --severity`
//! applies to one run. The precedence for those is flag, then `.evnx.toml`,
//! then the default (`core::config::pick`).

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use crate::core::config::{self, Config};

/// What was asked for.
pub struct Options {
    pub json: bool,
    /// The global flags, as given on this command line.
    pub quiet: bool,
    pub verbose: bool,
    pub no_color: bool,
}

/// One effective setting.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Setting {
    pub key: String,
    /// `null` when nothing is set and there is no built-in value.
    pub value: Value,
    pub source: String,
}

/// A file that was looked for.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FileRead {
    /// `project`, `user` or `policy`.
    pub role: &'static str,
    /// Where it is, or `null` when there is none.
    pub path: Option<String>,
}

/// The whole answer.
#[derive(Debug, Serialize)]
pub struct Report {
    pub files: Vec<FileRead>,
    pub settings: Vec<Setting>,
    /// Keys this version does not know, as the loader words them.
    pub warnings: Vec<String>,
}

type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// A variable that is set and not blank.
fn env_text(env: Env, name: &str) -> Option<String> {
    env(name)
        .and_then(|v| v.into_string().ok())
        .filter(|v| !v.trim().is_empty())
}

fn row(key: &str, value: Value, source: impl Into<String>) -> Setting {
    Setting {
        key: key.to_string(),
        value,
        source: source.into(),
    }
}

/// `dotted key -> line` for the keys of a TOML file, one level below a table
/// (`scan.severity`) and the tables themselves (`scan`).
fn lines_of(text: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    let Ok(doc) = toml_edit::ImDocument::parse(text) else {
        return out;
    };
    let line = |offset: usize| {
        1 + text.as_bytes()[..offset.min(text.len())]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    };
    let table = doc.as_table();
    for (name, item) in table {
        let Some((key, _)) = table.get_key_value(name) else {
            continue;
        };
        if let Some(span) = key.span().or_else(|| item.span()) {
            out.insert(name.to_string(), line(span.start));
        }
        if let Some(inner) = item.as_table() {
            for (sub, sub_item) in inner {
                if let Some((k, _)) = inner.get_key_value(sub) {
                    if let Some(span) = k.span().or_else(|| sub_item.span()) {
                        out.insert(format!("{name}.{sub}"), line(span.start));
                    }
                }
            }
        }
    }
    out
}

/// The rows of `.evnx.toml`: every key evnx reads, with the built-in value
/// where nothing sets it.
fn project_rows(cfg: &Config, file: Option<&Path>, text: Option<&str>) -> Vec<Setting> {
    let lines = text.map(lines_of).unwrap_or_default();
    let shown = file.map(crate::utils::ui::short_path);
    // The source of `key`: its line in the file when the file set it.
    let set = |key: &str| -> String {
        match (&shown, lines.get(key)) {
            (Some(f), Some(n)) => format!("{f}:{n}"),
            (Some(f), None) => f.clone(),
            _ => "default".to_string(),
        }
    };
    let pick = |key: &str, configured: Option<Value>, default: Value| -> Setting {
        match configured {
            Some(v) => row(key, v, set(key)),
            None => row(key, default, "default"),
        }
    };
    let list = |v: &Option<Vec<String>>| v.as_ref().map(|v| json!(v));
    vec![
        pick(
            "defaults.env_name",
            cfg.defaults.env_name.as_ref().map(|v| json!(v)),
            Value::Null,
        ),
        pick(
            "defaults.example",
            cfg.defaults.example.as_ref().map(|v| json!(v)),
            Value::Null,
        ),
        pick(
            "scan.severity",
            cfg.scan.severity.as_ref().map(|v| json!(v)),
            json!("low"),
        ),
        pick(
            "scan.confidence",
            cfg.scan.confidence.as_ref().map(|v| json!(v)),
            json!("low"),
        ),
        pick("scan.exclude", list(&cfg.scan.exclude), json!([])),
        pick(
            "scan.ignore_placeholders",
            cfg.scan.ignore_placeholders.map(|v| json!(v)),
            json!(false),
        ),
        pick("scan.disable", list(&cfg.scan.disable), json!([])),
        pick(
            "scan.builtins",
            cfg.scan.builtins.map(|v| json!(v)),
            json!(true),
        ),
        pick("scan.extensions", list(&cfg.scan.extensions), Value::Null),
        pick(
            "scan.max_file_size",
            cfg.scan.max_file_size.as_ref().map(|v| match v {
                config::SizeSetting::Bytes(n) => json!(n),
                config::SizeSetting::Text(t) => json!(t),
            }),
            json!("1MB"),
        ),
        // A count: the expressions are the project's, but a list of regexes is
        // not a setting anyone reads here.
        pick(
            "scan.patterns",
            cfg.scan.patterns.as_ref().map(|p| json!(p.len())),
            json!(0),
        ),
        pick(
            "validate.strict",
            cfg.validate.strict.map(|v| json!(v)),
            json!(false),
        ),
        pick(
            "validate.validate_formats",
            cfg.validate.validate_formats.map(|v| json!(v)),
            json!(true),
        ),
        pick("validate.ignore", list(&cfg.validate.ignore), json!([])),
        pick(
            "validate.fail_on",
            cfg.validate.fail_on.as_ref().map(|v| json!(v)),
            json!("error"),
        ),
        pick(
            "sync.naming_policy",
            cfg.sync.naming_policy.as_ref().map(|v| json!(v)),
            json!("warn"),
        ),
        pick("diff.ignore_keys", list(&cfg.diff.ignore_keys), json!([])),
        pick("backup.keep", cfg.backup.keep.map(|v| json!(v)), json!(3)),
        pick(
            "cloud.vault",
            cfg.cloud.vault.as_ref().map(|v| json!(v)),
            Value::Null,
        ),
        pick(
            "cloud.file",
            cfg.cloud.file.as_ref().map(|v| json!(v)),
            Value::Null,
        ),
        // The contract: how many variables it declares, never what they say.
        pick(
            "vars",
            (!cfg.vars.is_empty()).then(|| json!(cfg.vars.len())),
            json!(0),
        ),
    ]
}

/// The environment-level rows that need no feature: the global flags, colour
/// and the scan policy.
fn tool_rows(env: Env, opts: &Options) -> Result<(Vec<Setting>, FileRead)> {
    let mut rows = Vec::new();
    let flag_or_default = |key: &str, on: bool, flag: &str| {
        if on {
            row(key, json!(true), format!("flag {flag}"))
        } else {
            row(key, json!(false), "default")
        }
    };
    rows.push(flag_or_default("verbose", opts.verbose, "--verbose"));
    rows.push(flag_or_default("quiet", opts.quiet, "--quiet"));
    rows.push(if opts.no_color {
        row("color", json!("off"), "flag --no-color")
    } else if env("NO_COLOR").is_some() {
        row("color", json!("off"), "env NO_COLOR")
    } else {
        row("color", json!("auto"), "default")
    });
    rows.push(match env_text(env, "EVNX_OUTPUT_JSON") {
        Some(v) => row("output_json", json!(v), "env EVNX_OUTPUT_JSON"),
        None => row("output_json", json!(false), "default"),
    });

    let mut policy_file = FileRead {
        role: "policy",
        path: None,
    };
    match env_text(env, "EVNX_POLICY") {
        Some(path) => {
            rows.push(row("policy", json!(path), "env EVNX_POLICY"));
            let policy = crate::core::policy::Policy::load(Path::new(&path))?;
            let shown = crate::utils::ui::short_path(policy.file());
            for (key, value) in policy.settings() {
                rows.push(row(key, value, format!("policy {shown}")));
            }
            policy_file.path = Some(shown);
        }
        None => rows.push(row("policy", Value::Null, "default")),
    }
    Ok((rows, policy_file))
}

/// The account rows: server, config directory and credentials.
#[cfg(feature = "cloud")]
fn cloud_rows(env: Env) -> Result<(Vec<Setting>, FileRead)> {
    use crate::cloud::config::{self as cc, DEFAULT_SERVER, ENV_SERVER};
    let mut rows = Vec::new();
    let dir = cc::config_dir_with(env)?;
    rows.push(match env_text(env, cc::ENV_CONFIG_DIR) {
        Some(_) => row(
            "config_dir",
            json!(tilde(&dir)),
            format!("env {}", cc::ENV_CONFIG_DIR),
        ),
        None => row("config_dir", json!(tilde(&dir)), "default"),
    });

    let file = dir.join("config.toml");
    let text = std::fs::read_to_string(&file).ok();
    let from_file = cc::CloudConfig::load_from(&file)
        .with_context(|| format!("{} could not be read", tilde(&file)))?
        .server;
    let line = text
        .as_deref()
        .and_then(|t| lines_of(t).get("server").copied());
    let file_source = match line {
        Some(n) => format!("{}:{n}", tilde(&file)),
        None => tilde(&file),
    };
    let (raw, source) = if let Some(v) = env_text(env, ENV_SERVER) {
        (v, format!("env {ENV_SERVER}"))
    } else if let Some(v) = from_file {
        (v, file_source)
    } else {
        (DEFAULT_SERVER.to_string(), "default".to_string())
    };
    // ⚠️ The error would quote the value, and a URL can carry a password.
    let canonical = cc::canonical_server(&raw).map_err(|_| {
        anyhow::anyhow!(
            "the server setting from {source} is not a valid server URL (the value is not shown)"
        )
    })?;
    rows.push(row("server", json!(canonical), source));

    rows.push(match env_text(env, "EVNX_MIN_VERSION") {
        Some(v) => row("cloud.min_version", json!(v), "env EVNX_MIN_VERSION"),
        None => row("cloud.min_version", Value::Null, "default"),
    });

    // Credentials: whether they are set, never what they are. The `_FILE`
    // names are included: a path to a secret is still a place to look.
    for (key, name) in [
        ("token", "EVNX_TOKEN"),
        ("password", "EVNX_PASSWORD"),
        ("master_password", "EVNX_MASTER_PASSWORD"),
        ("master_password_file", "EVNX_MASTER_PASSWORD_FILE"),
        ("key_bundle", "EVNX_KEY_BUNDLE"),
        ("key_bundle_file", "EVNX_KEY_BUNDLE_FILE"),
    ] {
        rows.push(if env(name).is_some_and(|v| !v.is_empty()) {
            row(key, json!("set (hidden)"), format!("env {name}"))
        } else {
            row(key, json!("not set"), "default")
        });
    }
    Ok((
        rows,
        FileRead {
            role: "user",
            path: file.is_file().then(|| tilde(&file)),
        },
    ))
}

/// A path with the home folder written as `~`.
#[cfg(feature = "cloud")]
fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|h| path.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Build the report for the folder `here`.
pub fn build(here: &Path, env: Env, opts: &Options) -> Result<Report> {
    let loaded = config::load(here)?;
    let text = loaded
        .source
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let mut files = vec![FileRead {
        role: "project",
        path: loaded.source.as_deref().map(crate::utils::ui::short_path),
    }];
    let mut settings = project_rows(&loaded.config, loaded.source.as_deref(), text.as_deref());

    let (rows, policy) = tool_rows(env, opts)?;
    settings.extend(rows);
    #[cfg(feature = "cloud")]
    {
        let (rows, user) = cloud_rows(env)?;
        settings.extend(rows);
        files.push(user);
    }
    files.push(policy);
    Ok(Report {
        files,
        settings,
        warnings: loaded.warnings,
    })
}

/// How a value reads in the table.
fn display(v: &Value) -> String {
    match v {
        Value::Null => "unset".to_string(),
        Value::String(s) => s.clone(),
        Value::Array(a) if a.is_empty() => "[]".to_string(),
        Value::Array(a) => a.iter().map(display).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// `evnx config show`.
pub fn show(opts: Options) -> Result<()> {
    let here = std::env::current_dir().context("reading the current directory")?;
    let report = build(&here, &|name| std::env::var_os(name), &opts)?;
    if opts.json {
        crate::core::json::emit(&report, 1, None)?;
        return Ok(());
    }
    println!("  Files read");
    for f in &report.files {
        println!("    {:<8}  {}", f.role, f.path.as_deref().unwrap_or("none"));
    }
    println!();
    let kw = report
        .settings
        .iter()
        .map(|s| s.key.len())
        .max()
        .unwrap_or(3);
    let vals: Vec<String> = report.settings.iter().map(|s| display(&s.value)).collect();
    let vw = vals.iter().map(String::len).max().unwrap_or(5).min(48);
    println!("  {:<kw$}  {:<vw$}  SOURCE", "KEY", "VALUE");
    for (s, v) in report.settings.iter().zip(&vals) {
        println!("  {:<kw$}  {:<vw$}  {}", s.key, v, s.source);
    }
    if !report.warnings.is_empty() {
        println!();
        println!("  Warnings");
        for w in &report.warnings {
            println!("    {w}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    fn opts() -> Options {
        Options {
            json: false,
            quiet: false,
            verbose: false,
            no_color: false,
        }
    }

    fn get<'a>(r: &'a Report, key: &str) -> &'a Setting {
        r.settings.iter().find(|s| s.key == key).unwrap()
    }

    #[test]
    fn lines_are_those_of_the_keys() {
        let l = lines_of("[scan]\nseverity = \"high\"\n\n[backup]\nkeep = 5\n");
        assert_eq!(l["scan"], 1);
        assert_eq!(l["scan.severity"], 2);
        assert_eq!(l["backup.keep"], 5);
    }

    #[test]
    fn defaults_name_themselves_and_a_set_key_names_its_line() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::write(
            d.path().join(".evnx.toml"),
            "[scan]\n\nseverity = \"high\"\n",
        )
        .unwrap();
        let r = build(d.path(), &no_env, &opts()).unwrap();
        let sev = get(&r, "scan.severity");
        assert_eq!(sev.value, json!("high"));
        assert!(sev.source.ends_with(".evnx.toml:3"), "{}", sev.source);
        let keep = get(&r, "backup.keep");
        assert_eq!(
            (keep.value.clone(), keep.source.as_str()),
            (json!(3), "default")
        );
    }

    #[test]
    fn flags_and_env_are_named() {
        let d = tempfile::TempDir::new().unwrap();
        let env = |n: &str| (n == "NO_COLOR").then(|| OsString::from("1"));
        let mut o = opts();
        o.verbose = true;
        let r = build(d.path(), &env, &o).unwrap();
        assert_eq!(get(&r, "verbose").source, "flag --verbose");
        assert_eq!(get(&r, "color").source, "env NO_COLOR");
        o.no_color = true;
        let r = build(d.path(), &env, &o).unwrap();
        assert_eq!(get(&r, "color").source, "flag --no-color");
    }

    #[test]
    fn display_reads_each_kind_of_value() {
        assert_eq!(display(&Value::Null), "unset");
        assert_eq!(display(&json!([])), "[]");
        assert_eq!(display(&json!(["a", "b"])), "a, b");
        assert_eq!(display(&json!(true)), "true");
        assert_eq!(display(&json!("x")), "x");
    }
}
