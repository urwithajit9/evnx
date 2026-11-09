//! `evnx remove` (alias `rm`): take a variable out of the env file, and with
//! `--template`, `--contract` or `--all`, out of the template, the contract and
//! every environment file too (R1, v0.13.0).
//!
//! Design and every rule id below: `specs/evnx-remove-command-plan.md`, as
//! amended by its §13.
//!
//! # How a run goes
//!
//! 1. **Resolve** the files, the same way `add` and `validate` do: `--env-name`,
//!    else `[defaults] env_name`, else `.env`. The file and where its name came
//!    from are printed every time (S-7).
//! 2. **Plan** every file's new content in memory, each through
//!    [`edit::cut_entries`] or [`contract::remove`], both of which re-read
//!    their result and refuse one that lost anything else (S-3, S-4).
//! 3. **Ask**, unless `--yes`; with no terminal, refuse (S-8). "No" is exit 4.
//! 4. **Apply**: back up each env file, stage every write, and commit them in
//!    order, each only if the file still holds what was read (S-6). A failure
//!    part-way puts back the files already changed and says which (S-5).
//!
//! ⛔ No value is ever printed, in any mode or error (S-1): only names, files
//! and line numbers. `--json` carries none either.

pub mod check;
pub mod contract;
pub mod edit;
pub mod prune;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::*;

use crate::core::config::Template;
use crate::core::gitignore::{self, Exposure};
use crate::core::{env_name, glob::glob_match, safe_write};
use crate::utils::path::label;
use crate::utils::prompt::Declined;
use crate::utils::string::count;
use crate::utils::ui;

/// Everything `evnx remove` takes.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub names: Vec<String>,
    pub path: String,
    pub env_name: Option<String>,
    pub template: bool,
    pub contract: bool,
    pub all: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub ignore_missing: bool,
    pub no_backup: bool,
    pub json: bool,
    pub verbose: bool,
    /// `--check`: read-only; see [`check`].
    pub check: bool,
}

/// What a file is to this command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Env,
    Template,
    Contract,
}

impl Kind {
    /// The flag that brings a file of this kind into scope.
    fn flag(self) -> &'static str {
        match self {
            Kind::Env => "--all",
            Kind::Template => "--template",
            Kind::Contract => "--contract",
        }
    }
}

/// One file `remove` reads: its kind, path, label, the bytes read, and the
/// names it holds.
#[derive(Debug)]
struct Place {
    kind: Kind,
    path: PathBuf,
    label: String,
    original: String,
    names: Vec<String>,
}

/// A planned change to one file.
#[derive(Debug)]
struct Change {
    place_index: usize,
    text: String,
    /// Env files and the template: each entry cut. The contract: empty.
    removed: Vec<edit::Removed>,
    /// The contract: each `[vars.NAME]` removed.
    tables: Vec<String>,
}

/// Main entry point for `evnx remove`.
pub fn run(opts: Options) -> Result<()> {
    run_with(opts, &mut |_, _| {})
}

