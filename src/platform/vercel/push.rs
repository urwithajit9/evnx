//! `evnx platform push --to vercel`: the local file's values onto a Vercel
//! environment.
//!
//! # What it does
//!
//! * A key Vercel lacks is **created** (one bulk `POST …/env?upsert=false`). Its
//!   type is the classifier's (`plan::suggested_kind`): a public prefix is always
//!   Config, `--type` overrides the rest, and only for new variables.
//! * A **Config** whose value differs is **updated** by id (`PATCH`).
//! * A **Secret** on both sides is an update too, because its value cannot be
//!   compared (`--only-new` skips them).
//! * An existing variable keeps its type: a Secret cannot become Config in place,
//!   so a type difference is shown and not changed.
//! * Variables only Vercel has are kept. Deleting them is `--prune`, a later phase.
//! * A local key with an **empty value** is never pushed: `pull` writes `KEY=` for
//!   a Secret it cannot fetch, and pushing that back would blank the real one.
//!
//! # Never an upsert
//!
//! Upsert matches by key and **replaces the whole target list** (design §2a, P0
//! #3), so one environment's push could rewrite another's. A variable is a key
//! plus a set of targets: when the entry being changed also covers other
//! environments, this environment is first removed from its targets (`PATCH`),
//! then created as its own entry of the same type.
//!
//! # Safety
//!
//! * Before any write each entry about to change is compared with the state file
//!   (`.evnx/platform/vercel.json`): if Vercel's `updatedAt` differs from what the
//!   last pull or push saw, nothing is written and the exit code is 14 (D4).
//! * `failed[]` is read on every create: a 201 can carry failures. Any failure is
//!   exit 1 naming the keys and Vercel's error code; the rest stays applied.
//! * Values are held as [`SecretString`], reach the wire only in a request body,
//!   and appear in no output, error or file. `--dry-run` sends no request that
//!   has a value in it. A response body is never printed: create and delete
//!   answers echo ciphertext.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use colored::Colorize;
use indexmap::IndexMap;
use serde_json::json;

use super::api::{Created, Failure, Kind, NewEnv, RemoteVar};
use super::plan::{self, Action, Environment, Remote, Why};
use super::{shown, state, Session};
use crate::cli::PlatformPushArgs;
use crate::core::verdict::Verdict;
use crate::platform::error::PlatformError;
use crate::platform::local;
use crate::platform::secret::SecretString;
use crate::utils::protect::GitignoreMode;

// ── The plan ─────────────────────────────────────────────────────────────────

/// What push does with one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOp {
    /// Not on Vercel: create it.
    Create,
    /// On both sides, and different or not comparable: change it.
    Update,
    /// On both sides, and the same.
    Unchanged,
    /// A Secret on both sides, skipped by `--only-new`.
    SkipSecret,
    /// An empty local value: never pushed.
    SkipEmpty,
    /// The type differs: shown, not changed.
    TypeKept,
    /// On Vercel only: kept.
    Kept,
}

impl PushOp {
    /// The word in the table and in JSON.
    pub fn word(self) -> &'static str {
        match self {
            PushOp::Create => "create",
            PushOp::Update => "update",
            PushOp::Unchanged => "unchanged",
            PushOp::SkipSecret | PushOp::SkipEmpty => "skipped",
            PushOp::TypeKept => "type differs",
            PushOp::Kept => "remote only",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            PushOp::Create => "+",
            PushOp::Update => "~",
            PushOp::Unchanged | PushOp::SkipSecret => "=",
            PushOp::SkipEmpty => "-",
            PushOp::TypeKept | PushOp::Kept => "!",
        }
    }

    fn writes(self) -> bool {
        matches!(self, PushOp::Create | PushOp::Update)
    }
}

/// One line of the push plan. No value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRow {
    /// The variable's name.
    pub key: String,
    /// Its type: Vercel's, or for a create the one evnx will give it.
    pub kind: Kind,
    /// What happens.
    pub op: PushOp,
    /// The parenthesised remark.
    pub note: Option<String>,
}

