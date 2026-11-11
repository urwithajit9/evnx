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
    let (path, choice) = local::resolve(
        Path::new("."),
        args.common.file.as_deref(),
        session.environment.name(),
    );
    let file = local::read(&path)?;
    // F3: with no `.env.<env>` and no `.env`, the local side is empty and every
    // Vercel variable is "remote only", which the run says. A `--file` that does
    // not exist is a typo, and stays an error.
    let local_found = file.text.is_some();
    if !local_found {
        if choice == local::Choice::Explicit {
            bail!("{} not found.", shown(&path));
        }
        eprintln!(
            "note: neither .env.{} nor .env exists, so the local side is empty and every \
             Vercel variable shows as remote only.",
            session.environment.name()
        );
    }

    let (sel, listing) = session.list()?;
    // Values are read only for what is compared: Config keys the local file has.
    let remote = session.read(&sel, |k| file.vars.contains_key(k))?;
    session.warn(&sel, &listing);

    // A Secret on both sides cannot be compared: it is `present`, not a
    // difference, unless `--strict` asks for the old reading (F1).
    let plan = plan::diff(&file.vars, &remote);
    let plan = if args.strict { plan } else { plan.presented() };
    let counts = plan.counts();
    let label = session.project.label();
    let file_name = shown(&path);

    if args.json() {
        let mut j = plan::to_json(
            &plan,
            &session.project.project,
            session.project.team.as_deref(),
            &session.environment,
            &file_name,
            listing.hidden_production,
            args.strict,
        );
        j["local_file_found"] = local_found.into();
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
