//! `evnx add import <FILE|->`: many variables from one file into `.env`.
//!
//! The readers live in [`crate::core::import`]; this is the part that talks:
//! it picks the format, shows what each source key becomes, and hands the
//! names to the same plan / ask / write path every other `add` uses
//! ([`super::shared::confirm_and_apply`]). So the rules for a name `.env`
//! already has (kept, and reported), the placeholders in `.env.example`, the
//! `[vars]` contract, `--env-name`, `--dry-run`, `--json` and `--yes` are
//! `add`'s own, not a second copy.
//!
//! * Two sources that make one name, an array, a binary value or a name that
//!   cannot be valid: nothing is written, exit 2, every one listed.
//! * `--dry-run` prints the mapping `source key → NAME → new | kept | refused`.
//!   `--json` carries it as `mapping[]`. Neither has a value in it.
//! * A source file that git tracks and that holds secret-looking values gets a
//!   warning: untrack it and rotate them.

use std::io::Read as _;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use super::shared::{Flow, Mode, Outcome, confirm_and_apply};
use crate::core::import::{self, Format, Item, Plan};
use crate::schema::models::{VarCollection, VarMetadata, VarSource};
use crate::utils::ui;

/// The most text read from a file: far more than any variables file holds.
const CAP: u64 = 8 * 1024 * 1024;

/// What `evnx add import` was given.
pub struct Args {
    pub file: String,
    pub from: Option<Format>,
    pub section: Option<String>,
    pub only: Vec<String>,
    pub exclude: Vec<String>,
}

/// One line of the mapping.
#[derive(Serialize)]
struct Row<'a> {
    source: &'a str,
    name: &'a str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// The format the file's name or `--from` says.
fn format_of(args: &Args) -> Result<Format> {
    if let Some(f) = args.from {
        return Ok(f);
    }
    if args.file == "-" {
        return Err(crate::core::verdict::usage(
            "standard input has no file name to tell the format from: add --from \
             (env, properties, ini, json, yaml or csv).",
        ));
    }
    Format::detect(&args.file).ok_or_else(|| {
        crate::core::verdict::usage(format!(
            "cannot tell the format of {} from its name: add --from \
             (env, properties, ini, json, yaml or csv).",
            crate::utils::path::label(&args.file)
        ))
    })
}