/// A value change of an entry that only this environment uses.
pub struct Patch {
    /// The variable.
    pub key: String,
    /// The entry's id.
    pub id: String,
    /// The new value.
    pub value: SecretString,
}

/// An entry shared with other environments whose value must change here: this
/// environment leaves it (`PATCH target`), and gets its own entry (a create).
pub struct Split {
    /// The variable.
    pub key: String,
    /// The shared entry's id.
    pub id: String,
    /// The targets it keeps: everything but this environment.
    pub remaining: Vec<String>,
}

/// What push will send, and the table that shows it.
#[derive(Default)]
pub struct PushPlan {
    /// One row per key, in file order, then keys only Vercel has.
    pub rows: Vec<PushRow>,
    /// Entries to create: new keys, and the own entry of each [`Split`].
    pub creates: Vec<NewEnv>,
    /// Shared entries to leave this environment.
    pub splits: Vec<Split>,
    /// Entries to change in place.
    pub patches: Vec<Patch>,
}

/// What the flags change about the plan.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// `--only-new`: leave existing Secrets alone.
    pub only_new: bool,
    /// `--type`: the type of a new variable, whatever the classifier says.
    pub type_override: Option<Kind>,
}

impl PushPlan {
    /// Variables push will write.
    pub fn writes(&self) -> usize {
        self.rows.iter().filter(|r| r.op.writes()).count()
    }

    fn count(&self, op: PushOp) -> usize {
        self.rows.iter().filter(|r| r.op == op).count()
    }

    /// Whether any request carries a value.
    pub fn is_empty(&self) -> bool {
        self.creates.is_empty() && self.patches.is_empty()
    }

    /// The entries (by id) a write will touch, with the key.
    pub fn touched(&self) -> impl Iterator<Item = (&str, &str)> {
        self.patches
            .iter()
            .map(|p| (p.key.as_str(), p.id.as_str()))
            .chain(self.splits.iter().map(|s| (s.key.as_str(), s.id.as_str())))
    }
}

/// The type an entry that is being recreated keeps.
fn existing_type(r: &Remote) -> String {
    match (r.kind, r.var.raw_type.as_str()) {
        (Kind::Secret, _) => Kind::Secret.vercel_type().into(),
        (Kind::Config, t @ ("encrypted" | "plain")) => t.into(),
        (Kind::Config, _) => Kind::Config.vercel_type().into(),
    }
}

