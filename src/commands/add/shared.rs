//! Shared utilities for add subcommands.
//!
//! # How every subcommand writes (AD-S, 0.10.2)
//!
//! [`plan`] reads `.env.example` and `.env` and sorts each requested name, per
//! file, into *new*, *already present* or *in conflict*; [`apply`] then checks
//! that every target can be written, appends to each file only the names it
//! lacks, and creates `.env` from the template when there is none. `service`,
//! `framework`, `blueprint`, `custom` and `init --from-source` all go through
//! these two, so they skip and report the same way.
//!
//! ⚠️ Until 0.10.2 every subcommand appended every name, every time. A second
//! `add service redis` wrote `REDIS_URL` twice into both files and said "Added 5
//! variables" (I-016), and because dotenv takes the *last* assignment, the
//! placeholder appended to `.env` silently replaced the user's real value
//! (N-11). Only `blueprint` skipped anything, and only names whose value in
//! `.env.example` differed.

use anyhow::{Context, Result};
use colored::*;
use indexmap::IndexMap;
use std::fs;
use std::path::Path;

use crate::schema::models::VarCollection;
use crate::utils::ui;
use crate::utils::ui::glyph;

/// What a handler did, so `run` can tell the truth afterwards.
///
/// ⚠️ Every handler used to return `Ok(())` whether it wrote or the user said
/// no, and `run` printed "Next steps: Edit .env…" after "Aborted." (N-12, the
/// same defect init fixed as I-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The variables were appended.
    Written,
    /// Every name was already present; nothing needed writing.
    Unchanged,
    /// Nothing was written: the user answered no, pressed Esc, or (in
    /// `custom`) gave no variable.
    Aborted,
}

/// How a run talks: whether it may ask, whether it writes, what it prints.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mode {
    /// `--yes`: never ask for a confirmation.
    pub yes: bool,
    /// `--dry-run`: say what would happen, write nothing (I-026).
    pub dry_run: bool,
    /// `--json`: one JSON document on stdout, nothing else there (I-026).
    pub json: bool,
    pub verbose: bool,
}

impl Mode {
    /// Whether this run may stop and ask: no `--yes`, `--dry-run` or `--json`.
    pub fn interactive(&self) -> bool {
        !self.yes && !self.dry_run && !self.json
    }
}

/// Print the one line every cancel path shares, and say so in the type.
pub fn aborted() -> Outcome {
    println!("{}", "Aborted. Nothing was written.".yellow());
    Outcome::Aborted
}

/// The one line that says what an id is when it is added through a
/// subcommand named for another group: `vercel is an infra component; adding
/// it.` (I-018). One id space, as `init --with` has: the id is not refused.
pub fn kind_hint(c: &crate::schema::component::Component) -> String {
    use crate::schema::component::Kind;
    let what = match c.kind {
        Kind::Framework => format!("a {} framework", c.group),
        Kind::Service => "a service".to_string(),
        Kind::Infrastructure => "an infra".to_string(),
    };
    let what = if c.kind == Kind::Framework {
        what
    } else {
        format!("{what} component")
    };
    format!("{} is {what}; adding it.", c.id)
}

/// `1 variable`, `2 variables`: a count with its noun (I-022).
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A name `.env.example` already has, with a different value than the one
/// being added. It is kept as it is and reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub var_name: String,
    pub existing_value: String,
    pub new_value: String,
}

/// What [`apply`] will do, worked out before anything is asked or written.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Everything that was asked for (for the spec write-back).
    pub requested: VarCollection,
    /// Names `.env.example` does not have yet.
    pub to_example: VarCollection,
    /// Names an existing `.env` does not have yet.
    pub to_env: VarCollection,
    /// Names `.env.example` already has with the same value.
    pub present: Vec<String>,
    /// Names `.env.example` already has with another value. Never written.
    pub conflicts: Vec<Conflict>,
    /// Whether `.env` existed when the plan was made.
    pub env_exists: bool,
}

impl Plan {
    /// Neither file needs anything.
    pub fn is_empty(&self) -> bool {
        self.to_example.vars.is_empty() && self.to_env.vars.is_empty()
    }

