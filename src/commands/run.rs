//! `evnx run` (RM-43): a local `.env` into a command's environment, the way
//! `evnx cloud run` injects a vault. The command never reads the file itself,
//! and with masking its output carries `***` for the secrets.

use anyhow::{anyhow, Result};

use crate::core::inject;
use crate::core::Parser;

/// Read `env`, then run `command` with its variables. Exits with the
/// command's code; returns only on an error before it starts.
pub fn run(
    env: &str,
    include: Option<&[String]>,
    exclude: Option<&[String]>,
    allow_unsafe_names: bool,
    mask: bool,
    verbose: bool,
    command: &[String],
) -> Result<()> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow!("no command given. Usage: evnx run -- <cmd>"))?;
    let file = Parser::default().parse_file_or_hint(env, &Parser::missing_env_hint(env))?;
    let mut vars = file.vars;

    let before = vars.len();
    vars.retain(|name, _| crate::core::glob::admits(name, include, exclude));
    if vars.is_empty() && before > 0 {
        return Err(anyhow!(crate::core::glob::nothing_left(before, "inject")));
    }
    if verbose {
        // Names only: a value here would be in the scrollback.
        eprintln!(
            "  injecting {} variable(s) from {env}: {}",
            vars.len(),
            vars.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    inject::check_names(&vars, allow_unsafe_names, env, &format!("edit {env}"))?;

    let masker = mask.then(|| inject::Masker::new(&vars)).flatten();
    let status = inject::spawn_with_env(program, args, &vars, masker.as_ref());
    for value in vars.values_mut() {
        zeroize::Zeroize::zeroize(value);
    }
    let status = status.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "no such command: {program}\n\
                 \x20 `evnx run` runs the command after `--`, not through a shell; \
                 pipelines and builtins need one: evnx run -- sh -c 'a | b'"
            )
        } else {
            anyhow::Error::new(e).context(format!("failed to run {program}"))
        }
    })?;
    inject::exit_as(status)
}