/// Work out what push does. `remote` holds the chosen environment's entries,
/// with the Config values the local file has keys for already read.
pub fn build(
    local: &IndexMap<String, String>,
    remote: &[Remote],
    env: Environment,
    opts: Options,
) -> PushPlan {
    // A key with an empty value is out of the comparison altogether, so that it
    // is neither "remote only" nor an update that would blank a Secret.
    let sendable: IndexMap<String, String> = local
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let compared = plan::diff(&sendable, remote);
    let rows_by_key: HashMap<&str, &plan::Row> =
        compared.rows.iter().map(|r| (r.key.as_str(), r)).collect();
    let remote_by_key: HashMap<&str, &Remote> =
        remote.iter().map(|r| (r.var.key.as_str(), r)).collect();

    let mut out = PushPlan::default();
    for (key, value) in local {
        if value.is_empty() {
            let (kind, note) = match remote_by_key.get(key.as_str()) {
                Some(r) => (r.kind, "empty value locally; Vercel's is left as it is"),
                None => (
                    plan::suggested_kind(key, value),
                    "empty value; nothing to create",
                ),
            };
            out.rows.push(PushRow {
                key: key.clone(),
                kind,
                op: PushOp::SkipEmpty,
                note: Some(note.into()),
            });
            continue;
        }
        let row = rows_by_key[key.as_str()];
        let mut push_row = PushRow {
            key: key.clone(),
            kind: row.kind,
            op: PushOp::Unchanged,
            note: None,
        };
        match row.action {
            Action::Add => {
                // A public prefix is Config whatever was asked: Vercel refuses
                // a Secret there.
                let public = plan::has_public_prefix(key);
                let kind = if public {
                    Kind::Config
                } else {
                    opts.type_override.unwrap_or(row.kind)
                };
                if public && opts.type_override == Some(Kind::Secret) {
                    push_row.note =
                        Some("public prefix: stays config despite --type secret".into());
                }
                push_row.kind = kind;
                push_row.op = PushOp::Create;
                out.creates.push(NewEnv {
                    key: key.clone(),
                    value: SecretString::new(value.as_str()),
                    type_: kind.vercel_type().into(),
                    target: vec![env.name().into()],
                });
            }
            Action::Update(why) => {
                let r = remote_by_key[key.as_str()];
                if why == Why::WriteOnly && opts.only_new {
                    push_row.op = PushOp::SkipSecret;
                    push_row.note = Some("secret: --only-new leaves existing secrets alone".into());
                } else {
                    push_row.op = PushOp::Update;
                    let base = match why {
                        Why::ValueDiffers => "value differs",
                        Why::WriteOnly => {
                            "write-only: cannot tell whether it changed; --only-new skips"
                        }
                    };
                    let others: Vec<String> = r
                        .var
                        .target
                        .iter()
                        .filter(|t| t.as_str() != env.name())
                        .cloned()
                        .collect();
                    if others.is_empty() {
                        push_row.note = Some(base.into());
                        out.patches.push(Patch {
                            key: key.clone(),
                            id: r.var.id.clone(),
                            value: SecretString::new(value.as_str()),
                        });
                    } else {
                        push_row.note = Some(format!(
                            "{base}; shared with {}, so {} gets its own entry",
                            others.join(", "),
                            env.name()
                        ));
                        out.creates.push(NewEnv {
                            key: key.clone(),
                            value: SecretString::new(value.as_str()),
                            type_: existing_type(r),
                            target: vec![env.name().into()],
                        });
                        out.splits.push(Split {
                            key: key.clone(),
                            id: r.var.id.clone(),
                            remaining: others,
                        });
                    }
                }
            }
            Action::Unchanged => {}
            Action::Present => unreachable!("plan::diff never returns Present"),
            Action::TypeDiffers => {
                push_row.op = PushOp::TypeKept;
                push_row.note = row.note();
            }
            Action::RemoteOnly => unreachable!("a local key is never remote only"),
        }
        out.rows.push(push_row);
    }
    for r in remote {
        if !local.contains_key(&r.var.key) {
            out.rows.push(PushRow {
                key: r.var.key.clone(),
                kind: r.kind,
                op: PushOp::Kept,
                note: Some("kept; --prune to delete (coming)".into()),
            });
        }
    }
    out
}

/// The table. Keys, types and actions only.
pub fn render(plan: &PushPlan) -> String {
    use std::fmt::Write;
    let width = plan
        .rows
        .iter()
        .map(|r| r.key.len())
        .max()
        .unwrap_or(0)
        .max(8);
    let mut out = String::new();
    for r in &plan.rows {
        let note = r
            .note
            .as_ref()
            .map(|n| format!("  ({n})"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {} {:width$}  {:6}  {}{}",
            r.op.symbol(),
            r.key,
            r.kind.word(),
            r.op.word(),
            note
        );
    }
    out
}

/// `2 create · 3 update · 1 unchanged · 1 remote only · values never shown`
pub fn summary(plan: &PushPlan) -> String {
    let skipped = plan.count(PushOp::SkipSecret) + plan.count(PushOp::SkipEmpty);
    let mut parts = Vec::new();
    for (n, w) in [
        (plan.count(PushOp::Create), "create"),
        (plan.count(PushOp::Update), "update"),
        (plan.count(PushOp::Unchanged), "unchanged"),
        (skipped, "skipped"),
        (plan.count(PushOp::TypeKept), "type differs (not changed)"),
        (plan.count(PushOp::Kept), "remote only (kept)"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {w}"));
        }
    }
    if parts.is_empty() {
        parts.push("nothing to compare".into());
    }
    parts.push("values never shown".into());
    parts.join(" · ")
}

// ── Doing it ─────────────────────────────────────────────────────────────────

/// A variable push could not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    /// The variable, when known.
    pub key: Option<String>,
    /// Vercel's error code, or the HTTP status.
    pub code: String,
}

