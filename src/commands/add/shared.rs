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

use crate::core::secret::Vars;
use anyhow::{Context, Result};
use colored::*;
use indexmap::IndexMap;
use std::fs;
use std::path::Path;

use super::report::Step;
use crate::core::output::{Mode as OutMode, Out};
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
    /// The variables were appended, and at least one went in without a value
    /// of its own (a placeholder to replace).
    Written,
    /// The variables were appended, and every name written carried a value
    /// the user gave: there is no placeholder to replace (`add custom`).
    WrittenWithValues,
    /// Every name was already present; nothing needed writing.
    Unchanged,
    /// Nothing was written: the user answered no, pressed Esc, or (in
    /// `custom`) gave no variable.
    Aborted,
}

/// How a run talks: whether it may ask, whether it writes, what it prints.
#[derive(Debug, Clone, Default)]
pub struct Mode {
    /// `--yes`: never ask for a confirmation.
    pub yes: bool,
    /// `--dry-run`: say what would happen, write nothing (I-026).
    pub dry_run: bool,
    /// `--json`: one JSON document on stdout, nothing else there (I-026).
    pub json: bool,
    pub verbose: bool,
    /// `--env-name`: write `.env.<name>` instead of `.env` (RM-17). `None`
    /// takes `[defaults] env_name`, then `.env`.
    pub env_name: Option<String>,
    /// `--example`: the template, from the project folder (FLD-4). `None`
    /// takes the rule in `core::resolve`.
    pub example: Option<String>,
    /// `--no-template`: the env file only (ADD-3).
    pub no_template: bool,
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
    /// Names the env file already has with a value other than the one given
    /// (`add custom`). Kept as they are, in neither file, and reported; the
    /// values are never read into a message.
    pub env_kept: Vec<String>,
    /// Whether `.env` existed when the plan was made.
    pub env_exists: bool,
    /// `.env` does not exist yet and will be created to hold the values
    /// (`add custom`, DEC-U8-1): a missing `.env` is planned as empty.
    pub create_env_file: bool,
    /// The project's template (DQ-28): `[defaults] example`, else the
    /// `.env.example`, `.env.sample` or `.env.template` present, else
    /// `.env.example`. It was always `.env.example` (I-295).
    pub template: crate::core::config::Template,
    /// The env file this run writes: `.env`, or `.env.<name>` from
    /// `--env-name` or `[defaults] env_name` (RM-17).
    pub env_file: String,
}