    /// Names `.env` gets that `.env.example` already had: the gap between the
    /// two files that this run closes.
    pub fn env_only(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .to_env
            .vars
            .keys()
            .filter(|n| !self.to_example.vars.contains_key(*n))
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names
    }
}

/// Sort `requested` against what `.env.example` and `.env` in `output_path`
/// already hold.
///
/// Per file (DQ-A2): a name missing from one file is added to that file only.
/// A name `.env.example` has with the *same* value is skipped; with another
/// value it is a conflict: skipped, kept, and listed (DQ-A3), and not added to
/// `.env` either, so the two files do not disagree about it.
///
/// ⚠️ Values are compared in `.env.example` only. `.env` holds real values,
/// which differ from the catalogue's placeholders by design; a name it has is
/// simply skipped, and its value is never read into a message.
pub fn plan(output_path: &Path, requested: &VarCollection) -> Result<Plan> {
    let example = read_assignments(&output_path.join(".env.example"))?;
    let env_path = output_path.join(".env");
    let env = read_assignments(&env_path)?;

    let mut plan = Plan {
        requested: requested.clone(),
        env_exists: env_path.exists(),
        ..Plan::default()
    };

    let mut names: Vec<&String> = requested.vars.keys().collect();
    names.sort_unstable();
    for name in names {
        let meta = &requested.vars[name];
        match example.as_ref().and_then(|m| m.get(name)) {
            None => {
                plan.to_example.vars.insert(name.clone(), meta.clone());
            }
            Some(existing) if *existing == meta.example_value => {
                plan.present.push(name.clone());
            }
            Some(existing) => {
                plan.conflicts.push(Conflict {
                    var_name: name.clone(),
                    existing_value: existing.clone(),
                    new_value: meta.example_value.clone(),
                });
                continue; // not into .env either
            }
        }
        if let Some(env) = &env {
            if !env.contains_key(name) {
                plan.to_env.vars.insert(name.clone(), meta.clone());
            }
        }
    }
    Ok(plan)
}

/// `KEY → value` for every assignment in `path`, or `None` if it does not
/// exist.
///
/// Through the shared parser with expansion off: a template may reference
/// variables it does not define, and nothing here should evaluate them. A file
/// the parser refuses (a typo halfway down a hand-edited `.env`) is read line by
/// line instead, so a syntax error elsewhere does not stop `add`, as it never
/// did.
fn read_assignments(path: &Path) -> Result<Option<IndexMap<String, String>>> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let parser = crate::core::parser::Parser::new(crate::core::parser::ParserConfig {
        allow_expansion: false,
        ..Default::default()
    });
    if let Ok(map) = parser.parse_content(&text) {
        return Ok(Some(map));
    }
    let mut map = IndexMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            map.insert(key.trim().to_string(), value.to_string());
        }
    }
    Ok(Some(map))
}

/// Carry out `plan`: append to each file the names it lacks, rendered by
/// `render`, then create `.env` from the template if there was none.
///
/// `create_env`: `None` leaves a missing `.env` missing (init handles its own);
/// `Some(ask)` creates it as `init` does (DQ-A4): `.gitignore` first, then a
/// byte copy of the template, asking first when `ask`.
///
/// ⚠️ Writability is checked for every file before the first byte is written.
/// A read-only `.env` used to fail the run *after* `.env.example` had been
/// appended, leaving the two files disagreeing and a retry duplicating the
/// template's names (N-14). Full atomicity is C-04's `safe_write` (0.11.0).
/// What [`apply`] did beyond the plan.
#[derive(Debug, Clone, Copy, Default)]
pub struct Applied {
    /// `.env` was created from the template (DQ-A4).
    pub env_created: bool,
    /// `.env` was written and git would commit it.
    pub env_committable: bool,
}