/// What a push did.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Variables created.
    pub created: usize,
    /// Variables changed in place or recreated for this environment.
    pub updated: usize,
    /// What Vercel refused.
    pub failed: Vec<Failed>,
    /// Whether any request was accepted (the state file is stale then).
    pub touched: bool,
}

/// The code of a refusal that is about one entry, not about the whole run:
/// `None` for the failures that end it (token, rate limit, server, network).
fn entry_failure(e: &PlatformError) -> Option<String> {
    match e {
        PlatformError::Refused { status, detail } => Some(
            detail
                .as_deref()
                .and_then(|d| d.strip_prefix("error code "))
                .map_or_else(|| format!("HTTP {status}"), str::to_string),
        ),
        PlatformError::NotFound { .. } => Some("not found (HTTP 404)".into()),
        _ => None,
    }
}

/// Send the plan: leave shared entries, one bulk create, then in-place changes.
///
/// A refusal of one entry is recorded and the rest goes on; anything that ends
/// the run (token, rate limit, server, network) is an error saying how far it
/// got.
pub fn apply(session: &Session, plan: &PushPlan) -> Result<Outcome> {
    let project = session.project.project.as_str();
    let api = &session.api;
    let mut out = Outcome::default();
    let progress = |o: &Outcome| {
        format!(
            "{} created, {} updated before it stopped",
            o.created, o.updated
        )
    };

    // 1. Shared entries leave this environment. A key whose entry could not be
    //    split is left alone: creating its own entry would collide with it.
    let mut skip_create: Vec<&str> = Vec::new();
    for s in &plan.splits {
        match api.patch_target(project, &s.id, &s.remaining) {
            Ok(()) => out.touched = true,
            Err(e) => match entry_failure(&e) {
                Some(code) => {
                    skip_create.push(&s.key);
                    out.failed.push(Failed {
                        key: Some(s.key.clone()),
                        code,
                    });
                }
                None => {
                    return Err(anyhow::Error::new(e).context(format!(
                        "leaving a shared entry of {}: {}",
                        s.key,
                        progress(&out)
                    )))
                }
            },
        }
    }

    // 2. One bulk create.
    let creates: Vec<NewEnv> = plan
        .creates
        .iter()
        .filter(|c| !skip_create.contains(&c.key.as_str()))
        .map(|c| NewEnv {
            key: c.key.clone(),
            value: SecretString::new(c.value.expose()),
            type_: c.type_.clone(),
            target: c.target.clone(),
        })
        .collect();
    if !creates.is_empty() {
        let Created { created, failed } = api.create_envs(project, &creates).map_err(|e| {
            anyhow::Error::new(e).context(format!(
                "creating {} variable(s): {}",
                creates.len(),
                progress(&out)
            ))
        })?;
        out.touched |= created > 0;
        let split_keys: Vec<&str> = plan.splits.iter().map(|s| s.key.as_str()).collect();
        let n_split_ok = creates
            .iter()
            .filter(|c| split_keys.contains(&c.key.as_str()))
            .count()
            .saturating_sub(
                failed
                    .iter()
                    .filter(|f| f.key.as_deref().is_some_and(|k| split_keys.contains(&k)))
                    .count(),
            );
        // Recreated entries count as updates, new keys as creates.
        out.updated += n_split_ok.min(created);
        out.created += created.saturating_sub(n_split_ok.min(created));
        out.failed.extend(
            failed
                .into_iter()
                .map(|Failure { key, code }| Failed { key, code }),
        );
    }

    // 3. In-place changes.
    for p in &plan.patches {
        match api.patch_value(project, &p.id, &p.value) {
            Ok(()) => {
                out.touched = true;
                out.updated += 1;
            }
            Err(e) => match entry_failure(&e) {
                Some(code) => out.failed.push(Failed {
                    key: Some(p.key.clone()),
                    code,
                }),
                None => {
                    return Err(anyhow::Error::new(e).context(format!(
                        "updating {}: {}",
                        p.key,
                        progress(&out)
                    )))
                }
            },
        }
    }
    Ok(out)
}

