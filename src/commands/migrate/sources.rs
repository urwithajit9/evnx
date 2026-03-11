//! sources.rs — Load secrets from supported source systems
//!
//! # Supported sources
//!
//! | Source key      | Description                                |
//! |-----------------|--------------------------------------------|
//! | `env-file`      | Parse a `.env` file via `crate::core::Parser` |
//! | `environment`   | Read from current process environment vars |

use anyhow::{anyhow, Context, Result};
use indexmap::IndexMap;

use crate::core::Parser;

/// Load secrets from a named source.
///
/// # Arguments
///
/// * `source`   — one of `"env-file"` or `"environment"`.
/// * `file`     — path to the `.env` file (only used when `source == "env-file"`).
/// * `verbose`  — if `true`, print diagnostic messages.
pub fn load_secrets(source: &str, file: &str, verbose: bool) -> Result<IndexMap<String, String>> {
    if verbose {
        println!("Loading secrets from source='{}' file='{}'", source, file);
    }

    match source {
        "env-file" | "env" => {
            let parser = Parser::default();
            let env_file = parser
                .parse_file(file)
                .with_context(|| format!("Failed to parse '{}'", file))?;
            Ok(env_file.vars)
        }
        "environment" => {
            let mut secrets = IndexMap::new();
            for (key, value) in std::env::vars() {
                if !is_system_variable(&key) {
                    secrets.insert(key, value);
                }
            }
            Ok(secrets)
        }
        other => Err(anyhow!(
            "Unsupported source: '{}'. Valid options: env-file, environment",
            other
        )),
    }
}

/// Returns `true` for well-known operating-system variables that should not be
/// migrated to a secret manager when using the `environment` source.
pub fn is_system_variable(key: &str) -> bool {
    matches!(
        key,
        "PATH"
            | "HOME"
            | "USER"
            | "SHELL"
            | "PWD"
            | "TERM"
            | "LANG"
            | "LOGNAME"
            | "TMPDIR"
            | "TEMP"
            | "TMP"
            | "OLDPWD"
            | "SHLVL"
            | "_"
    )
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_system_variable_known() {
        assert!(is_system_variable("PATH"));
        assert!(is_system_variable("HOME"));
        assert!(is_system_variable("SHELL"));
        assert!(is_system_variable("TMPDIR"));
    }

    #[test]
    fn test_is_system_variable_unknown() {
        assert!(!is_system_variable("DATABASE_URL"));
        assert!(!is_system_variable("SECRET_KEY"));
        assert!(!is_system_variable("API_TOKEN"));
    }

    #[test]
    fn test_load_secrets_from_environment() {
        std::env::set_var("EVNX_TEST_SECRET_A", "alpha");
        std::env::set_var("EVNX_TEST_SECRET_B", "beta");

        let secrets = load_secrets("environment", "", false).unwrap();

        assert!(secrets.contains_key("EVNX_TEST_SECRET_A"));
        assert!(secrets.contains_key("EVNX_TEST_SECRET_B"));
        assert!(!secrets.contains_key("PATH")); // system var filtered out

        std::env::remove_var("EVNX_TEST_SECRET_A");
        std::env::remove_var("EVNX_TEST_SECRET_B");
    }

    #[test]
    fn test_load_secrets_unsupported_source() {
        let result = load_secrets("vault", "", false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported source"));
    }
}
