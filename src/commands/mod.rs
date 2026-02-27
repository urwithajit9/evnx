pub mod add;
pub mod backup;
pub mod convert;
pub mod diff;
pub mod doctor;
pub mod init;
pub mod migrate;
pub mod restore;
pub mod scan;
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
            _ => {
                eprintln!("Unsupported shell: {}", shell);
                std::process::exit(1);
            }
        };

        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "evnx", &mut io::stdout());
        Ok(())
    }
}