pub fn apply(
    output_path: &Path,
    plan: &Plan,
    render: &dyn Fn(&VarCollection) -> Result<String>,
    verbose: bool,
    create_env: Option<bool>,
    announce: bool,
) -> Result<Applied> {
    let mut applied = Applied::default();
    let example_path = output_path.join(".env.example");
    let env_path = output_path.join(".env");
    let write_example = !plan.to_example.vars.is_empty();
    let write_env = plan.env_exists && !plan.to_env.vars.is_empty();

    // 1. Every target writable, before anything is written.
    if write_example {
        check_writable(&example_path, output_path)?;
    }
    if write_env {
        check_writable(&env_path, output_path)?;
    }

    // 2. The template.
    if write_example {
        let addition = render(&plan.to_example)?;
        if example_path.exists() {
            let existing = read(&example_path)?;
            let tail = in_line_endings_of(&existing, format!("\n{addition}"));
            write_shared(&example_path, format!("{}{tail}", existing.trim_end()))?;
        } else {
            write_shared(&example_path, format!("{}\n", addition.trim()))?;
        }
        if verbose {
            println!(
                "{} Appended to {}",
                "[DEBUG]".dimmed(),
                example_path.display()
            );
        }
    }

    // 3. The local file.
    if write_env {
        let addition = render(&plan.to_env)?;
        let existing = read(&env_path)?;
        let body = addition.trim();
        let updated = if existing.trim().is_empty() {
            format!("{body}\n")
        } else {
            let tail = in_line_endings_of(&existing, format!("\n\n{body}\n"));
            format!("{}{tail}", existing.trim_end())
        };
        write_private(&env_path, updated)?;
        applied.env_committable = env_is_committable(output_path);
        if announce && applied.env_committable {
            warn_env_is_committable();
        }
        if verbose {
            // ⚠️ Said "with TODO placeholders" until 0.10.2, two releases after
            // `add` stopped writing them (I-024).
            println!("{} Appended to {}", "[DEBUG]".dimmed(), env_path.display());
        }
    } else if !plan.env_exists {
        if let Some(ask) = create_env {
            applied.env_created =
                crate::commands::init::shared::env_from_template(output_path, ask)?;
        }
    }

    // 4. The contract, when the project keeps one.
    match crate::commands::spec::writeback::record(output_path, &plan.requested) {
        Ok(outcome) => crate::commands::spec::writeback::report(&outcome, verbose),
        Err(e) => eprintln!("  could not update [vars]: {e:#}"),
    }

    Ok(applied)
}

/// Fail now, naming the file, if `path` cannot be written later.
fn check_writable(path: &Path, dir: &Path) -> Result<()> {
    if path.exists() {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map(drop)
            .with_context(|| format!("Failed to write {}", path.display()))
    } else {
        fs::metadata(dir)
            .map(drop)
            .with_context(|| format!("Failed to create {}", path.display()))
    }
}

// ⚠️ Both name the file. A bare `?` printed only `Permission denied (os error
// 13)`, with nothing to say which file (N-14b).
fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))
}

/// `.env.example`: shared, a new file 0644 (core::safe_write).
fn write_shared(path: &Path, content: String) -> Result<()> {
    let w = crate::core::safe_write::write_shared(path, content.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    crate::core::safe_write::report(&w);
    Ok(())
}

/// `.env`: holds real values, a new file 0600; an existing one keeps its mode
/// and is reported when others can read it (G9, DEC-G6).
fn write_private(path: &Path, content: String) -> Result<()> {
    let w = crate::core::safe_write::write_private(path, content.as_bytes())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    crate::core::safe_write::report(&w);
    Ok(())
}

/// How one subcommand asks and talks, for [`confirm_and_apply`].
pub struct Flow<'a> {
    /// What is counted: `variable`, or `custom variable`.
    pub noun: &'a str,
    /// What finishes the sentence: `for PostgreSQL`, `from blueprint 'T3'`.
    pub what: String,
    /// The same command with `--yes`, for the no-terminal message (I-020);
    /// `None` when the caller has already made sure there is a terminal.
    pub terminal_example: Option<String>,
    /// Printed above the preview (a header, `Preview:`).
    pub heading: &'a dyn Fn(),
    /// The preview of what will be written.
    pub preview: &'a dyn Fn(&VarCollection) -> String,
    /// The question, given the count and the file it goes to.
    pub question: &'a dyn Fn(usize, &str) -> String,
    /// The text appended for a set of variables.
    pub render: &'a dyn Fn(&VarCollection) -> Result<String>,
}

