//! `evnx update` — how to get a newer version, and whether there is one.
//!
//! # Why this command exists
//!
//! evnx ships through several channels — cargo, Homebrew, Scoop, winget, npm,
//! pip and the install script — and **none of them told anyone about a new
//! release.** A user on 0.4.0 had no way to learn 0.6.0 existed short of visiting
//! the repository; the first person to hit this was the maintainer, running
//! `evnx --version` on a machine three releases behind.
//!
//! # What it deliberately does not do
//!
//! **No passive check.** Nothing here runs unless the user typed `evnx update`. A
//! secrets tool whose `scan`, `validate`, `sync` and `diff` make no network request
//! at all is usable in an air-gapped environment and in a CI job with egress rules,
//! and that property is worth more than a convenient upgrade nag.
//!
//! **No self-replacement.** evnx is installed by several package managers, and
//! a binary that overwrites itself fights whichever one owns it — Homebrew and winget
//! would then report a version they did not install, and the next `brew upgrade`
//! would either clobber it or refuse. So this detects the channel and prints
//! *that channel's* command, cooperating with the system instead of racing it.
//!
//! # Offline by default, on purpose
//!
//! [`run`] needs no network: the channel is read from the path of the running
//! executable. Only `--check` makes a request, and `reqwest` is an optional
//! dependency enabled by `cloud` or `migrate` — so a bare `cargo install evnx`
//! gets the offline half and is told plainly why `--check` is missing, rather than
//! silently acquiring an HTTP stack it never asked for.

use anyhow::Result;
use colored::Colorize;

use crate::core::verdict::Verdict;
use crate::utils::ui;

mod channel;
pub use channel::Channel;
mod copies;

#[cfg(feature = "net")]
mod check;

/// Entry point for `evnx update`.
///
/// [`Verdict::Findings`] (exit 1) only from `--check`, when a newer release
/// exists (DEC-SF4); without `--check` there is nothing to find. `--list`
/// exits 0 however many copies it finds (D8).
pub fn run(check: bool, list: bool, json: bool) -> Result<Verdict> {
    if json && !check && !list {
        anyhow::bail!(
            "--json goes with --check or --list: plain `evnx update` has no data to print"
        );
    }
    if list {
        return run_list(json);
    }
    let channel = Channel::detect();
    let others = copies::find().iter().filter(|c| !c.this).count();

    if check {
        let verdict = run_check(channel, json, others)?;
        if !json && !ui::is_quiet() {
            print_others(others);
        }
        return Ok(verdict);
    }

    print_advice(channel);
    if !ui::is_quiet() {
        print_age();
        print_others(others);
    }
    Ok(Verdict::Clean)
}

/// "N other copies…", one line, when there are any (RM-42).
fn print_others(others: usize) {
    if others > 0 {
        println!(
            "  {} {others} other {} of evnx {} on this machine: {}",
            "note:".yellow(),
            if others == 1 { "copy" } else { "copies" },
            if others == 1 { "is" } else { "are" },
            "evnx update --list".cyan()
        );
        println!();
    }
}

/// How old the commit this binary was built from is (`build.rs`).
fn print_age() {
    let Some(days) = age_days() else { return };
    let age = match days {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        n => format!("{n} days ago"),
    };
    println!("  This build's source was committed {age}.");
    println!();
}

fn age_days() -> Option<i64> {
    let built: i64 = option_env!("EVNX_COMMIT_EPOCH")?.parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((now - built).max(0) / 86_400)
}

/// `update --list`: every copy, the one that runs first (RM-42).
fn run_list(json: bool) -> Result<Verdict> {
    let mut found = copies::find();
    copies::read_versions(&mut found);

    if json {
        let list: Vec<_> = found
            .iter()
            .map(|c| {
                serde_json::json!({
                    "path": c.path,
                    "resolved": c.resolved,
                    "channel": c.channel.id(),
                    "version": c.version,
                    "version_from": c.version_from.id(),
                    "runs": c.runs,
                    "this": c.this,
                    "remove": copies::removal(c),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": 1,
                "count": found.len(),
                "copies": list,
            }))?
        );
        return Ok(Verdict::Clean);
    }

    if found.is_empty() {
        // Run from somewhere no search covers (a build folder, say).
        println!("  No copy of evnx on PATH or in the usual install folders.");
        return Ok(Verdict::Clean);
    }
    println!();
    for c in &found {
        let marker = if c.runs {
            "▶".green().to_string()
        } else {
            " ".into()
        };
        let version = c.version.as_deref().unwrap_or("unknown");
        let mut notes = vec![c.channel.label().to_string()];
        if c.this {
            notes.push("this one".into());
        }
        if c.version_from == copies::VersionFrom::Ran {
            notes.push("version from running it".into());
        }
        println!(
            "  {marker} {:<10} {}  ({})",
            version.bold(),
            c.path.display(),
            notes.join(", ")
        );
        if c.resolved != c.path {
            println!("      → {}", c.resolved.display().to_string().dimmed());
        }
    }
    println!();
    if found.iter().any(|c| c.runs) {
        println!("  ▶ is the copy that runs when you type `evnx`.");
    }
    if found.len() > 1 {
        // Keep the newest, which is not always the one that runs: an older
        // copy earlier on PATH is exactly the case this list exists for.
        let keep = copies::newest(&found).unwrap_or(0);
        if let Some(runs) = found.iter().position(|c| c.runs) {
            if runs != keep {
                println!(
                    "  {} the copy that runs is older than {}.",
                    "!".yellow(),
                    found[keep].path.display()
                );
            }
        }
        println!(
            "  Keep {}. To remove the others:",
            found[keep].path.display()
        );
        for (i, c) in found.iter().enumerate() {
            if i != keep {
                println!("      {}", copies::removal(c));
            }
        }
        println!("  Then check:  type -a evnx");
    }
    println!();
    Ok(Verdict::Clean)
}