/// [`run`], with a call just before each file is committed: the place tests
/// change a file under a run (E-5) or break the third write (E-3).
pub fn run_with(opts: Options, before_commit: &mut dyn FnMut(usize, &Path)) -> Result<()> {
    if opts.json && !opts.yes && !opts.dry_run {
        bail!("--json needs --yes or --dry-run: it never asks.");
    }
    if opts.no_backup && !opts.yes && !opts.dry_run {
        bail!("--no-backup needs --yes: the backup is the only copy of what is removed.");
    }
    if opts.json {
        ui::set_quiet(true);
    }
    let project = Path::new(&opts.path);
    if !project.is_dir() {
        bail!("{} is not a folder (--path)", label(&opts.path));
    }
    check_arguments(project, &opts.names)?;

    let loaded = crate::core::config::load(project)?;
    let cfg = loaded.config;

    // ── Resolve ──────────────────────────────────────────────────────────────
    let mut places: Vec<Place> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let (env_path, env_source) = resolve_env(project, &opts, cfg.defaults.env_name.as_deref())?;
    let mut env_files: Vec<PathBuf> = env_path.iter().cloned().collect();
    // Every environment file, the ones in scope and the ones only checked for
    // the "still present in" hint.
    let mut all_env: Vec<PathBuf> = Vec::new();
    if project.join(env_name::DEFAULT_FILE).is_file() {
        all_env.push(in_dir(project, env_name::DEFAULT_FILE));
    }
    for n in env_name::available(project) {
        all_env.push(in_dir(project, &format!(".env.{n}")));
    }
    if opts.all {
        env_files = all_env.clone();
    }
    for p in &env_files {
        places.push(read_place(Kind::Env, p)?);
    }

    let template = Template::of(project, &cfg);
    let template_path = in_dir(project, &template.name);
    let template_scope = opts.template || opts.all;
    if template_path.is_file() {
        // Out of scope it is read only for the hint, and never stops a run.
        match read_place(Kind::Template, &template_path) {
            Ok(p) => places.push(p),
            Err(e) if template_scope => return Err(e),
            Err(_) => {}
        }
    } else if opts.template {
        bail!(
            "there is no template to remove from: {} does not exist{}",
            label(&template_path.to_string_lossy()),
            if template.configured {
                " (named by [defaults] example)"
            } else {
                ""
            }
        );
    } else if opts.all {
        notes.push(format!(
            "no template ({}), skipped",
            label(&template_path.to_string_lossy())
        ));
    }

    let contract_scope = opts.contract || opts.all;
    match &loaded.source {
        Some(src) => match read_place(Kind::Contract, src) {
            Ok(p) => places.push(p),
            Err(e) if contract_scope => return Err(e),
            Err(_) => {}
        },
        None if opts.contract => bail!(
            "there is no .evnx.toml to remove from (--contract): this project has no contract"
        ),
        None if opts.all => notes.push("no .evnx.toml, skipped".to_string()),
        None => {}
    }
    // Env files outside the scope are read only for the hint.
    for p in all_env.iter().filter(|p| !env_files.contains(p)) {
        if let Ok(place) = read_place(Kind::Env, p) {
            places.push(place);
        }
    }
    let in_scope = |p: &Place| match p.kind {
        Kind::Env => env_files.contains(&p.path),
        Kind::Template => template_scope,
        Kind::Contract => contract_scope,
    };

    // ── Match ────────────────────────────────────────────────────────────────
    let matches = |p: &Place| -> BTreeSet<String> {
        p.names
            .iter()
            .filter(|k| opts.names.iter().any(|pat| glob_match(pat, k)))
            .cloned()
            .collect()
    };
    let mut missing: Vec<&String> = Vec::new();
    for pat in &opts.names {
        let found = places
            .iter()
            .filter(|p| in_scope(p))
            .any(|p| p.names.iter().any(|k| glob_match(pat, k)));
        if !found {
            missing.push(pat);
        }
    }
    if !missing.is_empty() && !opts.ignore_missing {
        bail!("{}", not_found(&missing, &places, &in_scope));
    }
    // S-10: a glob that takes every variable in a file needs `--yes`.
    if !opts.yes && !opts.dry_run {
        for p in places
            .iter()
            .filter(|p| in_scope(p) && p.kind != Kind::Contract)
        {
            let wide = opts
                .names
                .iter()
                .filter(|pat| pat.contains(['*', '?']))
                .any(|pat| !p.names.is_empty() && p.names.iter().all(|k| glob_match(pat, k)));
            if wide {
                bail!(
                    "{} matches every variable in {} ({}). Pass --yes if that is what you mean; \
                     --dry-run lists them.",
                    opts.names.join(" "),
                    p.label,
                    count(p.names.len(), "variable")
                );
            }
        }
    }

    // ── Plan ─────────────────────────────────────────────────────────────────
    let mut changes: Vec<Change> = Vec::new();
    let mut removed_names: BTreeSet<String> = BTreeSet::new();
    for (i, p) in places.iter().enumerate().filter(|(_, p)| in_scope(p)) {
        let m = matches(p);
        if m.is_empty() {
            continue;
        }
        removed_names.extend(m.iter().cloned());
        changes.push(match p.kind {
            Kind::Contract => Change {
                place_index: i,
                text: contract::remove(&p.original, &m, &p.label)?,
                removed: Vec::new(),
                tables: m.into_iter().collect(),
            },
            _ => {
                let cut = edit::cut_entries(&p.original, &m, &p.label)?;
                Change {
                    place_index: i,
                    text: cut.text,
                    removed: cut.removed,
                    tables: Vec::new(),
                }
            }
        });
    }
    let still: Vec<(String, String, &'static str)> = places
        .iter()
        .filter(|p| !in_scope(p))
        .flat_map(|p| {
            removed_names
                .iter()
                .filter(|n| p.names.contains(n))
                .map(|n| {
                    let place = if p.kind == Kind::Contract {
                        format!("{} [vars]", p.label)
                    } else {
                        p.label.clone()
                    };
                    (n.clone(), place, p.kind.flag())
                })
                .collect::<Vec<_>>()
        })
        .collect();
    // C-4: the contract still requires a name that leaves the env file.
    let still_required: Vec<&String> = if contract_scope {
        Vec::new()
    } else {
        removed_names
            .iter()
            .filter(|n| cfg.vars.get(*n).is_some_and(|v| v.is_required()))
            .collect()
    };

    // S-15, S-16: git and the env files that change.
    let mut tracked: Vec<String> = Vec::new();
    let mut unignored: Vec<String> = Vec::new();
    if gitignore::in_work_tree(project) {
        for c in &changes {
            let p = &places[c.place_index];
            if p.kind != Kind::Env {
                continue;
            }
            let name = p.path.file_name().unwrap_or_default().to_string_lossy();
            match gitignore::exposure(project, &name) {
                Exposure::Tracked => tracked.push(p.label.clone()),
                Exposure::Unignored => unignored.push(p.label.clone()),
                Exposure::Protected => {}
            }
        }
    }
    if opts.no_backup && !tracked.is_empty() && !opts.dry_run {
        bail!(
            "--no-backup is refused for a file git tracks ({}): keep the backup.",
            tracked.join(", ")
        );
    }

    let resolved = env_path.as_ref().map(|p| (p, env_source));

    // ── Dry run ──────────────────────────────────────────────────────────────
    if opts.dry_run {
        if opts.json {
            print_json(true, resolved, &places, &changes, &still, &missing, &[]);
        } else {
            print_resolved(resolved);
            println!("Dry run: nothing was written.");
            print_changes(&places, &changes, &in_scope);
            print_still(&still);
            for n in &notes {
                println!("  {}", n.dimmed());
            }
        }
        warn_after(&still_required, &tracked, &unignored);
        return Ok(());
    }

    if changes.is_empty() {
        // Only reachable with --ignore-missing.
        if opts.json {
            print_json(false, resolved, &places, &changes, &still, &missing, &[]);
        } else {
            print_resolved(resolved);
            ui::say("  Nothing to remove.");
        }
        return Ok(());
    }

    // ── Ask ──────────────────────────────────────────────────────────────────
    if !opts.yes {
        print_resolved(resolved);
        print_changes(&places, &changes, &in_scope);
        print_still(&still);
        warn_after(&still_required, &tracked, &unignored);
        let required_in_contract: Vec<&String> = changes
            .iter()
            .flat_map(|c| &c.tables)
            .filter(|n| cfg.vars.get(*n).is_some_and(|v| v.is_required()))
            .collect();
        let question = format!(
            "Remove {} from {}{}{}?",
            removed_names.iter().cloned().collect::<Vec<_>>().join(", "),
            count(changes.len(), "place"),
            if required_in_contract.is_empty() {
                String::new()
            } else {
                format!(
                    " (the contract marks {} required)",
                    required_in_contract
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            match resolved {
                Some((p, Source::Configured))
                    if env_name::name_of(&p.to_string_lossy()) == Some("production") =>
                    " — .env.production, from [defaults] env_name",
                _ => "",
            }
        );
        let example = format!("evnx remove {} --yes", opts.names.join(" "));
        if !crate::utils::prompt::confirm(&question, false, &example)? {
            println!("  Cancelled; nothing was changed.");
            return Err(Declined.into());
        }
    } else {
        print_resolved(resolved);
        warn_after(&still_required, &tracked, &unignored);
    }

    // ── Apply ────────────────────────────────────────────────────────────────
    let backups = apply(&places, &changes, opts.no_backup, before_commit)?;

    if opts.json {
        print_json(
            false, resolved, &places, &changes, &still, &missing, &backups,
        );
        return Ok(());
    }
    for (c, b) in changes.iter().zip(&backups) {
        let p = &places[c.place_index];
        let what = describe(c);
        let backup = b
            .as_ref()
            .map(|b| format!("   backup {}", label(&b.to_string_lossy())))
            .unwrap_or_default();
        ui::success(format!("{:<18} removed {what}{}", p.label, backup.dimmed()));
    }
    print_still(&still);
    for n in &notes {
        ui::say(format!("  {}", n.dimmed()));
    }
    // S-20: removal is not rotation.
    ui::info(
        "The value still exists in git history, older vault versions and any .bak copy. \
         Rotate it at the provider if it was ever real.",
    );
    Ok(())
}

/// Where the env file's name came from (S-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Flag,
    Configured,
    Default,
}

impl Source {
    fn text(self) -> &'static str {
        match self {
            Source::Flag => "--env-name",
            Source::Configured => "from [defaults] env_name",
            Source::Default => "default",
        }
    }
    fn json(self) -> &'static str {
        match self {
            Source::Flag => "env_name",
            Source::Configured => "defaults.env_name",
            Source::Default => "default",
        }
    }
}

