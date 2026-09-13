//! `evnx cloud status` — report local cloud-sync state.

use anyhow::Result;
use colored::Colorize;

/// Print whether this machine is set up for cloud sync.
///
/// Scaffold. There is no credential store to read until PR 2, so this always
/// reports "not configured". The output shape is deliberate — later PRs fill in
/// the server, account and vault lines rather than reformatting it, so the
/// difference in a review stays small.
pub fn run(verbose: bool) -> Result<()> {
    println!("{}", "evnx cloud".bold());
    println!("  status    {}", "not configured".yellow());
    println!();
    println!("  Cloud sync is compiled into this build, but no account is set up");
    println!("  on this machine. Sign-in arrives with `evnx auth login`.");

    if verbose {
        println!();
        println!("  Nothing is read from disk yet: the credential store lands in a");
        println!("  later change. No network request was made.");
    }

    println!();
    println!("{}", crate::docs::CLOUD.hint_line());
    Ok(())
}
