pub mod add;
pub mod backup;
pub mod convert;
pub mod diff;
pub mod doctor;
pub mod init;
pub mod migrate;
pub mod restore;
pub mod run;
pub mod scan;
pub mod spec;
pub mod surface;
pub mod sync;
pub mod template;
pub mod update;
pub mod validate;

pub use add::run as run_add;
pub use init::run as run_init;

pub mod completions {
    use crate::cli::Cli;
    use anyhow::{Context, Result};
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};

    /// Print the completion script for `shell`.
    ///
    /// ⚠️ Generated into a buffer and written by `term::write_stdout`, not
    /// straight to stdout: clap_complete panicked on a closed pipe with a
    /// message the broken-pipe hook does not match (I-185, exit 101).
    pub fn run(shell: Shell) -> Result<()> {
        let mut cmd = Cli::command();
        let mut script = Vec::new();
        generate(shell, &mut cmd, "evnx", &mut script);
        crate::utils::term::write_stdout(&script).context("writing the completion script")
    }
}
