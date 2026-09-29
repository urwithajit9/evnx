//! `evnx update` — how to get a newer version, and whether there is one.
//!
//! # Why this command exists
//!
//! evnx ships through nine channels and **none of them told anyone about a new
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
//! **No self-replacement.** evnx is installed by nine package managers, and a
//! binary that overwrites itself fights whichever one owns it — Homebrew and winget
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

mod channel;
pub use channel::Channel;

#[cfg(feature = "net")]
mod check;

/// Entry point for `evnx update`.
pub fn run(check: bool) -> Result<()> {
    let channel = Channel::detect();

    if check {
        return run_check(channel);
    }

    print_advice(channel);
    Ok(())
}

/// Tell the user how to upgrade, without contacting anything.
fn print_advice(channel: Channel) {
    println!();
    println!("  evnx {}", env!("CARGO_PKG_VERSION").bold());
    println!();

    match channel.upgrade_command() {
        Some(command) => {
            println!("  Installed via {}. Upgrade with:", channel.label().cyan());
            println!();
            println!("      {}", command.bold());
        }
        None => {
            // ⚠️ Guessing a command here would be worse than admitting the gap: a
            // wrong upgrade instruction can uninstall a working binary.
            println!(
                "  {} could not tell how evnx was installed.",
                "note:".yellow()
            );
            println!();
            println!("  Pick the one that matches how you got it:");
            println!();
            for (label, command) in Channel::ALL_COMMANDS {
                println!("      {:<14} {}", label.dimmed(), command);
            }
        }
    }

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

#[cfg(feature = "net")]
fn run_check(channel: Channel) -> Result<()> {
    check::run(channel)
}

#[cfg(not(feature = "net"))]
fn run_check(_channel: Channel) -> Result<()> {
    anyhow::bail!(
        "`--check` needs network support, and this build has none.\n\
         \x20 It was compiled without the `cloud` or `migrate` feature — most likely \
         with `cargo install evnx`, which uses no default features.\n\
         \x20 Reinstall with `cargo install evnx --features cloud`, or run \
         `evnx update` on its own, which works offline."
    )
}
