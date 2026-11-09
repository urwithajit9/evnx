//! `evnx remove --check NAME…` (N14-11, plan §9 phase 2): is the name gone from
//! every place `remove` would touch?
//!
//! Read-only, for CI after a removal lands: exit `0` when no named variable is in
//! any environment file, the template or the contract, `1` when one still is
//! (and where), `2` when it could not look (no verdict).
//!
//! Always every place, never the narrow default of a real run: the question is
//! "is it gone", and an answer that depends on which flags a script remembered
//! is not one. It reads files `remove` would refuse to edit (read-only ones), as
//! it writes nothing. It refuses to answer, rather than guess, about a file it
//! cannot read whole: a key could hide behind a line the parser rejects.
//!
//! Names and line numbers only, never a value.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{check_arguments, contract, edit, in_dir, Kind, Options};
use crate::core::config::Template;
use crate::core::verdict::Verdict;
use crate::core::{env_name, glob::glob_match};
use crate::utils::path::label;
use crate::utils::ui;

/// A named variable found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub file: String,
    pub kind: &'static str,
    /// Each line it is assigned on. Empty for the contract.
    pub lines: Vec<usize>,
}

/// Run `remove --check`.
pub fn run(opts: Options) -> Result<Verdict> {
    if opts.json {
        ui::set_quiet(true);
    }
    let project = Path::new(&opts.path);
    if !project.is_dir() {
        bail!("{} is not a folder (--path)", label(&opts.path));
    }
    check_arguments(project, &opts.names)?;
    let loaded = crate::core::config::load(project)?;

    let mut found: Vec<Found> = Vec::new();
    let mut looked_in: Vec<String> = Vec::new();
    let wanted = |k: &str| opts.names.iter().any(|pat| glob_match(pat, k));

    let mut env_files: Vec<PathBuf> = Vec::new();
    if project.join(env_name::DEFAULT_FILE).is_file() {
        env_files.push(in_dir(project, env_name::DEFAULT_FILE));
    }
    for n in env_name::available(project) {
        env_files.push(in_dir(project, &format!(".env.{n}")));
    }
    let template = Template::of(project, &loaded.config);
    let template_path = in_dir(project, &template.name);

    for (kind, path) in env_files.iter().map(|p| (Kind::Env, p)).chain(
        template_path
            .is_file()
            .then_some((Kind::Template, &template_path)),
    ) {
        let shown = label(&path.to_string_lossy());
        looked_in.push(shown.clone());
        for (name, line) in entries(path, &shown)? {
            if !wanted(&name) {
                continue;
            }
            match found.iter_mut().find(|f| f.name == name && f.file == shown) {
                Some(f) => f.lines.push(line),
                None => found.push(Found {
                    name,
                    file: shown.clone(),
                    kind: if kind == Kind::Env { "env" } else { "template" },
                    lines: vec![line],
                }),
            }
        }
    }
    if let Some(src) = &loaded.source {
        let shown = ui::short_path(&std::path::absolute(src).unwrap_or_else(|_| src.clone()));
        looked_in.push(format!("{shown} [vars]"));
        let text = read_text(src, &shown)?;
        for name in contract::declared(&text, &shown)? {
            if wanted(&name) {
                found.push(Found {
                    name,
                    file: shown.clone(),
                    kind: "contract",
                    lines: Vec::new(),
                });
            }
        }
    }
    found.sort_by(|a, b| (&a.name, &a.file).cmp(&(&b.name, &b.file)));

    let clean = found.is_empty();
    if opts.json {
        print_json(&opts, &looked_in, &found);
    } else if clean {
        ui::success(format!(
            "{} gone from {}",
            opts.names.join(", "),
            if looked_in.is_empty() {
                "every place checked (there are no files to check)".to_string()
            } else {
                looked_in.join(", ")
            }
        ));
    } else {
        println!("  still present:");
        for f in &found {
            let lines = match f.lines.as_slice() {
                [] => String::new(),
                [l] => format!(" (line {l})"),
                ls => format!(
                    " (lines {})",
                    ls.iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let place = if f.kind == "contract" {
                format!("{} [vars.{}]", f.file, f.name)
            } else {
                f.file.clone()
            };
            println!("    {:<20} {place}{lines}", f.name);
        }
        let names: Vec<&str> = {
            let mut n: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
            n.dedup();
            n
        };
        eprintln!(
            "✗ Not removed everywhere. Clear it with: evnx remove {} --all",
            names.join(" ")
        );
    }
    Ok(if clean {
        Verdict::Clean
    } else {
        Verdict::Findings
    })
}

/// Every `(name, line)` an env file or the template assigns. Refuses a file it
/// cannot read whole, since an answer about it would be a guess.
pub(super) fn entries(path: &Path, shown: &str) -> Result<Vec<(String, usize)>> {
    let text = read_text(path, shown)?;
    let doc = edit::document(&text);
    if let Some(p) = doc.problems.first() {
        bail!(
            "{shown} has a line evnx cannot read, so a name could be hidden behind it and \
             nothing was concluded:\n  {}\n  Fix it first; `evnx validate` lists every problem.",
            p.describe()
        );
    }
    Ok(doc
        .entries
        .iter()
        .map(|e| (e.key.clone(), e.span.line))
        .collect())
}

fn read_text(path: &Path, shown: &str) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {shown}"))?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{shown} is not UTF-8 text"))
}

/// `--json`: names, files and lines, never a value.
fn print_json(opts: &Options, looked_in: &[String], found: &[Found]) {
    let doc = serde_json::json!({
        "schema_version": 1,
        "check": true,
        "clean": found.is_empty(),
        "names": opts.names,
        "checked": looked_in,
        "present": found.iter().map(|f| {
            let mut o = serde_json::json!({ "name": f.name, "file": f.file, "kind": f.kind });
            if f.kind == "contract" {
                o["table"] = format!("vars.{}", f.name).into();
            } else {
                o["lines"] = f.lines.clone().into();
            }
            o
        }).collect::<Vec<_>>(),
    });
    println!("{doc:#}");
}