/// Plan, preview, ask (unless `--yes`), apply and report: the whole of an
/// `add` subcommand after it knows its variables. `--dry-run` stops after the
/// plan; `--json` replaces every line on stdout with one document.
pub fn confirm_and_apply(
    output_path: &Path,
    requested: &VarCollection,
    flow: &Flow,
    mode: &Mode,
) -> Result<Outcome> {
    let plan = plan(output_path, requested)?;

    if mode.dry_run {
        if mode.json {
            print_json(&plan, true, &Applied::default());
        } else {
            print_dry_run(&plan, flow);
        }
        return Ok(Outcome::Unchanged);
    }
    if plan.is_empty() {
        if mode.json {
            print_json(&plan, false, &Applied::default());
        } else {
            report(&plan, flow, true);
        }
        return Ok(Outcome::Unchanged);
    }

    // `--yes` never previews (DQ-A6): the result line says what happened.
    if mode.interactive() {
        if let Some(example) = &flow.terminal_example {
            // Before the preview: without a terminal the question cannot be
            // asked, and dialoguer's own error says nothing about `--yes`
            // (I-020).
            crate::utils::prompt::require_terminal(example)?;
        }
        (flow.heading)();
        let (subset, file) = if plan.to_example.vars.is_empty() {
            (&plan.to_env, ".env")
        } else {
            (&plan.to_example, ".env.example")
        };
        println!("{}", (flow.preview)(subset).dimmed());
        if !plan.present.is_empty() {
            ui::info(format!(
                "Already in .env.example, skipped: {}",
                plan.present.join(", ")
            ));
        }
        print_conflicts(&plan);
        let confirm = crate::utils::prompt::yes_no()
            .with_prompt((flow.question)(subset.vars.len(), file))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
        }
    }

    let applied = apply(
        output_path,
        &plan,
        flow.render,
        mode.verbose && !mode.json,
        Some(mode.interactive()),
        !mode.json,
    )?;
    if mode.json {
        print_json(&plan, false, &applied);
    } else {
        // Asked: the conflicts were shown with the question.
        report(&plan, flow, !mode.interactive());
    }
    Ok(Outcome::Written)
}

/// `--dry-run`: what [`apply`] would do, file by file. Printed whatever `-q`
/// says: it is the output that was asked for.
fn print_dry_run(plan: &Plan, flow: &Flow) {
    let names = |v: &VarCollection| {
        let mut n: Vec<&str> = v.vars.keys().map(String::as_str).collect();
        n.sort_unstable();
        n.join(", ")
    };
    let what = flow.what.trim();
    println!(
        "Dry run{}: nothing was written.",
        if what.is_empty() {
            String::new()
        } else {
            format!(" ({what})")
        }
    );
    if plan.to_example.vars.is_empty() {
        println!("  .env.example: nothing to add");
    } else {
        println!(
            "  .env.example: would add {}: {}",
            count(plan.to_example.vars.len(), "variable"),
            names(&plan.to_example)
        );
    }
    if !plan.present.is_empty() {
        println!("  already there, would skip: {}", plan.present.join(", "));
    }
    for c in &plan.conflicts {
        println!(
            "  different value, would keep: {} (existing: \"{}\", new: \"{}\")",
            c.var_name, c.existing_value, c.new_value
        );
    }
    if plan.env_exists {
        if !plan.to_env.vars.is_empty() {
            println!(
                "  .env: would add {}: {}",
                count(plan.to_env.vars.len(), "variable"),
                names(&plan.to_env)
            );
        }
    } else if !plan.is_empty() {
        println!("  .env: would be created from .env.example (with .gitignore first)");
    }
}

/// `--json`: the result as one document. Names only, and conflicts with the
/// `.env.example` values they compare; nothing from `.env`, ever.
///
/// Stable fields (I-026): `dry_run`, `written`, `added` (to `.env.example`),
/// `skipped` (already there, same value), `conflicts` (`name`, `existing`,
/// `new`; kept), `env_added`, `env_created`, `env_committable`.
fn print_json(plan: &Plan, dry_run: bool, applied: &Applied) {
    fn sorted(v: &VarCollection) -> Vec<&str> {
        let mut n: Vec<&str> = v.vars.keys().map(String::as_str).collect();
        n.sort_unstable();
        n
    }
    let doc = serde_json::json!({
        "dry_run": dry_run,
        "written": !dry_run && !plan.is_empty(),
        "added": sorted(&plan.to_example),
        "skipped": plan.present,
        "conflicts": plan.conflicts.iter().map(|c| serde_json::json!({
            "name": c.var_name,
            "existing": c.existing_value,
            "new": c.new_value,
        })).collect::<Vec<_>>(),
        "env_added": sorted(&plan.to_env),
        "env_created": applied.env_created,
        "env_committable": applied.env_committable,
    });
    println!("{doc:#}");
}