/// Tell the user how to upgrade, without contacting anything.
fn print_advice(channel: Channel) {
    // DEC-SF5: `-q` keeps the answer, which is the command.
    if ui::is_quiet() {
        print_commands_only(channel);
        return;
    }

    println!();
    println!("  evnx {}", env!("CARGO_PKG_VERSION").bold());
    println!();

    print_upgrade(channel, true);

    println!();
    if cfg!(feature = "net") {
        println!(
            "  To see whether a newer version exists:  {}",
            "evnx update --check".cyan()
        );
    } else {
        // Said plainly rather than hidden, so nobody concludes the flag is broken.
        println!(
            "  {} this build has no network support, so {} is unavailable.",
            "note:".yellow(),
            "--check".cyan()
        );
        println!("  It was built without the `cloud` or `migrate` feature.");
    }
    println!();
}

/// How to upgrade from `channel`: its command, or the labelled list to choose
/// from. Shared by plain `update` and `--check`.
///
/// `list_unknown` is whether an unrecognised location prints every channel
/// (plain `update`) or points at `evnx update` (`--check`, whose answer is the
/// version, not the list).
fn print_upgrade(channel: Channel, list_unknown: bool) {
    let candidates = channel.candidates();
    match (channel, &candidates[..]) {
        (Channel::Unknown, _) if !list_unknown => {
            println!(
                "  Run {} to see the upgrade commands.",
                "evnx update".cyan()
            );
        }
        (Channel::Unknown, _) => {
            // ⚠️ Guessing a command here would be worse than admitting the gap: a
            // wrong upgrade instruction can uninstall a working binary.
            println!(
                "  {} could not tell how evnx was installed.",
                "note:".yellow()
            );
            println!();
            println!("  Pick the one that matches how you got it:");
            println!();
            print_list(&candidates);
        }
        (Channel::PipSystem, [(_, command)]) => {
            // UP-2: `pip install --upgrade` is refused here, and
            // `--break-system-packages` is never the advice.
            println!(
                "  Installed via pip, into a Python that refuses {} (PEP 668),",
                "pip install".cyan()
            );
            println!("  so pip cannot upgrade it. Move to pipx, which keeps its own environment:");
            println!();
            println!("      {}", command.bold());
            println!();
            let here = std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "this one".to_string());
            println!("  Then remove the old copy ({here}), so the two do not shadow each other.");
        }
        (_, [(_, command)]) => {
            println!("  Installed via {}. Upgrade with:", channel.label().cyan());
            println!();
            println!("      {}", command.bold());
        }
        _ => print_list(&candidates),
    }
}

fn print_list(candidates: &[(&str, &str)]) {
    for (label, command) in candidates {
        println!("      {:<14} {}", label.dimmed(), command);
    }
}

/// `-q`: the command and nothing else — bare when there is one, labelled when
/// the reader has to choose (DEC-SF5).
fn print_commands_only(channel: Channel) {
    match channel.candidates()[..] {
        [(_, command)] => println!("{command}"),
        ref several => {
            for (label, command) in several {
                println!("{label:<14} {command}");
            }
        }
    }
}

#[cfg(feature = "net")]
fn run_check(channel: Channel, json: bool, others: usize) -> Result<Verdict> {
    check::run(channel, json, others)
}

#[cfg(not(feature = "net"))]
fn run_check(_channel: Channel, _json: bool, _others: usize) -> Result<Verdict> {
    anyhow::bail!(
        "`--check` needs network support, and this build has none.\n\
         \x20 It was compiled without the `cloud` or `migrate` feature — most likely \
         with `cargo install evnx`, which uses no default features.\n\
         \x20 Reinstall with `{}`, or run `evnx update` on its own, which works \
         offline.",
        crate::core::build::CARGO_INSTALL
    )
}
