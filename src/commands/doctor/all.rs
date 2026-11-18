//! `evnx doctor --all`: the doctor checks, then `validate`, `sync --check` and
//! `scan`, as one report and one verdict.
//!
//! Each part is the command of that name, called as `main` calls it, so no check
//! is written twice. In JSON mode each part's own document is captured
//! ([`crate::core::json::capture`]) and placed under `sections`; in pretty mode
//! each part prints its own report under a heading.
//!
//! The verdict is the worst of the parts: 2 (no verdict) if any part could not
//! run, else 1 if any found something, else 0. `--fix` reaches the doctor part
//! only: it never runs `validate --fix` and never lets `sync` write.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use serde_json::{Map, Value, json};

use super::runner;
use crate::cli::args::ValidateFormat;
use crate::core::config::{Config, Template};
use crate::core::verdict::{self, Verdict};
use crate::utils::ui;

/// How one part ended.
enum Part {
    Ran(Verdict),
    Skipped(String),
    CouldNotRun(String),
}

impl Part {
    fn status(&self) -> &'static str {
        match self {
            Part::Ran(Verdict::Clean) => "clean",
            Part::Ran(_) => "findings",
            Part::Skipped(_) => "skipped",
            Part::CouldNotRun(_) => "could_not_run",
        }
    }
}

fn from_result(r: Result<Verdict>) -> Part {
    match r {
        Ok(v) => Part::Ran(v),
        Err(e) => Part::CouldNotRun(format!("{e:#}")),
    }
}

