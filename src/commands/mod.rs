pub mod add;
pub mod align;
pub mod backup;
pub mod config;
pub mod convert;
pub mod diff;
pub mod doctor;
pub mod fill;
pub mod get;
pub mod init;
pub mod migrate;
pub mod parse;
pub mod remove;
pub mod restore;
pub mod run;
pub mod scan;
pub mod set;
pub mod spec;
pub mod surface;
pub mod sync;
pub mod template;
pub mod undo;
pub mod update;
pub mod validate;

pub use add::run as run_add;
pub use init::run as run_init;

pub mod completions {
    use anyhow::{Context, Result};
    use clap_complete::{Shell, generate};

    /// Print the completion script for `shell`.
    ///
    /// ⚠️ Generated into a buffer and written by `term::write_stdout`, not
    /// straight to stdout: clap_complete panicked on a closed pipe with a
    /// message the broken-pipe hook does not match (I-185, exit 101).
    pub fn run(shell: Shell) -> Result<()> {
        let mut cmd = crate::cli::exit_help::command();
        let mut script = Vec::new();
        generate(shell, &mut cmd, "evnx", &mut script);
        crate::utils::term::write_stdout(&script).context("writing the completion script")
    }
}