// ── The command ──────────────────────────────────────────────────────────────

/// Where the lines about the run go: stdout, except with `--format json`, where
/// stdout holds the one JSON document.
struct Say {
    json: bool,
}

impl Say {
    fn line(&self, s: impl AsRef<str>) {
        if self.json {
            eprintln!("{}", s.as_ref());
        } else {
            println!("{}", s.as_ref());
        }
    }
}

/// Push. `Findings` when Vercel refused some of it; `Err(Declined)` on a no.
pub fn run(args: &PlatformPushArgs) -> Result<Verdict> {
    let session = Session::open(&args.common)?;
    let (path, choice) = local::resolve(
        Path::new("."),
        args.common.file.as_deref(),
        session.environment.name(),
    );
    let file = local::read(&path)?;
    if file.text.is_none() {
        bail!(
            "{} not found. Pass --file <path>, or create .env.{} or .env.",
            shown(&path),
            session.environment.name()
        );
    }
    let say = Say { json: args.json() };
    let env = session.environment;
    if choice == local::Choice::Default && env == Environment::Production {
        eprintln!(
            "warning: pushing {} to production — pass --file .env.production if that is not what you meant",
            shown(&path)
        );
    }

    let (sel, listing) = session.list()?;
    let remote = session.read(&sel.entries, |k| {
        file.vars.get(k).is_some_and(|v| !v.is_empty())
    })?;
    session.warn(&sel, &listing);

    let opts = Options {
        only_new: args.only_new,
        type_override: args.type_.map(Kind::from),
    };
    let plan = build(&file.vars, &remote, env, opts);
    let file_name = shown(&path);

    // D4: before anything is written, has Vercel moved since the last pull/push?
    check_state(&session, &plan, &remote)?;

    let nothing_to_say = crate::utils::ui::is_quiet() && args.yes && !args.dry_run && !args.json();
    if !args.json() && !nothing_to_say {
        println!(
            "Vercel  {}  {}        local  {file_name}",
            session.project.label(),
            env.name()
        );
        print!("{}", render(&plan));
        println!("{}", summary(&plan));
    }

    let finish_json = |outcome: Option<&Outcome>, applied: bool| -> Result<()> {
        if args.json() {
            let j = to_json(&plan, &session, &file_name, args, outcome, applied);
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        Ok(())
    };

    if plan.writes() == 0 {
        say.line(format!(
            "  Vercel already has everything in {file_name} that push would send."
        ));
        finish_json(None, false)?;
        return Ok(Verdict::Clean);
    }

    if args.dry_run {
        say.line("  Dry run: nothing was sent.");
        finish_json(None, false)?;
        return Ok(Verdict::Clean);
    }

    let question = format!(
        "Push {} variable(s) to Vercel ({}, {})?",
        plan.writes(),
        session.project.label(),
        env.name()
    );
    let example = format!(
        "evnx platform push --to vercel --env-name {} --yes",
        env.name()
    );
    if !crate::utils::prompt::confirm(&question, args.yes, &example)? {
        say.line("  Cancelled; nothing was sent to Vercel.");
        return Err(crate::utils::prompt::Declined.into());
    }

    let outcome = apply(&session, &plan)?;
    if outcome.created > 0 {
        say.line(format!(
            "  {} created {} variable{}",
            "✓".green(),
            outcome.created,
            plural(outcome.created)
        ));
    }
    if outcome.updated > 0 {
        say.line(format!(
            "  {} updated {} variable{}",
            "✓".green(),
            outcome.updated,
            plural(outcome.updated)
        ));
    }

    if outcome.touched {
        // What Vercel holds now becomes the new "last seen". If this listing
        // fails the writes are done, but the next push would see a change that
        // is its own: say so.
        let gitignore = GitignoreMode::from_flags(args.gitignore, args.no_gitignore);
        let after = session.list().context(
            "the changes were applied, but listing Vercel again to refresh \
             .evnx/platform/vercel.json failed",
        )?;
        record(
            &session,
            &after.0.entries,
            &gitignore,
            args.yes,
            args.json(),
        )?;
    }

    if !outcome.failed.is_empty() {
        report_failures(&outcome.failed, &plan);
    }
    finish_json(Some(&outcome), true)?;
    Ok(if outcome.failed.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Findings
    })
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// `super::record`, with its "recorded" line off stdout under `--format json`.
fn record(
    session: &Session,
    entries: &[RemoteVar],
    gitignore: &GitignoreMode,
    yes: bool,
    json: bool,
) -> Result<()> {
    if json {
        // stdout carries only the JSON document: the usual lines are not wanted.
        let root = Path::new(".");
        let written = state::write(root, &session.project, session.environment, entries)?;
        eprintln!("recorded {} (ids only, no values)", shown(&written));
        return Ok(());
    }
    super::record(session, entries, gitignore, yes)
}

/// Name every failed key with Vercel's code, on stderr.
fn report_failures(failed: &[Failed], plan: &PushPlan) {
    eprintln!(
        "error: Vercel did not accept {} variable{}; everything else was applied:",
        failed.len(),
        plural(failed.len())
    );
    for f in failed {
        match &f.key {
            Some(k) => eprintln!("  {k}  ({})", f.code),
            None => eprintln!("  (a variable Vercel did not name)  ({})", f.code),
        }
        if let Some(k) = &f.key {
            if plan.splits.iter().any(|s| &s.key == k) {
                eprintln!(
                    "    {k} may no longer exist for this environment: its shared entry was split first. \
                     Run `evnx platform diff` and push again."
                );
            }
        }
        let lower = f.code.to_ascii_lowercase();
        if lower.contains("separate") || lower.contains("policy") {
            eprintln!(
                "    A team policy (\"Separate Production Secret Values\") may refuse the same \
                 Secret value in Production and another environment."
            );
        }
    }
}

// ── The D4 check ─────────────────────────────────────────────────────────────

/// Stop (exit 14) when an entry about to change is not as the state file says.
/// No state file, or one for another project or environment, is a warning.
fn check_state(session: &Session, plan: &PushPlan, remote: &[Remote]) -> Result<()> {
    let touched: Vec<(&str, &str)> = plan.touched().collect();
    if touched.is_empty() {
        return Ok(());
    }
    let loaded = match state::load(Path::new("."), &session.project, session.environment) {
        state::Loaded::Usable(s) => s,
        state::Loaded::Missing => {
            eprintln!(
                "warning: no {} yet, so a change made on Vercel since your last pull cannot be detected. \
                 `evnx platform pull --to vercel` records it.",
                state::PATH
            );
            return Ok(());
        }
        state::Loaded::Unusable(why) => {
            eprintln!(
                "warning: {} {why}, so changes made on Vercel since your last pull cannot be detected.",
                state::PATH
            );
            return Ok(());
        }
    };
    let current: HashMap<&str, &RemoteVar> =
        remote.iter().map(|r| (r.var.id.as_str(), &r.var)).collect();
    let mut changed: Vec<String> = Vec::new();
    for (key, id) in touched {
        if let (Some(seen), Some(now)) = (loaded.updated_at(id), current.get(id)) {
            if seen != &now.updated_at && !changed.iter().any(|k| k == key) {
                changed.push(key.to_string());
            }
        }
    }
    if changed.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::Changed { keys: changed }.into())
    }
}

