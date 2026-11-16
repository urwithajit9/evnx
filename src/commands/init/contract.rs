//! The `.evnx.toml` contract init leaves behind (IN-15).
//!
//! # Where, and what
//!
//! - **No `.evnx.toml` at or above the target**: one is created in the target
//!   folder (DQ-18), with a `[vars]` entry for every variable the template now
//!   declares — in a package below the git root too, so `apps/web`'s variables
//!   are not required of `apps/api` (DQ-26).
//! - **One with `[vars]`**: the names this run introduced are recorded into
//!   it, exactly as `evnx add` records them; an entry already there is never
//!   rewritten (DQ-17).
//! - **One without `[vars]`**: the entries are appended, and every other
//!   section stays byte for byte (DQ-27).
//!
//! ⚠️ init never wrote this file, and never recorded into one either (I-242):
//! `spec init` and `add` kept a contract that the command setting the project
//! up ignored, and the docs said otherwise (D-132, D-164).
//!
//! The entries are the ones `spec init --with` writes, through the one
//! `.evnx.toml` writer (`config::save`), so the contract does not depend on
//! which command made it. A write that fails is said and does not fail init:
//! the env files are already written.

use colored::Colorize;
use std::path::Path;

use crate::commands::spec::writeback::{self, Outcome};
use crate::core::config;
use crate::schema::models::VarCollection;
use crate::utils::ui;

use super::shared::Target;

/// Keep the contract for `target` after a run. See the module docs.
///
/// Writes now and says so later: the returned closure prints the line. Every
/// file is written before init prints anything (RM-03): `init … | head -n 1`
/// closed stdout at the first line, and the contract, written after it, was
/// never written at all.
pub(crate) fn record(
    target: &Target,
    written: &VarCollection,
    whole: &VarCollection,
) -> impl FnOnce() {
    let verbose = target.verbose;
    let template = target.template_name().to_string();
    let skipped = target.no_contract;
    let mut counts = None;
    let done = if skipped {
        Ok(Outcome::AlreadyDeclared)
    } else {
        write_contract(target, written, whole, &mut counts)
    };
    move || match done {
        Ok(_) if skipped => {
            if verbose && !ui::is_quiet() {
                eprintln!("  .evnx.toml: no contract written (--no-contract)");
            }
        }
        // Only a contract built from the template: a record into an existing
        // `[vars]` keeps `writeback`'s line.
        Ok(Outcome::Created { names, config } | Outcome::Wrote { names, config, .. })
            if !names.is_empty() && counts.is_some() =>
        {
            say_where_from(
                &names,
                &config,
                &template,
                counts.unwrap_or_default(),
                verbose,
            );
        }
        Ok(outcome) => writeback::report(&outcome, verbose),
        Err((path, e)) => could_not(&path, &e),
    }
}

type Done = Result<Outcome, (std::path::PathBuf, anyhow::Error)>;

fn write_contract(
    target: &Target,
    written: &VarCollection,
    whole: &VarCollection,
    counts: &mut Option<(usize, usize)>,
) -> Done {
    let done: Result<Outcome, (std::path::PathBuf, anyhow::Error)> = match plan(target) {
        Ok(Contract::Record(path)) => {
            writeback::record(&target.root, written).map_err(|e| (path, e))
        }
        Ok(Contract::Create(path)) => crate::commands::spec::contract_into(&path, whole)
            .map(|d| {
                *counts = Some((d.secret, d.with_format));
                Outcome::Created {
                    names: d.names,
                    config: path.clone(),
                }
            })
            .map_err(|e| (path, e)),
        Ok(Contract::Append(path)) => crate::commands::spec::contract_into(&path, whole)
            .map(|d| {
                *counts = Some((d.secret, d.with_format));
                Outcome::Wrote {
                    names: d.names,
                    config: path.clone(),
                    through: None,
                }
            })
            .map_err(|e| (path, e)),
        Err(e) => Err((target.root.clone(), e)),
    };
    done
}

/// The line for a contract `init` wrote: where its names came from, where they
/// went, and what was guessed for them. stderr, hidden by `-q`, as the
/// contract line always was.
fn say_where_from(
    names: &[String],
    config: &Path,
    template: &str,
    (secret, with_format): (usize, usize),
    verbose: bool,
) {
    if ui::is_quiet() {
        return;
    }
    eprintln!(
        "  {} {} from {template} → {} ({secret} secret, {with_format} with a format)",
        ui::glyph::OK.green(),
        crate::utils::string::count(names.len(), "name"),
        ui::short_path(config),
    );
    if verbose {
        for n in names {
            eprintln!("     {n}");
        }
    }
}

/// What [`record`] does for `target`, decided before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Contract {
    /// No `.evnx.toml` at or above the target: create this one.
    Create(std::path::PathBuf),
    /// The one in force has `[vars]`: record new names into it.
    Record(std::path::PathBuf),
    /// The one in force has no `[vars]`: append entries to it.
    Append(std::path::PathBuf),
}

pub(crate) fn plan(target: &Target) -> anyhow::Result<Contract> {
    // `--here`: a config above this folder is not this project's.
    let found = config::find(&target.root).filter(|found| {
        !target.here || found.parent() == std::path::absolute(&target.root).ok().as_deref()
    });
    let Some(path) = found else {
        // Absolute, as `find` returns one, so it is shown like every other
        // config line (`.evnx.toml`, not `./.evnx.toml`).
        let root = std::path::absolute(&target.root).unwrap_or_else(|_| target.root.clone());
        return Ok(Contract::Create(root.join(config::PROJECT_FILE)));
    };
    let text = std::fs::read_to_string(&path)?;
    Ok(if config::has_vars_section(&text) {
        Contract::Record(path)
    } else {
        Contract::Append(path)
    })
}

fn could_not(path: &Path, e: &anyhow::Error) {
    ui::warning(format!("could not update {}: {e:#}", ui::short_path(path)));
}
