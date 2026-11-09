//! `evnx platform`: two-way sync with a hosting platform's environment
//! variables. Vercel is the first; others (Netlify, Fly, Railway, Render) slot in
//! as another `--to` value and another module beside `vercel`.
//!
//! Phase 1 is read-only against the platform: `diff` and `pull`. Behind the `net`
//! feature, like everything that opens a socket.
//!
//! # Never
//!
//! Print a value. Write a value into a log or the state file. Keep the token
//! anywhere but the one request header.

use anyhow::Result;

use crate::cli::PlatformCommands;
use crate::core::verdict::Verdict;

pub mod error;
pub mod local;
pub mod secret;
pub mod vercel;

pub use error::exit_code_of;

/// Platforms `--to` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Platform {
    /// Vercel project environment variables.
    Vercel,
}

/// Run a `platform` subcommand. `diff` answers with a verdict (1 when they
/// differ); `pull` is `Clean` or an error, `Declined` among them.
pub fn run(command: PlatformCommands) -> Result<Verdict> {
    match command {
        PlatformCommands::Diff(a) => match a.common.to {
            Platform::Vercel => vercel::diff::run(&a),
        },
    }
}
