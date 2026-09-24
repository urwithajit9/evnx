pub mod add;
pub mod backup;
pub mod convert;
pub mod diff;
pub mod doctor;
pub mod init;
pub mod migrate;
pub mod restore;
pub mod scan;
pub mod spec;
pub mod sync;
pub mod template;
pub mod validate;

pub use add::run as run_add;
pub use init::run as run_init;

pub mod completions {
    use crate::cli::Cli;
    use anyhow::Result;
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    use std::io;

    pub fn run(shell: String) -> Result<()> {
        let shell = match shell.to_lowercase().as_str() {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            "powershell" => Shell::PowerShell,
            // ⚠️ `Err`, not `process::exit(1)`. Exiting here bypassed the
            // centralised mapping in `main`, which numbers any command error 2 —
            // so `completions` was the one command that answered an unusable
            // argument with 1, the code every other command reserves for a real
            // finding.
            other => {
                anyhow::bail!(
                    "unsupported shell '{other}' — expected one of: bash, zsh, fish, powershell"
                );
            }
        };

        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "evnx", &mut io::stdout());
        Ok(())
    }
}
