//! `evnx run` (RM-43): a local `.env` into a command's environment, the way
//! `evnx cloud run` injects a vault. The command never reads the file itself,
//! and with masking its output carries `***` for the secrets.

use anyhow::{Result, anyhow};

use crate::core::Parser;
use crate::core::inject;

/// What `evnx run` was asked for.
pub struct RunOptions<'a> {
    pub env: &'a str,
    pub include: Option<&'a [String]>,
    pub exclude: Option<&'a [String]>,
    pub allow_unsafe_names: bool,
    pub mask: bool,
    pub verbose: bool,
    pub prefer: inject::Prefer,
    pub command: &'a [String],
}

/// Read `env`, then run `command` with its variables. The result is the
/// command's code (`inject::ChildExit`) or an error before it started; with no
/// masking on Unix the command replaces evnx and this does not return.
pub fn run(options: RunOptions<'_>) -> Result<()> {
    let RunOptions {
        env,
        include,
        exclude,
        allow_unsafe_names,
        mask,
        verbose,
        prefer,
        command,
    } = options;
    let (program, args) = command.split_first().ok_or_else(|| {
        crate::core::verdict::usage("no command given. Usage: evnx run -- <cmd> [args]")
    })?;
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
    inject::warn_sensitive_names(&vars);

    // After the name check (which looks at the file's names whichever side
    // wins) and before the masker, so it holds what the child really gets.
    let shadowed = prefer.resolve(&mut vars);
    inject::report_shadowed(prefer, &shadowed, verbose);

    let masker = mask.then(|| inject::Masker::new(&vars)).flatten();
    let status = inject::run_with_env(program, args, &vars, masker.as_ref());
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
