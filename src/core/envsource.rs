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
    fn the_skip_list_is_the_documented_one() {
        assert_eq!(SKIP.len(), 14);
        assert!(is_skipped("PATH") && is_skipped("_"));
        assert!(!is_skipped("DATABASE_URL"));
    }
}