/// The file's text, or standard input's.
fn read_text(file: &str) -> Result<String> {
    if file == "-" {
        return crate::utils::term::read_piped("evnx add import -");
    }
    let shown = crate::utils::path::label(file);
    let handle = std::fs::File::open(file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            crate::core::verdict::usage(format!("{shown} does not exist"))
        }
        _ => crate::core::verdict::usage(format!("cannot read {shown}: {e}")),
    })?;
    let mut bytes = Vec::new();
    handle
        .take(CAP + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {shown}"))?;
    if bytes.len() as u64 > CAP {
        return Err(crate::core::verdict::usage(format!(
            "{shown} is over 8 MiB: that is not a file of variables"
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| crate::core::verdict::usage(format!("{shown} is not UTF-8 text")))
}

pub fn handle(output_path: &Path, args: Args, mode: &Mode) -> Result<Outcome> {
    let format = format_of(&args)?;
    let text = read_text(&args.file)?;
    let found = import::read(format, &text, args.section.as_deref())
        .map_err(|e| crate::core::verdict::usage(format!("{e:#}")))?;
    drop(text);
    let plan = import::plan(found, format, &args.only, &args.exclude);

    // What `.env` has now, to say new or kept.
    let env_file = super::shared::env_file_of(output_path, mode);
    let existing = super::shared::read_assignments(&output_path.join(&env_file))?;
    let has = |name: &str| existing.as_ref().is_some_and(|m| m.contains_key(name));

    let rows = rows(&plan, &has);
    if mode.dry_run && !mode.json || mode.interactive() {
        print_table(&rows, &plan);
    }
    if !plan.is_clean() {
        return Err(crate::core::verdict::usage(problems(&plan)));
    }
    for note in &plan.notes {
        ui::info(note);
    }
    if plan.items.is_empty() {
        let why = if plan.filtered_out > 0 {
            "--only and --exclude left nothing"
        } else {
            "the file has no variables"
        };
        if mode.json {
            // The same document an empty plan makes elsewhere.
        } else {
            println!("Nothing to import: {why}.");
            return Ok(Outcome::Unchanged);
        }
    }
    if let Some(w) = import::tracked_warning(&args.file, &plan.items) {
        ui::warning(w);
    }

    let from = if args.file == "-" {
        "standard input".to_string()
    } else {
        crate::utils::path::label(&args.file)
            .trim_start_matches("./")
            .to_string()
    };
    let mut collected = VarCollection::default();
    for i in &plan.items {
        collected.vars.insert(
            i.name.clone(),
            VarMetadata {
                example_value: placeholder_for(&i.name),
                description: None,
                category: None,
                required: false,
                source: VarSource::BlueprintOverride,
            },
        );
    }
    let values = import::values(&plan.items);
    let json = serde_json::json!({
        "format": format.name(),
        "mapping": rows,
    });
    let items: &[Item] = &plan.items;
    let render = |v: &VarCollection| -> Result<String> {
        Ok(import::render_env(
            items,
            &|n| v.vars.contains_key(n),
            &from,
        ))
    };
    let render_example = |v: &VarCollection| -> Result<String> {
        Ok(import::render_example(
            items,
            &|n| v.vars.contains_key(n),
            &from,
        ))
    };
    let preview = |v: &VarCollection| {
        format!(
            "  • {} to import from {from}",
            crate::utils::string::count(v.vars.len(), "variable")
        )
    };
    let example = format!(
        "evnx add import {}{} --yes",
        args.file,
        args.from
            .map(|f| format!(" --from {}", f.name()))
            .unwrap_or_default()
    );
    let flow = Flow {
        noun: "variable",
        what: String::new(),
        terminal_example: Some(example),
        heading: &ui::print_preview_header,
        preview: &preview,
        question: &|_, file| format!("Append these variables to {file}?"),
        render: &render,
        example_render: Some(&render_example),
        env_from_values: true,
        values: Some(&values),
        json_extra: Some(&json),
    };
    confirm_and_apply(output_path, &collected, &flow, mode)
}

fn placeholder_for(name: &str) -> String {
    crate::core::placeholder::generate_placeholder(
        name,
        None,
        &crate::core::placeholder::PlaceholderConfig::default(),
    )
}

/// Every source in the file's order, with what becomes of it.
fn rows<'a>(plan: &'a Plan, has: &dyn Fn(&str) -> bool) -> Vec<Row<'a>> {
    let collided = |name: &str| plan.collisions.iter().any(|c| c.name == name);
    let mut all: Vec<(usize, Row)> = Vec::new();
    for i in &plan.items {
        let (status, reason) = if collided(&i.name) {
            let others: Vec<&str> = plan
                .collisions
                .iter()
                .filter(|c| c.name == i.name)
                .flat_map(|c| c.sources.iter().map(String::as_str))
                .filter(|s| *s != i.source)
                .collect();
            (
                "refused",
                Some(format!("{} also makes this name", others.join(", "))),
            )
        } else if has(&i.name) {
            ("kept", Some("already in the env file".to_string()))
        } else {
            ("new", None)
        };
        all.push((
            i.index,
            Row {
                source: &i.source,
                name: &i.name,
                status,
                reason,
            },
        ));
    }
    for r in &plan.refused {
        all.push((
            r.index,
            Row {
                source: &r.source,
                name: &r.name,
                status: "refused",
                reason: Some(r.reason.clone()),
            },
        ));
    }
    all.sort_by_key(|(i, _)| *i);
    all.into_iter().map(|(_, r)| r).collect()
}

/// `source key → NAME → status`, with no value.
fn print_table(rows: &[Row], plan: &Plan) {
    if rows.is_empty() {
        return;
    }
    let w1 = rows
        .iter()
        .map(|r| r.source.chars().count())
        .max()
        .unwrap_or(0);
    let w2 = rows
        .iter()
        .map(|r| r.name.chars().count().max(1))
        .max()
        .unwrap_or(0)
        .max("NAME".len());
    println!("  {:<w1$}  →  {:<w2$}  →  status", "source key", "NAME");
    for r in rows {
        let name = if r.name.is_empty() { "-" } else { r.name };
        let status = match &r.reason {
            Some(why) => format!("{}: {why}", r.status),
            None => r.status.to_string(),
        };
        println!("  {:<w1$}  →  {:<w2$}  →  {status}", r.source, name);
    }
    if plan.filtered_out > 0 {
        println!("  ({} left out by --only / --exclude)", plan.filtered_out);
    }
}

/// The error for a file that cannot be imported whole.
fn problems(plan: &Plan) -> String {
    let mut out = String::from("nothing was imported:\n");
    for c in &plan.collisions {
        out.push_str(&format!(
            "  {} is made by {}\n",
            c.name,
            c.sources.join(" and ")
        ));
    }
    for r in &plan.refused {
        out.push_str(&format!("  {} {}\n", r.source, r.reason));
    }
    out.push_str(
        "\nNarrow the import with --only, --exclude or --section, or rename the keys in the file.",
    );
    out
}