/// Print what [`apply`] did: the conflicts (when `show_conflicts`), then one
/// result line, and a second when `.env` caught up with `.env.example`.
///
/// Conflicts are a finding and print under `-q`; the result lines do not.
pub fn report(plan: &Plan, flow: &Flow, show_conflicts: bool) {
    if show_conflicts {
        print_conflicts(plan);
    }
    let what = &flow.what;
    let added = plan.to_example.vars.len();
    let skipped = plan.present.len() + plan.conflicts.len();
    let sentence = |n: usize| {
        let mut s = format!("{} {}", count(n, flow.noun), what);
        s.truncate(s.trim_end().len());
        s
    };
    if added > 0 {
        let tail = if skipped > 0 {
            format!(", skipped {skipped} already present")
        } else {
            String::new()
        };
        ui::success(format!("Added {}{tail}", sentence(added)));
    } else if plan.to_env.vars.is_empty() {
        ui::info(format!(
            "Nothing to add: {} already in .env.example",
            sentence(plan.requested.vars.len())
        ));
    }
    let env_only = plan.env_only();
    if plan.env_exists && !env_only.is_empty() {
        ui::info(format!(
            "Added {} to .env that .env.example already had: {}",
            count(env_only.len(), "variable"),
            env_only.join(", ")
        ));
    }
}

/// The conflicts in `plan`, as a warning and one line each. Values shown are
/// `.env.example`'s, never `.env`'s.
pub fn print_conflicts(plan: &Plan) {
    if plan.conflicts.is_empty() {
        return;
    }
    ui::warning(format!(
        "{} with a different value in .env.example: kept as it is",
        count(plan.conflicts.len(), "variable")
    ));
    for c in &plan.conflicts {
        ui::warning_detail(format!(
            "  • {} (existing: \"{}\", new: \"{}\")",
            c.var_name.bold(),
            c.existing_value.dimmed(),
            c.new_value.dimmed()
        ));
    }
}

/// `tail`, in the line endings `existing` already uses.
///
/// ⚠️ `add` wrote `\n` into every file, so a CRLF file (one saved on Windows)
/// came back with mixed line endings: the user's lines in CRLF, the added block
/// in LF (N-16). The user's own lines are never rewritten; only what is
/// appended follows them. A file's first line ending decides.
fn in_line_endings_of(existing: &str, tail: String) -> String {
    let crlf = existing
        .find('\n')
        .is_some_and(|i| i > 0 && existing.as_bytes()[i - 1] == b'\r');
    if crlf {
        tail.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        tail
    }
}

pub fn format_var_line(
    name: &str,
    example_value: &str,
    description: Option<&str>,
    required: bool,
    _category: Option<&str>,
) -> String {
    let mut lines = Vec::new();

    match (description, required) {
        (Some(desc), true) => lines.push(format!("# {} (required)", desc)),
        (Some(desc), false) => lines.push(format!("# {}", desc)),
        (None, true) => lines.push("# (required)".to_string()),
        (None, false) => {}
    }

    lines.push(crate::core::envfile::line(name, example_value));

    lines.join("\n")
}

fn env_is_committable(project_root: &Path) -> bool {
    !crate::core::gitignore::env_file_is_protected(project_root, ".env")
}