pub(super) fn run(
    project_root: &Path,
    config: &Config,
    template: &Template,
    verbose: bool,
    fix: bool,
    strict: bool,
    json_output: bool,
) -> Result<Verdict> {
    let mut sections = Map::new();
    let mut parts: Vec<(&'static str, Part)> = Vec::new();

    // 1. doctor
    if json_output {
        let report = runner::build_report(project_root, template, verbose, fix, |_| {});
        let verdict = runner::finish(&report, strict);
        sections.insert(
            "doctor".into(),
            section(&from_result(verdict_copy(&verdict)), report_value(&report)),
        );
        parts.push(("doctor", from_result(verdict)));
    } else {
        let verdict = runner::run_text(project_root, template, verbose, fix, strict, true);
        parts.push(("doctor", from_result(verdict)));
    }

    // The files the other three look at, by the one rule every command uses.
    let resolved = crate::core::resolve::resolve(
        project_root,
        crate::core::resolve::Flags::default(),
        config,
        crate::core::resolve::Need::MayCreate,
    );
    let (env, example) = match &resolved {
        Ok(r) => (
            project_root.join(&r.env).to_string_lossy().into_owned(),
            project_root
                .join(&r.template)
                .to_string_lossy()
                .into_owned(),
        ),
        Err(_) => (
            project_root.join(".env").to_string_lossy().into_owned(),
            template.path(project_root).to_string_lossy().into_owned(),
        ),
    };
    let have_env = Path::new(&env).is_file();
    let have_template = Path::new(&example).is_file();

    // 2. validate
    let validate = if !have_env {
        Part::Skipped(format!("{env} not found"))
    } else if !have_template && config.vars.is_empty() {
        Part::Skipped(format!("{example} not found and no [vars] contract"))
    } else {
        heading(json_output, "validate");
        let opts = crate::commands::validate::Options {
            env: env.clone(),
            input_format: crate::commands::validate::InputFormat::Dotenv,
            example: example.clone(),
            strict: config.validate.strict.unwrap_or(false),
            fix: false,
            dry_run: false,
            rotate_weak_secrets: false,
            format: if json_output {
                ValidateFormat::Json
            } else {
                ValidateFormat::Pretty
            },
            exit_zero: false,
            fail_on: None,
            fail_on_configured: config.validate.fail_on.clone(),
            verbose,
            ignore: Vec::new(),
            ignore_configured: config.validate.ignore.clone().unwrap_or_default(),
            validate_formats: config.validate.validate_formats.unwrap_or(true),
            spec: config.vars.clone(),
        };
        let (result, docs) = captured(json_output, || crate::commands::validate::run(opts));
        let part = from_result(result);
        add_section(&mut sections, json_output, "validate", &part, docs);
        part
    };
    note_skip(json_output, "validate", &validate, &mut sections);
    parts.push(("validate", validate));

    // 3. sync --check, two-way
    let sync = if !have_env {
        Part::Skipped(format!("{env} not found"))
    } else if !have_template {
        Part::Skipped(format!("no template ({example} not found)"))
    } else {
        heading(json_output, "sync --check");
        let naming = config
            .sync
            .naming_policy
            .as_deref()
            .and_then(|p| crate::cli::NamingPolicy::from_config(p).ok())
            .unwrap_or_default();
        let (result, docs) = captured(json_output, || {
            crate::commands::sync::run(crate::commands::sync::SyncOptions {
                env: env.clone(),
                example: example.clone(),
                direction: None,
                placeholder: false,
                verbose,
                dry_run: true,
                force: false,
                check: true,
                format: if json_output { "json" } else { "pretty" }.to_string(),
                template_config: None,
                naming_policy: naming,
                spec: config.vars.clone(),
                skip_without_env: false,
            })
        });
        let part = from_result(result);
        add_section(&mut sections, json_output, "sync", &part, docs);
        part
    };
    note_skip(json_output, "sync", &sync, &mut sections);
    parts.push(("sync", sync));

    // 4. scan the project
    heading(json_output, "scan");
    let scan_opts = scan_options(project_root, config, verbose, json_output);
    let (result, docs) = captured(json_output, || crate::commands::scan::run(scan_opts));
    let scan = from_result(result);
    add_section(&mut sections, json_output, "scan", &scan, docs);
    parts.push(("scan", scan));

    // One verdict: the worst of the parts.
    let not_run: Vec<&str> = parts
        .iter()
        .filter(|(_, p)| matches!(p, Part::CouldNotRun(_)))
        .map(|(n, _)| *n)
        .collect();
    let findings = parts
        .iter()
        .any(|(_, p)| matches!(p, Part::Ran(Verdict::Findings)));

    if json_output {
        for (name, part) in &parts {
            if let Part::CouldNotRun(msg) = part {
                if let Some(Value::Object(s)) = sections.get_mut(*name) {
                    s.entry("note").or_insert(json!(msg));
                }
            }
        }
        let verdict = if !not_run.is_empty() {
            "could_not_run"
        } else if findings {
            "findings"
        } else {
            "clean"
        };
        let doc = json!({
            "path": project_root.to_string_lossy(),
            "verdict": verdict,
            "sections": Value::Object(sections),
        });
        crate::core::json::emit(&doc, 2, None)?;
    } else {
        println!();
        println!("{}", "doctor --all".bold());
        for (name, part) in &parts {
            let line = match part {
                Part::Ran(Verdict::Clean) => "clean".green().to_string(),
                Part::Ran(_) => "findings".red().to_string(),
                Part::Skipped(why) => format!("skipped ({why})").dimmed().to_string(),
                Part::CouldNotRun(why) => format!("could not run ({why})").red().to_string(),
            };
            println!("  {name:<9} {line}");
        }
    }

    if !not_run.is_empty() {
        return Err(verdict::footed(
            None,
            format!(
                "No complete verdict: {} could not run. This is not a clean result.",
                not_run.join(", ")
            ),
        ));
    }
    Ok(if findings {
        Verdict::Findings
    } else {
        Verdict::Clean
    })
}

/// The scan of the whole project folder, from `[scan]` in its `.evnx.toml`.
fn scan_options(
    project_root: &Path,
    config: &Config,
    verbose: bool,
    json_output: bool,
) -> crate::commands::scan::ScanOptions {
    use crate::commands::scan;
    let s = &config.scan;
    let mut exclude = vec![".env.example".to_string()];
    exclude = crate::core::config::extend(exclude, s.exclude.clone());
    let max_file_size = s
        .max_file_size
        .as_ref()
        .and_then(|m| m.bytes().ok())
        .filter(|b| *b < scan::filters::MAX_WALK_BYTES);
    scan::ScanOptions {
        paths: vec![project_root.to_string_lossy().into_owned()],
        exclude,
        disable: s.disable.clone().unwrap_or_default(),
        patterns: scan::patternset::merge(
            Vec::new(),
            s.patterns.clone(),
            s.builtins.unwrap_or(true),
            s.disable.clone(),
        ),
        ignore_placeholders: s.ignore_placeholders == Some(true),
        severity: s.severity.clone().unwrap_or_else(|| "low".to_string()),
        confidence: s.confidence.clone().unwrap_or_else(|| "low".to_string()),
        short_values: s.confidence.as_deref() == Some("low"),
        project_root: project_root.to_path_buf(),
        format: if json_output {
            scan::OutputFormat::Json
        } else {
            scan::OutputFormat::Pretty
        },
        verbose,
        spec: config.vars.clone(),
        extensions: s
            .extensions
            .clone()
            .map(|e| (e, ".evnx.toml [scan] extensions")),
        max_file_size,
        builtins_disabled: s.builtins == Some(false),
        repo_settings: scan::models::RepoSettings {
            exclude: s.exclude.clone().unwrap_or_default(),
            ignore_placeholders: s.ignore_placeholders == Some(true),
            extensions: s.extensions.clone(),
            max_file_size,
            secret_false: config
                .vars
                .iter()
                .filter(|(_, v)| v.secret == Some(false))
                .map(|(k, _)| k.clone())
                .collect(),
        },
        ..scan::ScanOptions::default()
    }
}

/// Run `f`; in JSON mode keep the documents it would have printed.
fn captured<R>(json_output: bool, f: impl FnOnce() -> R) -> (R, Vec<String>) {
    if json_output {
        crate::core::json::capture(f)
    } else {
        (f(), Vec::new())
    }
}

fn heading(json_output: bool, name: &str) {
    if !json_output && !ui::is_quiet() {
        println!();
        println!("{}", format!("-- {name} --").bold());
    }
}

/// A part's own document, minus its `run` header, under `sections`.
fn add_section(
    sections: &mut Map<String, Value>,
    json_output: bool,
    name: &str,
    part: &Part,
    docs: Vec<String>,
) {
    if !json_output {
        return;
    }
    let report = docs
        .first()
        .and_then(|d| serde_json::from_str::<Value>(d).ok())
        .map(|mut v| {
            if let Value::Object(o) = &mut v {
                o.remove("run");
            }
            v
        });
    sections.insert(name.to_string(), section(part, report));
}

/// A skipped part still has its place in the document, with the reason.
fn note_skip(json_output: bool, name: &str, part: &Part, sections: &mut Map<String, Value>) {
    let Part::Skipped(why) = part else { return };
    if json_output {
        sections.insert(name.to_string(), section(part, None));
    } else {
        heading(false, name);
        println!("  {}", format!("skipped: {why}").dimmed());
    }
}

fn section(part: &Part, report: Option<Value>) -> Value {
    let mut o = Map::new();
    o.insert("status".into(), json!(part.status()));
    if let Part::Skipped(why) = part {
        o.insert("note".into(), json!(why));
    }
    if let Some(r) = report {
        o.insert("report".into(), r);
    }
    Value::Object(o)
}

fn report_value(report: &super::types::DiagnosticReport) -> Option<Value> {
    serde_json::to_value(report).ok()
}

/// `Result<Verdict>` cannot be cloned (`anyhow::Error`); the status is all the
/// section needs.
fn verdict_copy(r: &Result<Verdict>) -> Result<Verdict> {
    match r {
        Ok(v) => Ok(*v),
        Err(e) => Err(anyhow::anyhow!("{e:#}")),
    }
}
