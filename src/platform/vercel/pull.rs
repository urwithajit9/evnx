//! `evnx platform pull --to vercel`: Vercel's readable values into the local file.
//!
//! * **Config** values are written (existing lines keep their place, comments and
//!   formatting; new keys go at the end).
//! * A **Secret** whose key already has a local value keeps that value: pull never
//!   blanks a real local value (D2).
//! * A **Secret** the local file lacks is written as `KEY=` and listed, so a
//!   missing secret is a visible gap and not a surprise at runtime.
//! * Local-only keys are never touched.
//!
//! A preview and a confirmation come first (exit 4 on a no, `--yes` without a
//! terminal), the file is replaced atomically with a `.bak` copy, and no value is
//! ever printed. After a pull that went through, `.evnx/platform/vercel.json`
//! records the ids and `updatedAt` of what was pulled (D4).

use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use super::api::Kind;
use super::plan::{self, PullChange, PullOp};
use super::{shown, state, Session};
use crate::cli::PlatformPullArgs;
use crate::platform::local;
use crate::utils::protect::GitignoreMode;

/// Pull. `Err(Declined)` when the confirmation was answered no.
pub fn run(args: &PlatformPullArgs) -> Result<()> {
    let session = Session::open(&args.common)?;
    let (path, choice) = local::resolve(
        Path::new("."),
        args.common.file.as_deref(),
        session.environment.name(),
    );
    let file = local::read(&path)?;

    crate::say!(
        "  Vercel    {} ({})  {}",
        session.project.label(),
        session.project.source,
        session.environment.name()
    );
    crate::say!("  file      {} ({})", shown(&path), choice.note());
    if choice == local::Choice::Default && session.environment == plan::Environment::Production {
        eprintln!(
            "warning: pulling production values into {} — pass --file .env.production if that is not what you meant",
            shown(&path)
        );
    }

    let (sel, listing) = session.list()?;
    let remote = session.read(&sel.entries, |_| true)?;
    session.warn(&sel, &listing);
    let changes = plan::pull_plan(&file.vars, remote);

    // The preview is what a yes is given to, so it prints unless `-q` and nothing
    // is being asked: `--yes` (nobody to ask) or an up-to-date file stay silent.
    let local_only = file
        .vars
        .keys()
        .filter(|k| !changes.iter().any(|c| &c.key == *k))
        .count();
    let empty: Vec<&str> = changes
        .iter()
        .filter(|c| c.op == PullOp::WriteEmpty)
        .map(|c| c.key.as_str())
        .collect();
    if !crate::utils::ui::is_quiet() || args.dry_run || !args.yes {
        print!("{}", render(&changes));
        println!("{}", summary(&changes, local_only));
        if !empty.is_empty() {
            println!(
                "  Written empty (write-only on Vercel, so evnx cannot fetch the value): {}",
                empty.join(", ")
            );
        }
    }

    let set: Vec<(&str, &str)> = changes
        .iter()
        .filter(|c| c.op == PullOp::Update)
        .filter_map(|c| Some((c.key.as_str(), c.value.as_ref()?.expose())))
        .collect();
    let mut add: Vec<(&str, &str)> = changes
        .iter()
        .filter(|c| c.op == PullOp::Add)
        .filter_map(|c| Some((c.key.as_str(), c.value.as_ref()?.expose())))
        .collect();
    add.extend(empty.iter().map(|k| (*k, "")));

    let gitignore = GitignoreMode::from_flags(args.gitignore, args.no_gitignore);

    if set.is_empty() && add.is_empty() {
        crate::say!("  {} is already up to date.", shown(&path));
        if !args.dry_run {
            record(&session, &sel.entries, &gitignore)?;
        }
        return Ok(());
    }

    if args.dry_run {
        println!("  Dry run: nothing was written.");
        return Ok(());
    }

    let question = format!(
        "{} {} in {}?",
        if file.text.is_some() {
            "Update"
        } else {
            "Create"
        },
        format_args!("{} variable(s)", set.len() + add.len()),
        shown(&path)
    );
    let example = format!(
        "evnx platform pull --to vercel --env-name {} --yes",
        session.environment.name()
    );
    if !crate::utils::prompt::confirm(&question, args.yes, &example)? {
        println!("  Cancelled; {} was not changed.", shown(&path));
        return Err(crate::utils::prompt::Declined.into());
    }

    let current = file.text.as_ref().map_or("", |t| t.as_str());
    let updated = local::edit(current, &file.doc, &set, &add)?;
    let kept = match &file.text {
        Some(previous) if previous.as_str() != updated.as_str() => Some(
            crate::core::safe_write::backup(&path, previous.as_bytes())
                .with_context(|| format!("keeping a copy of {}", shown(&path)))?,
        ),
        _ => None,
    };
    let written = crate::core::safe_write::write_private(&path, updated.as_bytes())
        .with_context(|| format!("writing {}", shown(&path)))?;
    crate::core::safe_write::report(&written);
    crate::say!("  {} wrote {}", "✓".green(), shown(&path));
    if let Some(kept) = &kept {
        crate::say!(
            "  {} kept the previous {} as {}",
            "✓".green(),
            shown(&path),
            shown(kept)
        );
    }

    record(&session, &sel.entries, &gitignore)?;
    crate::utils::protect::protect_output(&path.to_string_lossy(), &gitignore)?;
    if let Some(kept) = &kept {
        crate::utils::protect::protect_output(&kept.to_string_lossy(), &gitignore)?;
    }
    Ok(())
}