fn warn_env_is_committable() {
    ui::warning(".env is not covered by .gitignore — it can be committed");
    ui::warning_detail(format!(
        "     {}  {}",
        glyph::ARROW.dimmed(),
        "evnx doctor --fix".cyan()
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::models::{VarMetadata, VarSource};
    use tempfile::TempDir;

    fn vars(pairs: &[(&str, &str)]) -> VarCollection {
        let mut v = VarCollection::default();
        for (k, val) in pairs {
            v.vars.insert(
                k.to_string(),
                VarMetadata {
                    example_value: val.to_string(),
                    description: None,
                    category: None,
                    required: false,
                    source: VarSource::Service("test".to_string()),
                },
            );
        }
        v
    }

    fn names(v: &VarCollection) -> Vec<&str> {
        let mut n: Vec<&str> = v.vars.keys().map(String::as_str).collect();
        n.sort_unstable();
        n
    }

    /// I-016, N-11, DQ-A2, DQ-A3: every case of the per-file rule at once.
    #[test]
    fn plan_sorts_each_name_per_file() {
        let d = TempDir::new().unwrap();
        fs::write(
            d.path().join(".env.example"),
            "SAME=x\nDIFF=mine\nIN_EXAMPLE_ONLY=y\n",
        )
        .unwrap();
        fs::write(d.path().join(".env"), "SAME=real\nIN_ENV_ONLY=real\n").unwrap();
        let p = plan(
            d.path(),
            &vars(&[
                ("SAME", "x"),
                ("DIFF", "theirs"),
                ("IN_EXAMPLE_ONLY", "y"),
                ("IN_ENV_ONLY", "z"),
                ("NEW", "n"),
            ]),
        )
        .unwrap();
        assert_eq!(names(&p.to_example), ["IN_ENV_ONLY", "NEW"]);
        assert_eq!(names(&p.to_env), ["IN_EXAMPLE_ONLY", "NEW"]);
        assert_eq!(p.present, ["IN_EXAMPLE_ONLY", "SAME"]);
        assert_eq!(
            p.conflicts,
            [Conflict {
                var_name: "DIFF".into(),
                existing_value: "mine".into(),
                new_value: "theirs".into(),
            }]
        );
        assert_eq!(p.env_only(), ["IN_EXAMPLE_ONLY"]);
    }

    #[test]
    fn plan_without_files_adds_everything_to_the_template() {
        let d = TempDir::new().unwrap();
        let p = plan(d.path(), &vars(&[("A", "1"), ("B", "2")])).unwrap();
        assert_eq!(names(&p.to_example), ["A", "B"]);
        assert!(p.to_env.vars.is_empty());
        assert!(!p.env_exists);
    }

    /// Quotes and `export` are the parser's business: `A="x"` has the value `x`.
    #[test]
    fn plan_compares_parsed_values_not_raw_text() {
        let d = TempDir::new().unwrap();
        fs::write(
            d.path().join(".env.example"),
            "export A=\"x\"\nB='${NOPE}'\n",
        )
        .unwrap();
        let p = plan(d.path(), &vars(&[("A", "x"), ("B", "${NOPE}")])).unwrap();
        assert_eq!(p.present, ["A", "B"]);
        assert!(p.conflicts.is_empty());
    }

    /// A file the parser refuses is still read, line by line.
    #[test]
    fn plan_reads_a_file_the_parser_refuses() {
        let d = TempDir::new().unwrap();
        fs::write(d.path().join(".env.example"), "A=1\nB=\"never closed\n").unwrap();
        let p = plan(d.path(), &vars(&[("A", "1")])).unwrap();
        assert_eq!(p.present, ["A"]);
    }

    /// N-16: what is appended follows the file's line endings; the file's
    /// own lines are not touched.
    #[test]
    fn appended_lines_follow_the_files_line_endings() {
        assert_eq!(
            in_line_endings_of("A=1\r\nB=2\r\n", "\nC=3\n".to_string()),
            "\r\nC=3\r\n"
        );
        assert_eq!(
            in_line_endings_of("A=1\nB=2\n", "\nC=3\n".to_string()),
            "\nC=3\n"
        );
        // One line with no newline at all: nothing to follow, so LF.
        assert_eq!(in_line_endings_of("A=1", "\nC=3\n".to_string()), "\nC=3\n");
    }

    #[test]
    fn test_format_var_line_with_metadata() {
        let line = format_var_line(
            "API_KEY",
            "sk_test_xxx",
            Some("API key for testing"),
            true,
            Some("Auth"),
        );

        assert!(line.contains("# API key for testing"));
        assert!(line.contains("API_KEY=sk_test_xxx"));
        assert!(line.contains("(required)"));
    }
}
