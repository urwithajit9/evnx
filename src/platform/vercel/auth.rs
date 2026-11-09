//! Where the Vercel token comes from: `VERCEL_TOKEN`, or a file.
//!
//! There is deliberately **no `--token <value>` flag**: it would sit in shell
//! history and in `ps` output. The token is held as a [`SecretString`], sent
//! only in the `Authorization` header, and never printed, logged or stored.

use std::path::Path;

use anyhow::{bail, Context, Result};
use zeroize::Zeroizing;

use crate::platform::secret::SecretString;

/// The environment variable `vercel` itself reads.
pub const ENV_TOKEN: &str = "VERCEL_TOKEN";

/// The token: `--token-file` if given, else `VERCEL_TOKEN`.
///
/// `env` looks a variable up, so a test can supply one without touching the
/// process environment. A blank token (empty, or only whitespace) is no token.
pub fn resolve(
    token_file: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<SecretString> {
    if let Some(path) = token_file {
        let raw = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            let why = match e.kind() {
                std::io::ErrorKind::NotFound => "that file does not exist".to_string(),
                _ => format!("it cannot be read ({e})"),
            };
            anyhow::anyhow!("--token-file {}: {why}", path.display())
        })?);
        let token = raw.trim();
        if token.is_empty() {
            bail!("--token-file {} is empty", path.display());
        }
        check_shape(token).with_context(|| format!("--token-file {}", path.display()))?;
        return Ok(SecretString::new(token));
    }
    match env(ENV_TOKEN).map(Zeroizing::new) {
        Some(t) if !t.trim().is_empty() => {
            check_shape(t.trim()).with_context(|| format!("{ENV_TOKEN} is set but unusable"))?;
            Ok(SecretString::new(t.trim()))
        }
        _ => bail!(
            "no Vercel token.\n  \
             Set {ENV_TOKEN}, or pass --token-file <path> (a file holding only the token).\n  \
             There is no --token flag on purpose: a value on the command line lands in shell history.\n  \
             Create a token at https://vercel.com/account/tokens"
        ),
    }
}

/// A header value cannot hold a line break or control character; say so
/// without echoing the token.
fn check_shape(token: &str) -> Result<()> {
    if token.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("the token contains whitespace or control characters; it should be one line");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn the_environment_variable_is_used() {
        let t = resolve(None, |k| (k == ENV_TOKEN).then(|| " tok-123 \n".into())).unwrap();
        assert_eq!(t.expose(), "tok-123");
    }

    #[test]
    fn a_file_wins_over_the_variable() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t");
        std::fs::write(&f, "from-file\n").unwrap();
        let t = resolve(Some(&f), |_| Some("from-env".into())).unwrap();
        assert_eq!(t.expose(), "from-file");
    }

    #[test]
    fn nothing_given_names_both_ways_and_no_flag() {
        let e = resolve(None, no_env).unwrap_err().to_string();
        assert!(
            e.contains("VERCEL_TOKEN") && e.contains("--token-file"),
            "{e}"
        );
        assert!(e.contains("no --token flag"), "{e}");
    }

    #[test]
    fn a_blank_variable_is_no_token() {
        assert!(resolve(None, |_| Some("  \n".into())).is_err());
    }

    #[test]
    fn a_missing_or_empty_file_is_an_error_naming_the_path() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        let e = resolve(Some(&gone), no_env).unwrap_err().to_string();
        assert!(e.contains("gone") && e.contains("does not exist"), "{e}");
        let empty = d.path().join("empty");
        std::fs::write(&empty, "\n").unwrap();
        assert!(resolve(Some(&empty), no_env)
            .unwrap_err()
            .to_string()
            .contains("empty"));
    }

    #[test]
    fn a_token_with_whitespace_inside_is_refused_without_echoing_it() {
        let e = resolve(None, |_| Some("abc def".into()))
            .unwrap_err()
            .to_string();
        assert!(!e.contains("abc def"), "{e}");
        let e = format!(
            "{:#}",
            resolve(None, |_| Some("abc\u{7}def".into())).unwrap_err()
        );
        assert!(!e.contains("abc"), "{e}");
    }
}