// ── JSON ─────────────────────────────────────────────────────────────────────

fn to_json(
    plan: &PushPlan,
    session: &Session,
    file: &str,
    args: &PlatformPushArgs,
    outcome: Option<&Outcome>,
    applied: bool,
) -> serde_json::Value {
    json!({
        "platform": "vercel",
        "project": session.project.project,
        "team": session.project.team,
        "environment": session.environment.name(),
        "file": file,
        "dry_run": args.dry_run,
        "applied": applied,
        "summary": {
            "create": plan.count(PushOp::Create),
            "update": plan.count(PushOp::Update),
            "unchanged": plan.count(PushOp::Unchanged),
            "skipped": plan.count(PushOp::SkipSecret) + plan.count(PushOp::SkipEmpty),
            "type_differs": plan.count(PushOp::TypeKept),
            "remote_only": plan.count(PushOp::Kept),
        },
        "entries": plan.rows.iter().map(|r| json!({
            "key": r.key,
            "type": r.kind.word(),
            "action": r.op.word().replace(' ', "_"),
        })).collect::<Vec<_>>(),
        "result": outcome.map(|o| json!({
            "created": o.created,
            "updated": o.updated,
            "failed": o.failed.iter().map(|f| json!({ "key": f.key, "code": f.code })).collect::<Vec<_>>(),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn remote(id: &str, key: &str, kind: Kind, target: &[&str], value: Option<&str>) -> Remote {
        Remote {
            var: RemoteVar {
                id: id.into(),
                key: key.into(),
                kind,
                raw_type: match kind {
                    Kind::Config => "encrypted".into(),
                    Kind::Secret => "sensitive".into(),
                },
                target: target.iter().map(|s| (*s).to_string()).collect(),
                git_branch: None,
                updated_at: Value::from(1),
            },
            kind,
            value: value.map(SecretString::new),
            read: true,
        }
    }

    fn local(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn op(p: &PushPlan, k: &str) -> PushOp {
        p.rows.iter().find(|r| r.key == k).unwrap().op
    }

    #[test]
    fn every_op_is_produced_and_only_writes_are_sent() {
        let l = local(&[
            ("NEW_ONE", "1"),
            ("API_URL", "http://b"),
            ("SAME", "x"),
            ("DB_URL", "postgres://x"),
            ("AUTH_SECRET", "hunter2"),
            ("EMPTY", ""),
        ]);
        let r = vec![
            remote(
                "a",
                "API_URL",
                Kind::Config,
                &["development"],
                Some("http://a"),
            ),
            remote("s", "SAME", Kind::Config, &["development"], Some("x")),
            remote("d", "DB_URL", Kind::Secret, &["development"], None),
            remote(
                "h",
                "AUTH_SECRET",
                Kind::Config,
                &["development"],
                Some("hunter2"),
            ),
            remote("r", "REDIS_URL", Kind::Secret, &["development"], None),
            remote("e", "EMPTY", Kind::Secret, &["development"], None),
        ];
        let p = build(&l, &r, Environment::Development, Options::default());
        assert_eq!(op(&p, "NEW_ONE"), PushOp::Create);
        assert_eq!(op(&p, "API_URL"), PushOp::Update);
        assert_eq!(op(&p, "SAME"), PushOp::Unchanged);
        assert_eq!(op(&p, "DB_URL"), PushOp::Update);
        assert_eq!(op(&p, "AUTH_SECRET"), PushOp::TypeKept);
        assert_eq!(op(&p, "EMPTY"), PushOp::SkipEmpty);
        assert_eq!(op(&p, "REDIS_URL"), PushOp::Kept);
        assert_eq!(p.creates.len(), 1);
        assert_eq!(p.patches.len(), 2);
        assert!(p.splits.is_empty());
        // The empty local value is never sent and "EMPTY" is not remote only.
        assert!(p.patches.iter().all(|x| x.key != "EMPTY"));
        assert_eq!(p.rows.iter().filter(|r| r.key == "EMPTY").count(), 1);
    }

    #[test]
    fn only_new_skips_existing_secrets_and_type_override_is_for_new_keys() {
        let l = local(&[("DB_URL", "x"), ("PLAIN", "v"), ("ALSO", "w")]);
        let r = vec![remote("d", "DB_URL", Kind::Secret, &["development"], None)];
        let p = build(
            &l,
            &r,
            Environment::Development,
            Options {
                only_new: true,
                type_override: Some(Kind::Secret),
            },
        );
        assert_eq!(op(&p, "DB_URL"), PushOp::SkipSecret);
        assert!(p.patches.is_empty());
        assert_eq!(p.creates.len(), 2);
        assert!(p.creates.iter().all(|c| c.type_ == "sensitive"));
        let p = build(
            &local(&[("STRIPE_SECRET_KEY", "x")]),
            &[],
            Environment::Development,
            Options {
                only_new: false,
                type_override: Some(Kind::Config),
            },
        );
        assert_eq!(p.creates[0].type_, "encrypted");
    }

    #[test]
    fn a_public_prefix_is_config_even_when_secret_is_asked() {
        let p = build(
            &local(&[("NEXT_PUBLIC_API_KEY", "sk_live_abcdefghijklmnop")]),
            &[],
            Environment::Production,
            Options {
                only_new: false,
                type_override: Some(Kind::Secret),
            },
        );
        assert_eq!(p.creates[0].type_, "encrypted");
        assert_eq!(p.creates[0].target, ["production"]);
        assert!(p.rows[0].note.as_deref().unwrap().contains("public prefix"));
    }

    #[test]
    fn a_shared_entry_is_split_and_recreated_with_its_own_type() {
        let l = local(&[("API_URL", "http://b"), ("TOKEN", "t")]);
        let r = vec![
            remote(
                "a",
                "API_URL",
                Kind::Config,
                &["development", "preview"],
                Some("http://a"),
            ),
            remote(
                "t",
                "TOKEN",
                Kind::Secret,
                &["production", "development"],
                None,
            ),
        ];
        let p = build(&l, &r, Environment::Development, Options::default());
        assert!(p.patches.is_empty());
        assert_eq!(p.splits.len(), 2);
        assert_eq!(p.splits[0].remaining, ["preview"]);
        assert_eq!(p.splits[1].remaining, ["production"]);
        assert_eq!(p.creates.len(), 2);
        assert_eq!(p.creates[0].type_, "encrypted");
        assert_eq!(p.creates[1].type_, "sensitive");
        assert!(p.creates.iter().all(|c| c.target == ["development"]));
        assert!(p.rows[0]
            .note
            .as_deref()
            .unwrap()
            .contains("shared with preview"));
    }

    #[test]
    fn the_table_holds_no_value() {
        let l = local(&[("A", "very-secret-local"), ("B", "very-secret-two")]);
        let r = vec![remote(
            "b",
            "B",
            Kind::Config,
            &["development"],
            Some("very-secret-remote"),
        )];
        let p = build(&l, &r, Environment::Development, Options::default());
        let text = format!("{}{}", render(&p), summary(&p));
        assert!(!text.contains("very-secret"), "{text}");
        assert!(text.contains("1 create · 1 update"), "{text}");
    }

    #[test]
    fn entry_failures_are_told_from_run_ending_ones() {
        let refused = PlatformError::Refused {
            status: 400,
            detail: Some("error code bad_request".into()),
        };
        assert_eq!(entry_failure(&refused).as_deref(), Some("bad_request"));
        assert!(entry_failure(&PlatformError::Unauthorized).is_none());
        assert!(entry_failure(&PlatformError::RateLimited { tries: 3 }).is_none());
    }
}