/// The env file in scope without `--all`, and where its name came from. With
/// `--all` and no `.env`, `None`: every other place is still in scope.
fn resolve_env(
    project: &Path,
    opts: &Options,
    configured: Option<&str>,
) -> Result<(Option<PathBuf>, Source)> {
    let (name, source) = match (&opts.env_name, configured) {
        (Some(n), _) => (Some(n.as_str()), Source::Flag),
        (None, Some(n)) => (Some(n), Source::Configured),
        (None, None) => (None, Source::Default),
    };
    if opts.all && opts.env_name.is_none() {
        let p = in_dir(project, env_name::DEFAULT_FILE);
        return Ok((p.is_file().then_some(p), Source::Default));
    }
    let path = env_name::resolve(project, name)?;
    if !path.is_file() {
        if path.exists() {
            bail!("{} is not a file", label(&path.to_string_lossy()));
        }
        bail!(
            "{} does not exist, so there is nothing to remove from.\n  {}",
            label(&path.to_string_lossy()),
            crate::core::Parser::missing_env_hint(&path.to_string_lossy())
        );
    }
    Ok((Some(path), source))
}

/// `file` in `project`, as a person types it from there.
fn in_dir(project: &Path, file: &str) -> PathBuf {
    if project == Path::new(".") {
        PathBuf::from(file)
    } else {
        project.join(file)
    }
}

