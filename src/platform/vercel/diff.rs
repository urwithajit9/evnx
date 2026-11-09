//! `evnx platform diff --to vercel`: the plan, and nothing written anywhere.

use std::path::Path;

use anyhow::{bail, Result};

use super::{plan, shown, Session};
use crate::cli::PlatformDiffArgs;
use crate::core::verdict::Verdict;
use crate::platform::local;

/// Compare the local file with a Vercel environment. `Findings` when they differ.
pub fn run(args: &PlatformDiffArgs) -> Result<Verdict> {
    let session = Session::open(&args.common)?;
    let (path, _) = local::resolve(
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

    let (sel, listing) = session.list()?;
    // Values are read only for what is compared: Config keys the local file has.
    let remote = session.read(&sel.entries, |k| file.vars.contains_key(k))?;
    session.warn(&sel, &listing);

    let plan = plan::diff(&file.vars, &remote);
    let counts = plan.counts();
    let label = session.project.label();
    let file_name = shown(&path);

    if args.json() {
        let j = plan::to_json(
            &plan,
            &session.project.project,
            session.project.team.as_deref(),
            session.environment,
            &file_name,
            listing.hidden_production,
        );
        println!("{}", serde_json::to_string_pretty(&j)?);
    } else {
        println!(
            "Vercel  {label}  {}        local  {file_name}",
            session.environment.name()
        );
        print!("{}", plan::render(&plan));
        println!("{}", plan::summary(counts));
    }

    Ok(if plan.differs() {
        Verdict::Findings
    } else {
        Verdict::Clean
    })
}