/// Write the D4 state file, and hint at `.gitignore` when it is not ignored.
fn record(
    session: &Session,
    entries: &[super::api::RemoteVar],
    gitignore: &GitignoreMode,
) -> Result<()> {
    let root = Path::new(".");
    let written = state::write(root, &session.project, session.environment, entries)?;
    crate::say!(
        "  {} recorded {} (ids only, no values)",
        "✓".green(),
        shown(&written)
    );
    if !matches!(gitignore, GitignoreMode::Skip)
        && crate::core::gitignore::in_work_tree(root)
        && !crate::core::gitignore::is_protected(root, state::PATH)
    {
        eprintln!(
            "hint: {} is per-machine state; add `.evnx/platform/` to .gitignore to keep it out of commits.",
            state::PATH
        );
    }
    Ok(())
}

fn render(changes: &[PullChange]) -> String {
    use std::fmt::Write;
    let width = changes
        .iter()
        .map(|c| c.key.len())
        .max()
        .unwrap_or(0)
        .max(8);
    let mut out = String::new();
    for c in changes {
        let (sym, what) = match c.op {
            PullOp::Add => ("+", "add"),
            PullOp::Update => ("~", "update (local value replaced)"),
            PullOp::Same => ("=", "unchanged"),
            PullOp::KeepLocal => (
                "=",
                "kept (secret: write-only on Vercel, local value stays)",
            ),
            PullOp::WriteEmpty => ("!", "written empty (secret: write-only on Vercel)"),
        };
        let _ = writeln!(
            out,
            "  {sym} {:width$}  {:6}  {what}",
            c.key,
            match c.kind {
                Kind::Config => "config",
                Kind::Secret => "secret",
            }
        );
    }
    out
}

fn summary(changes: &[PullChange], local_only: usize) -> String {
    let n = |op: PullOp| changes.iter().filter(|c| c.op == op).count();
    let mut parts = Vec::new();
    for (count, word) in [
        (n(PullOp::Add), "add"),
        (n(PullOp::Update), "update"),
        (n(PullOp::Same), "unchanged"),
        (n(PullOp::KeepLocal), "secret kept"),
        (n(PullOp::WriteEmpty), "written empty"),
        (local_only, "local only (untouched)"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {word}"));
        }
    }
    if parts.is_empty() {
        parts.push("nothing on Vercel for this environment".into());
    }
    parts.push("values never shown".into());
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::secret::SecretString;

    #[test]
    fn the_preview_holds_no_value() {
        let changes = vec![
            PullChange {
                key: "A".into(),
                kind: Kind::Config,
                op: PullOp::Add,
                value: Some(SecretString::new("topsecret-value")),
            },
            PullChange {
                key: "B".into(),
                kind: Kind::Secret,
                op: PullOp::WriteEmpty,
                value: None,
            },
        ];
        let text = format!("{}{}", render(&changes), summary(&changes, 2));
        assert!(!text.contains("topsecret"), "{text}");
        assert!(text.contains("2 local only"), "{text}");
    }
}