/// Read one file, refusing what cannot be edited in place (R-13, A-20).
fn read_place(kind: Kind, path: &Path) -> Result<Place> {
    let shown = if kind == Kind::Contract {
        ui::short_path(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
    } else {
        label(&path.to_string_lossy())
    };
    let meta = std::fs::metadata(path).with_context(|| format!("reading {shown}"))?;
    if !meta.is_file() {
        bail!("{shown} is not a regular file");
    }
    if meta.permissions().readonly() {
        bail!("{shown} is read-only, so nothing was changed");
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {shown}"))?;
    let original = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("{shown} is not UTF-8 text, so nothing was changed"))?;
    let names = match kind {
        Kind::Contract => contract::declared(&original, &shown)?,
        _ => edit::keys(&edit::document(&original)),
    };
    Ok(Place {
        kind,
        path: path.to_path_buf(),
        label: shown,
        original,
        names,
    })
}

/// S-11: an argument a shell made out of an unquoted glob is a file name.
/// Since D12, `.` and `-` can be in a key, so only an argument that names an
/// existing path and no variable is refused for that.
fn check_arguments(project: &Path, names: &[String]) -> Result<()> {
    for n in names {
        if n.is_empty() {
            bail!("a variable name cannot be empty");
        }
        if let Some((k, _)) = n.split_once('=') {
            bail!("give only the name to remove, not a value: `evnx remove {k}`");
        }
        let ok = n
            .chars()
            .all(|c| c.is_alphanumeric() || "_*?.-".contains(c));
        if !ok {
            bail!(
                "'{n}' is not a variable name or a glob. If your shell expanded a glob into \
                 file names, quote it: evnx remove 'NAME_*'"
            );
        }
        if (n.contains('.') || n.contains('-'))
            && (Path::new(n).exists() || project.join(n).exists())
        {
            bail!(
                "'{n}' is a file here, not a variable name: did your shell expand an unquoted \
                 glob? Quote it: evnx remove 'NAME_*'"
            );
        }
    }
    Ok(())
}

/// "X is not in .env", with a near miss and wherever it is instead.
fn not_found(missing: &[&String], places: &[Place], in_scope: &dyn Fn(&Place) -> bool) -> String {
    let scope: Vec<&str> = places
        .iter()
        .filter(|p| in_scope(p))
        .map(|p| p.label.as_str())
        .collect();
    let mut out = Vec::new();
    for m in missing {
        let where_ = if scope.is_empty() {
            "any file in scope".to_string()
        } else {
            scope.join(", ")
        };
        let mut s = format!("{m} is not in {where_}.");
        let elsewhere: Vec<String> = places
            .iter()
            .filter(|p| !in_scope(p) && p.names.iter().any(|k| glob_match(m, k)))
            .map(|p| format!("\n  It is in {}: add {}", p.label, p.kind.flag()))
            .collect();
        s.extend(elsewhere);
        let all: BTreeSet<&str> = places
            .iter()
            .flat_map(|p| p.names.iter().map(String::as_str))
            .collect();
        let near = all
            .iter()
            .find(|k| k.eq_ignore_ascii_case(m))
            .copied()
            .or_else(|| {
                crate::utils::string::closest(m, all.iter().map(|k| (*k, *k)))
                    .first()
                    .copied()
            });
        if let Some(k) = near {
            s.push_str(&format!(
                "\n  Did you mean {k}?  (names are case-sensitive)"
            ));
        }
        out.push(s);
    }
    out.push("Pass --ignore-missing to treat a missing name as already removed.".to_string());
    out.join("\n")
}

fn describe(c: &Change) -> String {
    if !c.tables.is_empty() {
        return c
            .tables
            .iter()
            .map(|t| format!("[vars.{}]", crate::core::spec::toml_key(t)))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let mut by_name: BTreeMap<&str, Vec<&edit::Removed>> = BTreeMap::new();
    for r in &c.removed {
        by_name.entry(&r.name).or_default().push(r);
    }
    by_name
        .iter()
        .map(|(n, rs)| {
            let lines: Vec<String> = rs.iter().map(|r| r.line.to_string()).collect();
            let comments = rs.iter().filter(|r| r.lines[0] < r.line).count();
            format!(
                "{n} ({} {}{})",
                if lines.len() == 1 { "line" } else { "lines" },
                lines.join(", "),
                if comments > 0 {
                    format!(", with {}", count(comments, "attached comment line"))
                } else {
                    String::new()
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_resolved(resolved: Option<(&PathBuf, Source)>) {
    if let Some((p, s)) = resolved {
        let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        ui::say(format!("  {:<9} {} ({})", "file", abs.display(), s.text()));
    }
}

/// What a run would do, file by file: the dry run, and before the question.
fn print_changes(places: &[Place], changes: &[Change], in_scope: &dyn Fn(&Place) -> bool) {
    for c in changes {
        let p = &places[c.place_index];
        println!("  {:<18} would remove {}", p.label, describe(c));
    }
    // E-4: in scope, nothing to remove there.
    for (i, p) in places.iter().enumerate() {
        if in_scope(p) && !changes.iter().any(|c| c.place_index == i) {
            println!("  {:<18} {}", p.label, "not present".dimmed());
        }
    }
}

fn print_still(still: &[(String, String, &'static str)]) {
    if still.is_empty() {
        return;
    }
    ui::say("  still present in:");
    for (n, place, flag) in still {
        ui::say(format!("    {place:<18} {n:<16} add {flag}"));
    }
}

fn warn_after(required: &[&String], tracked: &[String], unignored: &[String]) {
    for n in required {
        ui::warning(format!(
            "{n} is required in .evnx.toml [vars]: `evnx validate` will now report it missing. \
             Add --contract to remove the declaration too."
        ));
    }
    for f in tracked {
        ui::warning(format!(
            "{f} is tracked by git: the removed value stays in its history"
        ));
    }
    for f in unignored {
        ui::warning(format!(
            "{f} is not covered by .gitignore: it and its .bak copy could be committed"
        ));
    }
}

/// Back up, stage, then commit each change; on a failure, put back the files
/// already committed (S-5). Returns each change's backup.
fn apply(
    places: &[Place],
    changes: &[Change],
    no_backup: bool,
    before_commit: &mut dyn FnMut(usize, &Path),
) -> Result<Vec<Option<PathBuf>>> {
    let mut backups = Vec::new();
    for c in changes {
        let p = &places[c.place_index];
        // Env files hold the only copy of a secret; the template and the
        // contract are committed, so git keeps theirs (S-14).
        backups.push(if p.kind == Kind::Env && !no_backup {
            Some(safe_write::backup(&p.path, p.original.as_bytes())?)
        } else {
            None
        });
    }
    let mut staged = Vec::new();
    for c in changes {
        let p = &places[c.place_index];
        staged.push(if p.kind == Kind::Env {
            safe_write::stage_private(&p.path, c.text.as_bytes())?
        } else {
            safe_write::stage_shared(&p.path, c.text.as_bytes())?
        });
    }
    let mut done: Vec<usize> = Vec::new();
    for (i, (c, s)) in changes.iter().zip(staged).enumerate() {
        let p = &places[c.place_index];
        before_commit(i, &p.path);
        let result = match std::fs::read(&p.path) {
            // S-6: compare and swap.
            Ok(now) if now == p.original.as_bytes() => s.commit().map(|w| safe_write::report(&w)),
            Ok(_) => Err(anyhow::anyhow!(
                "{} changed while evnx was running",
                p.label
            )),
            Err(e) => Err(anyhow::Error::from(e).context(format!("reading {}", p.label))),
        };
        if let Err(e) = result {
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            for &j in &done {
                let q = &places[changes[j].place_index];
                let put_back = if q.kind == Kind::Env {
                    safe_write::write_private(&q.path, q.original.as_bytes())
                } else {
                    safe_write::write_shared(&q.path, q.original.as_bytes())
                };
                match put_back {
                    Ok(_) => restored.push(q.label.clone()),
                    Err(_) => failed.push(q.label.clone()),
                }
            }
            let mut msg = format!("{e:#}\n  {} was not changed.", p.label);
            if !restored.is_empty() {
                msg.push_str(&format!(
                    "\n  Put back as they were: {}.",
                    restored.join(", ")
                ));
            }
            if !failed.is_empty() {
                msg.push_str(&format!(
                    "\n  ⚠ Could not put back: {}. Their .bak copies hold the originals.",
                    failed.join(", ")
                ));
            }
            let untouched: Vec<&str> = changes[i + 1..]
                .iter()
                .map(|c| places[c.place_index].label.as_str())
                .collect();
            if !untouched.is_empty() {
                msg.push_str(&format!("\n  Not reached: {}.", untouched.join(", ")));
            }
            bail!(msg);
        }
        done.push(i);
    }
    Ok(backups)
}

/// `--json` (§7.2): names, files and lines only, never a value.
fn print_json(
    dry_run: bool,
    resolved: Option<(&PathBuf, Source)>,
    places: &[Place],
    changes: &[Change],
    still: &[(String, String, &'static str)],
    missing: &[&String],
    backups: &[Option<PathBuf>],
) {
    let mut removed = Vec::new();
    for c in changes {
        let file = &places[c.place_index].label;
        for r in &c.removed {
            removed.push(serde_json::json!({ "name": r.name, "file": file, "lines": r.lines }));
        }
        for t in &c.tables {
            removed
                .push(serde_json::json!({ "name": t, "file": file, "table": format!("vars.{}", crate::core::spec::toml_key(t)) }));
        }
    }
    let doc = serde_json::json!({
        "schema_version": 1,
        "dry_run": dry_run,
        "resolved": resolved.map(|(p, s)| serde_json::json!({
            "env_file": std::path::absolute(p).unwrap_or_else(|_| p.clone()),
            "source": s.json(),
        })),
        "removed": removed,
        "still_present": still.iter().map(|(n, w, f)| serde_json::json!({
            "name": n, "where": w, "flag": f,
        })).collect::<Vec<_>>(),
        "missing": missing,
        "backups": backups.iter().flatten().map(|b| label(&b.to_string_lossy())).collect::<Vec<_>>(),
    });
    println!("{doc:#}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        for (n, c) in files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        d
    }

    fn all_yes(d: &tempfile::TempDir, name: &str) -> Options {
        Options {
            names: vec![name.to_string()],
            path: d.path().to_string_lossy().into_owned(),
            all: true,
            yes: true,
            ..Options::default()
        }
    }

    fn read(d: &tempfile::TempDir, n: &str) -> String {
        std::fs::read_to_string(d.path().join(n)).unwrap()
    }

    const FILES: &[(&str, &str)] = &[
        (".env", "X=1\nY=1\n"),
        (".env.production", "X=2\nY=2\n"),
        (".env.staging", "X=3\nY=3\n"),
        (".env.example", "X=\nY=\n"),
    ];

    /// E-5, S-6: a file changed between the read and the write is not
    /// overwritten, and the files already written are put back.
    #[test]
    fn e5_a_file_changed_under_the_run_is_left_alone_and_the_rest_put_back() {
        let d = project(FILES);
        let err = run_with(all_yes(&d, "X"), &mut |i, path| {
            if i == 1 {
                std::fs::write(path, "X=2\nY=2\nZ=someone else\n").unwrap();
            }
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("changed while evnx was running"), "{err}");
        assert!(err.contains("Put back as they were: "), "{err}");
        assert_eq!(read(&d, ".env"), "X=1\nY=1\n");
        assert_eq!(read(&d, ".env.production"), "X=2\nY=2\nZ=someone else\n");
        assert_eq!(read(&d, ".env.staging"), "X=3\nY=3\n");
        assert_eq!(read(&d, ".env.example"), "X=\nY=\n");
    }

    /// E-3, S-5: the third write fails; the first two are restored and the
    /// error says exactly which.
    #[test]
    fn e3_a_failed_third_write_restores_the_first_two() {
        let d = project(FILES);
        let err = run_with(all_yes(&d, "X"), &mut |i, path| {
            if i == 2 {
                std::fs::remove_file(path).unwrap();
                std::fs::create_dir(path).unwrap();
            }
        })
        .unwrap_err()
        .to_string();
        let staging = d.path().join(".env.staging").to_string_lossy().into_owned();
        assert!(err.contains(&format!("{staging} was not changed")), "{err}");
        let env = d.path().join(".env").to_string_lossy().into_owned();
        let prod = d
            .path()
            .join(".env.production")
            .to_string_lossy()
            .into_owned();
        assert!(
            err.contains(&format!("Put back as they were: {env}, {prod}.")),
            "{err}"
        );
        assert!(err.contains("Not reached: "), "{err}");
        assert_eq!(read(&d, ".env"), "X=1\nY=1\n");
        assert_eq!(read(&d, ".env.production"), "X=2\nY=2\n");
        assert_eq!(read(&d, ".env.example"), "X=\nY=\n");
        // The backups were taken first and stay.
        assert_eq!(read(&d, ".env.bak"), "X=1\nY=1\n");
    }

    #[test]
    fn the_hook_does_nothing_on_a_good_run() {
        let d = project(FILES);
        run_with(all_yes(&d, "X"), &mut |_, _| {}).unwrap();
        for (n, _) in FILES {
            assert!(!read(&d, n).contains('X'), "{n}");
        }
    }
}
