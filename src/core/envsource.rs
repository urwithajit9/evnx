//! Reading variables from the process environment (A-18, U6).
//!
//! ⚠️ A shell's or a CI job's environment is not a `.env`. It holds the runner's
//! own tokens (`GITHUB_TOKEN`, `ACTIONS_RUNTIME_TOKEN`, cloud credentials) next
//! to whatever the job set on purpose. `migrate --from environment` sent all of
//! it minus 14 operating-system names — 196 variables on the review machine,
//! with no filter and no list (I-103). Now [`require_include`] refuses to read
//! it without `--include`, and the caller names what it will send.

use anyhow::{bail, Result};
use indexmap::IndexMap;

/// Names never read from the environment, whatever `--include` says: the
/// operating system's and the shell's own.
pub const SKIP: &[&str] = &[
    "PATH", "HOME", "USER", "SHELL", "PWD", "TERM", "LANG", "LOGNAME", "TMPDIR", "TEMP", "TMP",
    "OLDPWD", "SHLVL", "_",
];

/// Whether `key` is on the [`SKIP`] list.
pub fn is_skipped(key: &str) -> bool {
    SKIP.contains(&key)
}

/// Refuse to read the environment unless `--include` selects from it.
pub fn require_include(include: Option<&[String]>) -> Result<()> {
    let selects = include.is_some_and(|p| p.iter().any(|p| !p.trim().is_empty()));
    if !selects {
        bail!(
            "--from environment needs --include.\n\n\
             The environment of a shell or CI job holds tokens you did not mean to \
             send — the runner's own among them. Name what to migrate:\n  \
             evnx migrate --from environment --include 'APP_*,DATABASE_URL' …"
        );
    }
    Ok(())
}

/// Every variable of the process environment except [`SKIP`].
pub fn read() -> IndexMap<String, String> {
    std::env::vars().filter(|(k, _)| !is_skipped(k)).collect()
}

/// Variable names that make a program or its runtime load or run other code.
///
/// ⚠️ `cloud run` put every vault variable into the child's environment, so a
/// vault writer could set `LD_PRELOAD` or `NODE_OPTIONS` and run code inside
/// every reader's processes (I-228, DEC-CL1). Refused unless the user passes
/// `--allow-unsafe-names`. Names with a prefix rule are below.
pub const UNSAFE_NAMES: &[&str] = &[
    "PATH",
    "BASH_ENV",
    "ENV",
    "SHELLOPTS",
    "BASHOPTS",
    "PROMPT_COMMAND",
    "PS4",
    "IFS",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "RUBYLIB",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "GIT_SSH_COMMAND",
    "GIT_EXEC_PATH",
];

/// Prefixes with the same effect: the dynamic loaders' `LD_*` (Linux) and
/// `DYLD_*` (macOS).
pub const UNSAFE_PREFIXES: &[&str] = &["LD_", "DYLD_"];

/// Whether `name` must not be injected into a child process without consent.
pub fn is_unsafe_name(name: &str) -> bool {
    UNSAFE_NAMES.contains(&name) || UNSAFE_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Whether `name` is one of evnx's own settings (`EVNX_TOKEN`, `EVNX_SERVER`,
/// `EVNX_PASSWORD`, …), which a child process must not inherit (I-227).
pub fn is_evnx_own(name: &str) -> bool {
    name.starts_with("EVNX_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_i103_include_is_required_and_must_select() {
        assert!(require_include(None).is_err());
        assert!(require_include(Some(&[])).is_err());
        assert!(require_include(Some(&[" ".into()])).is_err());
        assert!(require_include(Some(&["APP_*".into()])).is_ok());
    }

    #[test]
    fn t_i228_code_loading_names_are_unsafe() {
        for n in [
            "PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "DYLD_INSERT_LIBRARIES",
            "BASH_ENV",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "JAVA_TOOL_OPTIONS",
        ] {
            assert!(is_unsafe_name(n), "{n}");
        }
        for n in [
            "DATABASE_URL",
            "API_KEY",
            "NODE_ENV",
            "PATHS",
            "MY_PATH",
            "LDAP_URL",
        ] {
            assert!(!is_unsafe_name(n), "{n}");
        }
    }

    #[test]
    fn t_i227_evnx_settings_are_recognised() {
        assert!(is_evnx_own("EVNX_TOKEN") && is_evnx_own("EVNX_PASSWORD"));
        assert!(!is_evnx_own("EVNX") && !is_evnx_own("MY_EVNX_TOKEN"));
    }

    #[test]
    fn the_skip_list_is_the_documented_one() {
        assert_eq!(SKIP.len(), 14);
        assert!(is_skipped("PATH") && is_skipped("_"));
        assert!(!is_skipped("DATABASE_URL"));
    }
}