impl Plan {
    /// What goes into `[vars]`: the names asked for, less the ones kept as they
    /// were (nothing was written for those, so the contract must not declare them).
    pub fn recordable(&self) -> VarCollection {
        let mut vars = self.requested.clone();
        for name in &self.env_kept {
            vars.vars.remove(name);
        }
        vars
    }

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

/// The env file and template a run in `output_path` writes, by the one rule
/// (`core::resolve`, WP-5): `--env-name`, else `[defaults] env_name`, else
/// `.env`; the template `--example`, else the environment's own, else
/// `[defaults] example`, else the project's.
pub fn resolved(output_path: &Path, mode: &Mode) -> Result<crate::core::resolve::Resolved> {
    let config = crate::core::config::load(output_path)?.config;
    resolve_with(
        output_path,
        mode.env_name.as_deref(),
        mode.example.as_deref(),
        &config,
    )
}

fn resolve_with(
    output_path: &Path,
    env_name: Option<&str>,
    example: Option<&str>,
    config: &crate::core::config::Config,
) -> Result<crate::core::resolve::Resolved> {
    use crate::core::resolve::{Flags, Need, resolve};
    let flags = Flags {
        env: None,
        env_name,
        example,
    };
    resolve(output_path, flags, config, Need::MayCreate)
}

/// INIT-3, ADD-6: a new `--env-name` close to a known one
/// (`core::resolve::confirm_new_env_name`).
pub fn check_new_env_name(output_path: &Path, mode: &Mode) -> Result<()> {
    crate::core::resolve::confirm_new_env_name(
        output_path,
        mode.env_name.as_deref(),
        mode.yes,
        mode.dry_run,
        mode.json,
    )
}

/// The template a run in `output_path` writes to, for a message that names
/// it before there is a plan. A config that does not parse names the project
/// default here; the plan reports it.
pub fn template_of(output_path: &Path, mode: &Mode) -> crate::core::config::Template {
    match resolved(output_path, mode) {
        Ok(r) => crate::core::config::Template {
            configured: r.template_from != crate::core::resolve::TemplateFrom::Project,
            name: r.template,
        },
        Err(_) => crate::core::config::Template::default(),
    }
}

/// The env file a run in `output_path` writes, for a message before there
/// is a plan. A config or name that does not parse says `.env`; the plan
/// reports it.
pub fn env_file_of(output_path: &Path, mode: &Mode) -> String {
    resolved(output_path, mode)
        .map(|r| r.env)
        .unwrap_or_else(|_| crate::core::env_name::DEFAULT_FILE.to_string())
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
pub fn plan(output_path: &Path, requested: &VarCollection, env_name: Option<&str>) -> Result<Plan> {
    plan_with(output_path, requested, false, env_name, None)
}

/// [`plan`], and with `create_env_file` a missing `.env` is planned as an
/// empty one that [`apply`] creates (0600) — for values that must land
/// somewhere other than the template (I-245).
pub fn plan_with(
    output_path: &Path,
    requested: &VarCollection,
    create_env_file: bool,
    env_name: Option<&str>,
    example_flag: Option<&str>,
) -> Result<Plan> {
    let config = crate::core::config::load(output_path)?.config;
    let files = resolve_with(output_path, env_name, example_flag, &config)?;
    let template = crate::core::config::Template {
        configured: files.template_from != crate::core::resolve::TemplateFrom::Project,
        name: files.template,
    };
    let example = read_assignments(&template.path(output_path))?;
    let env_file = files.env;
    let env_path = output_path.join(&env_file);
    let mut env = read_assignments(&env_path)?;
    let create_env_file = create_env_file && env.is_none();
    if create_env_file {
        env = Some(Default::default());
    }

    let mut plan = Plan {
        requested: requested.clone(),
        env_exists: env_path.exists(),
        create_env_file,
        template,
        env_file,
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
                    existing_value: existing.expose().to_string(),
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

/// ADD-3, `--no-template`: the plan for the env file alone. Every requested
/// name the env file lacks goes to it, whatever the template says; nothing
/// goes to the template, and it is not created.
fn env_only(
    output_path: &Path,
    plan: &mut Plan,
    requested: &VarCollection,
    create_env_file: bool,
) -> Result<()> {
    let env = read_assignments(&output_path.join(&plan.env_file))?;
    if env.is_none() && !create_env_file {
        plan.to_example.vars.clear();
        plan.to_env.vars.clear();
        return Ok(());
    }
    let env = env.unwrap_or_default();
    plan.to_example.vars.clear();
    plan.conflicts.clear();
    plan.present.clear();
    plan.to_env.vars.clear();
    for (name, meta) in &requested.vars {
        if env.contains_key(name) {
            plan.present.push(name.clone());
        } else {
            plan.to_env.vars.insert(name.clone(), meta.clone());
        }
    }
    plan.present.sort();
    Ok(())
}

/// `KEY → value` for every assignment in `path`, or `None` if it does not
/// exist.
///
/// Through the shared parser with expansion off: a template may reference
/// variables it does not define, and nothing here should evaluate them. A file
/// the parser refuses (a typo halfway down a hand-edited `.env`) is read
/// lossily, so a syntax error elsewhere does not stop `add`, as it never did.
///
/// ⚠️ Lossily, not line by line: a line inside a multiline value is value
/// text. Read line by line, `KEY=x` there counted as `KEY` being set, so `add`
/// skipped a variable the file did not have.
pub(crate) fn read_assignments(path: &Path) -> Result<Option<Vars>> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let parser = crate::core::parser::Parser::new(crate::core::parser::ParserConfig {
        allow_expansion: false,
        ..Default::default()
    });
    Ok(Some(parser.parse_document(&text).vars()))
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
    example_render: Option<&Render<'_>>,
    verbose: bool,
    create_env: Option<bool>,
    announce: bool,
) -> Result<Applied> {
    let applied = apply_files(
        output_path,
        plan,
        render,
        example_render,
        verbose,
        create_env,
        announce,
    )?;
    record_contract(output_path, plan, verbose);
    Ok(applied)
}

/// [`apply`] without the `[vars]` write-back: for `init`, which keeps the
/// contract itself (IN-15) and would otherwise be told to run `spec init`
/// about a file it is about to write.
pub fn apply_files(
    output_path: &Path,
    plan: &Plan,
    render: &dyn Fn(&VarCollection) -> Result<String>,
    example_render: Option<&Render<'_>>,
    verbose: bool,
    create_env: Option<bool>,
    announce: bool,
) -> Result<Applied> {
    let mut applied = Applied::default();
    let example_path = plan.template.path(output_path);
    let env_path = output_path.join(&plan.env_file);
    let write_example = !plan.to_example.vars.is_empty();
    let write_env = (plan.env_exists || plan.create_env_file) && !plan.to_env.vars.is_empty();

    // 1. Every target writable, before anything is written.
    if write_example {
        check_writable(&example_path, output_path)?;
    }
    if write_env {
        check_writable(&env_path, output_path)?;
    }

    // 2. The template.
    if write_example {
        // The template's own text when the flow has one: `add custom` writes
        // placeholders there and values only to `.env` (I-245, DEC-U8-1).
        let addition = example_render.unwrap_or(render)(&plan.to_example)?;
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
        let existing = if env_path.exists() {
            read(&env_path)?
        } else {
            // Created now, 0600, by `write_private` below. `.gitignore` first,
            // as `env_from_template` does (DQ-A4): this `.env` holds the values
            // `add custom` was given, and was left committable (I-251).
            if let Some(line) = crate::commands::init::shared::update_gitignore(output_path)? {
                crate::utils::ui::say(line);
            }
            String::new()
        };
        append_block(&env_path, &existing, &addition)?;
        applied.env_committable = env_is_committable(output_path, &plan.env_file);
        if announce && applied.env_committable {
            // The shared wording (E6), then add's way out when no rule covers it.
            if crate::utils::protect::warn_if_exposed(output_path, &plan.env_file)
                == crate::core::gitignore::Exposure::Unignored
            {
                ui::warning_detail(format!(
                    "     {}  {}",
                    glyph::ARROW.dimmed(),
                    "evnx doctor --fix".cyan()
                ));
            }
        }
        if verbose {
            // ⚠️ Said "with TODO placeholders" until 0.10.2, two releases after
            // `add` stopped writing them (I-024).
            println!("{} Appended to {}", "[DEBUG]".dimmed(), env_path.display());
        }
    } else if !plan.env_exists {
        if let Some(ask) = create_env {
            applied.env_created = crate::commands::init::shared::env_from_template(
                output_path,
                &plan.template,
                &plan.env_file,
                ask,
            )?;
        }
    }

    Ok(applied)
}

/// The contract, when the project keeps one: new names into `[vars]`.
pub fn record_contract(output_path: &Path, plan: &Plan, verbose: bool) {
    match crate::commands::spec::writeback::record(output_path, &plan.recordable()) {
        Ok(outcome) => crate::commands::spec::writeback::report(&outcome, verbose),
        Err(e) => eprintln!("  could not update [vars]: {e:#}"),
    }
}

/// Append `addition` to the env file at `path`, whose text is `existing`: a
/// blank line, the block, the file's own line endings; 0600 when new, an
/// existing file's mode kept. Shared with `init`, which catches an existing
/// `.env` up with a new template (DQ-29).
pub(crate) fn append_block(path: &Path, existing: &str, addition: &str) -> Result<()> {
    // A new block starts with a blank line before its `# ── … ──` header; one
    // that continues the file's last section (`add custom` under an existing
    // "Custom Variables", ADD-3) follows it directly.
    let gap = if addition.starts_with('\n') || addition.starts_with("\r\n") {
        "\n\n"
    } else {
        "\n"
    };
    let body = addition.trim();
    let updated = if existing.trim().is_empty() {
        format!("{body}\n")
    } else {
        let tail = in_line_endings_of(existing, format!("{gap}{body}\n"));
        format!("{}{tail}", existing.trim_end())
    };
    write_private(path, updated)
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
pub(crate) fn read(path: &Path) -> Result<String> {
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

/// Text appended to a file for a set of variables.
pub type Render<'r> = dyn Fn(&VarCollection) -> Result<String> + 'r;

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
    /// The text appended to `.env.example`, when it differs from [`Flow::render`]
    /// (`add custom`: placeholders, DEC-U8-1). `None`: the same text.
    pub example_render: Option<&'a Render<'a>>,
    /// Create `.env` for the values when it does not exist (`add custom`).
    pub env_from_values: bool,
    /// The value given for each name, `None` when none was (a placeholder
    /// stands in): `add custom` only. With it, a name the env file already
    /// has with another value is kept and reported, not counted as added.
    pub values: Option<&'a IndexMap<String, Option<String>>>,
    /// Fields added to the `--json` document (`add import`'s `mapping`).
    /// Names and statuses only: nothing in it may be a value.
    pub json_extra: Option<&'a serde_json::Value>,
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
    let mut plan = plan_with(
        output_path,
        requested,
        flow.env_from_values,
        mode.env_name.as_deref(),
        mode.example.as_deref(),
    )?;
    if mode.no_template {
        env_only(output_path, &mut plan, requested, flow.env_from_values)?;
    }
    if let Some(values) = flow.values {
        keep_differing(output_path, &mut plan, values)?;
    }
    // For `--json`, worked out before the write, as `init` does (DQ-21): the
    // `.gitignore` entries a new `.env` brings, and the `[vars]` record.
    let extra = if mode.json {
        Some(JsonExtra::of(output_path, &plan)?)
    } else {
        None
    };

    if mode.dry_run {
        let json = extra
            .as_ref()
            .map(|e| with_extra(json_doc(&plan, true, &Applied::default(), e), flow));
        emit(&plan, flow, Step::DryRun, json)?;
        return Ok(Outcome::Unchanged);
    }
    if plan.is_empty() {
        let json = extra
            .as_ref()
            .map(|e| with_extra(json_doc(&plan, false, &Applied::default(), e), flow));
        emit(
            &plan,
            flow,
            Step::Done {
                show_conflicts: true,
            },
            json,
        )?;
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
        let subset = if plan.to_example.vars.is_empty() {
            &plan.to_env
        } else {
            &plan.to_example
        };
        // ⚠️ Every file this run writes (I-247). It named only `.env.example`
        // while `.env` got the names too. A `.env` created afterwards from the
        // template is asked about on its own.
        let writes_env = !plan.to_env.vars.is_empty() && (plan.env_exists || plan.create_env_file);
        let both = format!("{} and {}", plan.template.name, plan.env_file);
        let file = match (plan.to_example.vars.is_empty(), writes_env) {
            (false, true) => both.as_str(),
            (false, false) => plan.template.name.as_str(),
            (true, _) => plan.env_file.as_str(),
        };
        println!("{}", (flow.preview)(subset).dimmed());
        if !plan.present.is_empty() {
            ui::info(format!(
                "Already in {}, skipped: {}",
                plan.template.name,
                plan.present.join(", ")
            ));
        }
        print_conflicts(&plan);
        let confirm = crate::utils::prompt::yes_no()
            .with_prompt((flow.question)(subset.vars.len(), file))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            // ⚠️ Exit 4 since 0.13.0 (R1): "no" to the write is the answer
            // `restore`, `cloud pull` and `remove` give 4 for. It was 0, which a
            // script could not tell from a write.
            aborted();
            return Err(crate::utils::prompt::Declined.into());
        }
    }

    let applied = apply(
        output_path,
        &plan,
        flow.render,
        flow.example_render,
        mode.verbose && !mode.json,
        Some(mode.interactive()),
        !mode.json,
    )?;
    // Asked: the conflicts were shown with the question.
    let json = extra
        .as_ref()
        .map(|e| with_extra(json_doc(&plan, false, &applied, e), flow));
    emit(
        &plan,
        flow,
        Step::Done {
            show_conflicts: !mode.interactive(),
        },
        json,
    )?;
    let without_value = |name: &String| {
        flow.values
            .is_none_or(|v| v.get(name).is_none_or(Option::is_none))
    };
    Ok(
        if flow.values.is_some() && !plan.to_env.vars.keys().any(without_value) {
            Outcome::WrittenWithValues
        } else {
            Outcome::Written
        },
    )
}

/// `add custom`: a name the env file already has with a value other than the
/// one given is kept, and taken out of the plan for both files (nothing about
/// it is written), so the run cannot say it added it. A name given without a
/// value never conflicts: the placeholder is not a value to compare.
///
/// ⚠️ The env file's difference wins over a template conflict for the same
/// name. It used to be skipped when `.env.example` also conflicted, and the
/// template nearly always does: it holds `FLAG=` while `add` would write the
/// placeholder. So `FLAG=true` against `.env`'s `FLAG=false` reported only
/// "already in .env" and a template conflict about a placeholder nobody typed,
/// and `--json` never named `.env` (ADD-1, found re-running the review
/// reproductions). The template conflict is dropped: a name given with a
/// value never puts that value in the template, so what differs is `.env`.
fn keep_differing(
    output_path: &Path,
    plan: &mut Plan,
    values: &IndexMap<String, Option<String>>,
) -> Result<()> {
    let env = read_assignments(&output_path.join(&plan.env_file))?.unwrap_or_default();
    for (name, given) in values {
        let Some(given) = given else { continue };
        let Some(existing) = env.get(name) else {
            // The env file lacks it. The template listing it with another
            // value (`API_URL=` against the placeholder) says nothing about
            // `.env`: the value given is written there, and the template is
            // left as it is (it already has the name).
            if plan.conflicts.iter().any(|c| &c.var_name == name) {
                plan.conflicts.retain(|c| &c.var_name != name);
                if plan.env_exists || plan.create_env_file {
                    if let Some(meta) = plan.requested.vars.get(name) {
                        plan.to_env.vars.insert(name.clone(), meta.clone());
                    }
                }
            }
            continue;
        };
        if existing != given {
            plan.env_kept.push(name.clone());
            plan.to_example.vars.remove(name);
            plan.conflicts.retain(|c| &c.var_name != name);
        }
    }
    Ok(())
}

/// `doc` with the flow's own fields added.
fn with_extra(mut doc: serde_json::Value, flow: &Flow) -> serde_json::Value {
    if let (Some(serde_json::Value::Object(extra)), Some(map)) =
        (flow.json_extra, doc.as_object_mut())
    {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    doc
}

/// `--json`: the result as one document. Names only, and conflicts with the
/// `.env.example` values they compare; nothing from `.env`, ever.
///
/// Stable fields (I-026): `dry_run`, `written`, `added` (to the template),
/// `skipped` (already there, same value), `conflicts` (`name`, `file`, and
/// `existing`, `new` for the template's; kept — an env file's own value is
/// never printed), `env_added`, `env_created`, `env_committable`; and since
/// 0.10.16, shared with `init --json` (DQ-21, T-INIT-36): `template`,
/// `gitignore` (entries added), `contract` (`action`, `file`, `added`),
/// `notices`.
fn json_doc(plan: &Plan, dry_run: bool, applied: &Applied, extra: &JsonExtra) -> serde_json::Value {
    fn sorted(v: &VarCollection) -> Vec<&str> {
        let mut n: Vec<&str> = v.vars.keys().map(String::as_str).collect();
        n.sort_unstable();
        n
    }
    serde_json::json!({
        "dry_run": dry_run,
        "written": !dry_run && !plan.is_empty(),
        "added": sorted(&plan.to_example),
        "skipped": plan.present,
        "conflicts": plan.conflicts.iter().map(|c| serde_json::json!({
            "name": c.var_name,
            "file": plan.template.name,
            "existing": c.existing_value,
            "new": c.new_value,
        })).chain(plan.env_kept.iter().map(|name| serde_json::json!({
            "name": name,
            "file": plan.env_file,
            "reason": "already set to a different value; kept",
        }))).collect::<Vec<_>>(),
        "env_added": sorted(&plan.to_env),
        "env_created": applied.env_created,
        "env_committable": applied.env_committable,
        "template": plan.template.name,
        "gitignore": if dry_run || applied.env_created || plan.create_env_file {
            extra.gitignore.clone()
        } else {
            Vec::new()
        },
        "contract": extra.contract,
        "notices": Vec::<String>::new(),
    })
}

/// The `--json` fields worked out before the write.
pub struct JsonExtra {
    /// What `.gitignore` gains if this run creates `.env`.
    gitignore: Vec<String>,
    contract: serde_json::Value,
}

impl JsonExtra {
    fn of(output_path: &Path, plan: &Plan) -> Result<JsonExtra> {
        let creates_env = !plan.env_exists && !plan.is_empty();
        Ok(JsonExtra {
            gitignore: if creates_env {
                crate::core::gitignore::pending_entries(output_path)
            } else {
                Vec::new()
            },
            contract: crate::commands::spec::writeback::pending(output_path, &plan.recordable())?
                .0
                .json(),
        })
    }
}

/// The conflicts in `plan`, as a warning and one line each (the question
/// shows them before it asks). Values shown are `.env.example`'s, never
/// `.env`'s; a name the env file already sets to another value is listed
/// without either value.
pub fn print_conflicts(plan: &Plan) {
    super::report::print_conflicts(&super::report::PlanView::of(plan));
}

/// Say what happened, in the form the flags ask for: one document under
/// `--json`, else the dry-run listing or the result lines.
fn emit(
    plan: &Plan,
    flow: &Flow,
    step: super::report::Step,
    json: Option<serde_json::Value>,
) -> Result<()> {
    let mode = if json.is_some() {
        OutMode::Json
    } else {
        OutMode::Text
    };
    let report = super::report::AddReport::new(
        plan,
        flow.what.clone(),
        flow.noun,
        flow.values.is_some(),
        step,
        json,
    );
    Out::new(mode).emit(&report)
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

fn env_is_committable(project_root: &Path, env_file: &str) -> bool {
    !crate::core::gitignore::env_file_is_protected(project_root, env_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::models::{VarMetadata, VarSource};
    use tempfile::TempDir;

    /// A file with a syntax error is read lossily, not line by line: 0.11.0
    /// took `DATABASE_URL=x` inside `P`'s value as set, so `add` skipped it.
    #[test]
    fn b1_a_broken_file_does_not_count_a_line_inside_a_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "P=\"a\nDATABASE_URL=x\nb\"\nbad line\nQ='1'\n").unwrap();
        let map = read_assignments(&path).unwrap().unwrap();
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(keys, ["P", "Q"]);
        assert_eq!(map["Q"], "1");
    }

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
            None,
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
        let p = plan(d.path(), &vars(&[("A", "1"), ("B", "2")]), None).unwrap();
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
        let p = plan(d.path(), &vars(&[("A", "x"), ("B", "${NOPE}")]), None).unwrap();
        assert_eq!(p.present, ["A", "B"]);
        assert!(p.conflicts.is_empty());
    }

    /// A file the parser refuses is still read, line by line.
    #[test]
    fn plan_reads_a_file_the_parser_refuses() {
        let d = TempDir::new().unwrap();
        fs::write(d.path().join(".env.example"), "A=1\nB=\"never closed\n").unwrap();
        let p = plan(d.path(), &vars(&[("A", "1")]), None).unwrap();
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
