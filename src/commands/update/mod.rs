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

#[cfg(feature = "net")]
mod check;

/// Entry point for `evnx update`.
///
/// [`Verdict::Findings`] (exit 1) only from `--check`, when a newer release
/// exists (DEC-SF4); without `--check` there is nothing to find.
pub fn run(check: bool, json: bool) -> Result<Verdict> {
    let channel = Channel::detect();

    if check {
        return run_check(channel, json);
    }

    print_advice(channel);
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
fn run_check(channel: Channel, json: bool) -> Result<Verdict> {
    check::run(channel, json)
}

#[cfg(not(feature = "net"))]
fn run_check(_channel: Channel, _json: bool) -> Result<Verdict> {
    anyhow::bail!(
        "`--check` needs network support, and this build has none.\n\
         \x20 It was compiled without the `cloud` or `migrate` feature — most likely \
         with `cargo install evnx`, which uses no default features.\n\
         \x20 Reinstall with `{}`, or run `evnx update` on its own, which works \
         offline.",
        crate::core::build::CARGO_INSTALL
    )
}
