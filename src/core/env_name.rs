//! Resolving an environment *name* to the file that holds it.
//!
//! # Why this exists
//!
//! Every local command defaulted to exactly two files, `.env` and `.env.example`,
//! with no notion of a mode. The layered-`.env` convention is settled across
//! ecosystems — Next.js, Vite, Create React App, Rails, Laravel, `dotenv-flow`
//! and `direnv` all use some subset of `.env`, `.env.local`,
//! `.env.<mode>` and `.env.<mode>.local` — and evnx could not address any of it.
//!
//! The cloud side already had the vocabulary: `evnx vault create` takes an
//! environment from a closed set, and vaults are addressed as `name/environment`.
//! This brings the same words to the local side rather than inventing a second
//! set.
//!
//! # What it deliberately does not do
//!
//! **No layered loading.** Each command still operates on one named file. Merging
//! `.env` + `.env.local` + `.env.<mode>` is a *runtime loader's* job; a scanner or
//! a backup that silently merged four files would report findings against
//! something that does not exist on disk.
//!
//! **No silent fallback.** `--env-name production` when `.env.production` is
//! absent is an error, not a quiet retreat to `.env`. Operating on the wrong file
//! while reporting success is the failure mode this crate keeps having to fix.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Environment names evnx suggests, matching the closed set the server accepts
/// for a vault, plus `local` for per-machine overrides.
///
/// This is a list of *suggestions*, not a whitelist — projects legitimately use
/// `.env.prod`, `.env.dev`, `.env.ci`. An unrecognised name resolves literally;
/// what protects against a typo is that the file has to exist.
pub const SUGGESTED: &[&str] = &["development", "test", "staging", "production", "local"];

/// The default file, when no environment is named.
pub const DEFAULT_FILE: &str = ".env";

/// Resolve the file a command should operate on.
///
/// `None` yields `<dir>/.env` unchecked — each command already has its own
/// message for a missing default. A name is validated, resolved to
/// `<dir>/.env.<name>`, and **checked for existence**, because a name that
/// resolves to nothing is a typo and should say so.
pub fn resolve(dir: &Path, name: Option<&str>) -> Result<PathBuf> {
    let Some(name) = name else {
        return Ok(dir.join(DEFAULT_FILE));
    };

    let name = name.trim();
    if name.is_empty() {
        bail!(
            "--env-name cannot be empty (suggested: {})",
            SUGGESTED.join(", ")
        );
    }

    // Keep it to a single path segment. A name reaching `..` or `/` would let a
    // flag that reads as "which environment" address arbitrary files.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || name.contains("..")
    {
        bail!(
            "invalid environment name '{name}' — use letters, digits, '-' or '_' \
             (suggested: {})",
            SUGGESTED.join(", ")
        );
    }

    let path = dir.join(format!(".env.{name}"));
    if path.is_file() {
        return Ok(path);
    }

    bail!("{}", not_found_message(dir, name));
}

/// Choose between an explicit path and a named environment.
///
/// The one place `--env`, `--env-name` and `[defaults] env_name` meet, in that
/// order of precedence.
///
/// ⚠️ `explicit` is `Option` rather than a defaulted `String` for the same reason
/// the other flags became `Option`: with a clap `default_value`, "the user typed
/// `--env .env`" and "the user typed nothing" arrive as the same string. Config
/// would then beat an explicit flag — which it did, until a test caught
/// `convert --env .env` reading `.env.production` because `[defaults] env_name`
/// was set.
pub fn select(
    dir: &Path,
    explicit: Option<&str>,
    name: Option<&str>,
    configured: Option<&str>,
) -> Result<String> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    if name.is_some() {
        return Ok(resolve(dir, name)?.to_string_lossy().into_owned());
    }
    if configured.is_some() {
        return Ok(resolve(dir, configured)?.to_string_lossy().into_owned());
    }
    Ok(DEFAULT_FILE.to_owned())
}

/// Every `.env.<something>` present in `dir` that holds real values.
///
/// The `.env.example` family and `evnx backup` artefacts are left out: neither is
/// a thing you would pass to `--env-name`.
pub fn available(dir: &Path) -> Vec<String> {
    const TEMPLATES: &[&str] = &["example", "sample", "template"];

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix(".env.").map(str::to_owned))
        .filter(|n| !TEMPLATES.contains(&n.as_str()))
        .filter(|n| !n.contains("backup"))
        .collect();

    names.sort();
    names.dedup();
    names
}

/// A message that says what is missing *and* what is there.
///
/// "No such file" sends someone to `ls`. Listing the environments that do exist
/// usually answers the question in the same breath — most of the time the name
/// was `prod` and the file is `.env.production`.
fn not_found_message(dir: &Path, name: &str) -> String {
    let wanted = format!(".env.{name}");
    let present = available(dir);

    let mut msg = format!("{wanted} does not exist");

    // The common typo is an abbreviation of a real name.
    if let Some(guess) = present
        .iter()
        .chain(
            SUGGESTED
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .iter(),
        )
        .find(|candidate| candidate.starts_with(name) && candidate.as_str() != name)
    {
        msg.push_str(&format!(" — did you mean '{guess}'?"));
    }

    if present.is_empty() {
        if dir.join(DEFAULT_FILE).is_file() {
            msg.push_str(
                "\n\nThis project has only .env. Drop --env-name to use it, or create \
                 the file first.",
            );
        }
    } else {
        msg.push_str(&format!("\n\nAvailable: {}", present.join(", ")));
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn project(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for f in files {
            fs::write(dir.path().join(f), "A=1\n").unwrap();
        }
        dir
    }

    #[test]
    fn no_name_means_dot_env() {
        let dir = project(&[]);
        assert_eq!(resolve(dir.path(), None).unwrap(), dir.path().join(".env"));
    }

    #[test]
    fn a_name_resolves_to_its_file() {
        let dir = project(&[".env", ".env.production", ".env.local"]);

        for name in ["production", "local"] {
            assert_eq!(
                resolve(dir.path(), Some(name)).unwrap(),
                dir.path().join(format!(".env.{name}"))
            );
        }
    }

    /// Names outside the suggested set are fine — projects really do use
    /// `.env.prod` and `.env.ci`. Existence is what validates them.
    #[test]
    fn unsuggested_names_resolve_when_the_file_is_there() {
        let dir = project(&[".env.ci"]);
        assert_eq!(
            resolve(dir.path(), Some("ci")).unwrap(),
            dir.path().join(".env.ci")
        );
    }

    /// ⚠️ The property that matters: no silent fallback to `.env`.
    #[test]
    fn a_missing_environment_is_an_error_not_a_fallback() {
        let dir = project(&[".env", ".env.production"]);

        let err = resolve(dir.path(), Some("staging"))
            .unwrap_err()
            .to_string();
        assert!(err.contains(".env.staging does not exist"), "{err}");
        assert!(
            err.contains("production"),
            "it should list what is there: {err}"
        );
    }

    #[test]
    fn an_abbreviation_is_guessed() {
        let dir = project(&[".env.production"]);
        let err = resolve(dir.path(), Some("prod")).unwrap_err().to_string();
        assert!(err.contains("did you mean 'production'"), "{err}");
    }

    #[test]
    fn a_project_with_only_dot_env_is_told_so() {
        let dir = project(&[".env"]);
        let err = resolve(dir.path(), Some("production"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("only .env"), "{err}");
        assert!(err.contains("Drop --env-name"), "{err}");
    }

    /// A flag that reads as "which environment" must not address arbitrary files.
    #[test]
    fn traversal_and_separators_are_refused() {
        let dir = project(&[".env"]);
        for bad in ["../secrets", "..", "a/b", "a\\b", ""] {
            assert!(
                resolve(dir.path(), Some(bad)).is_err(),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn select_prefers_the_named_environment() {
        let dir = project(&[".env", ".env.production"]);

        // Nothing given: the default file.
        assert_eq!(select(dir.path(), None, None, None).unwrap(), ".env");
        // An explicit path is passed straight through.
        assert_eq!(
            select(dir.path(), Some("custom.env"), None, None).unwrap(),
            "custom.env"
        );
        // A name resolves, keeping the directory it was resolved in.
        let picked = select(dir.path(), None, Some("production"), None).unwrap();
        assert!(picked.ends_with(".env.production"), "{picked}");
    }

    /// ⚠️ Precedence: an explicit `--env` beats a configured environment.
    ///
    /// This failed before `explicit` became `Option`. With a clap
    /// `default_value`, "the user typed `--env .env`" and "the user typed
    /// nothing" arrive as the same string — so in a project with
    /// `[defaults] env_name = "production"`, `convert --env .env` read
    /// `.env.production`.
    #[test]
    fn an_explicit_path_beats_a_configured_environment() {
        let dir = project(&[".env", ".env.production"]);

        assert_eq!(
            select(dir.path(), Some(".env"), None, Some("production")).unwrap(),
            ".env"
        );
    }

    #[test]
    fn a_configured_environment_applies_when_no_flag_was_given() {
        let dir = project(&[".env", ".env.production"]);

        let picked = select(dir.path(), None, None, Some("production")).unwrap();
        assert!(picked.ends_with(".env.production"), "{picked}");
    }

    /// A configured name that resolves to nothing fails loudly, exactly as a
    /// typed one does — the project asked for it.
    #[test]
    fn select_propagates_a_bad_name() {
        let dir = project(&[".env"]);
        assert!(select(dir.path(), None, Some("nope"), None).is_err());
        assert!(select(dir.path(), None, None, Some("nope")).is_err());
    }

    #[test]
    fn available_lists_real_environments_only() {
        let dir = project(&[
            ".env",
            ".env.production",
            ".env.test",
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.backup",
        ]);

        assert_eq!(available(dir.path()), vec!["production", "test"]);
    }
}

/// The environment name a resolved path refers to, if any.
///
/// `.env.production` → `Some("production")`, and the plain `.env` → `None`
/// because it is not an environment: it is the base file every declared
/// variable applies to.
///
/// ⚠️ Derived from the path that will actually be read, not from the flag that
/// asked for it. `--env-name production` and `--env .env.production` are the
/// same run, and a spec's `environments` list has to treat them that way.
pub fn name_of(path: &str) -> Option<&str> {
    let file = std::path::Path::new(path).file_name()?.to_str()?;
    let rest = file.strip_prefix(".env.")?;
    // `.env.example` and `.env.local.bak` are not environments anyone declares
    // rules for; an empty remainder is not a name at all.
    if rest.is_empty() || rest == "example" || rest == "sample" || rest == "template" {
        return None;
    }
    // Re-borrow from `path` so the lifetime is the caller's.
    let at = path.len() - rest.len();
    Some(&path[at..])
}

#[cfg(test)]
mod name_of_tests {
    use super::name_of;

    #[test]
    fn a_dotted_suffix_is_the_environment_name() {
        assert_eq!(name_of(".env.production"), Some("production"));
        assert_eq!(name_of("./.env.staging"), Some("staging"));
        assert_eq!(name_of("/srv/app/.env.local"), Some("local"));
    }

    /// The base file is not an environment — a spec's `environments` list must
    /// not exclude anything from it.
    #[test]
    fn the_plain_env_file_has_no_name() {
        assert_eq!(name_of(".env"), None);
        assert_eq!(name_of("./.env"), None);
        assert_eq!(name_of("/srv/app/.env"), None);
    }

    /// Templates are not environments either.
    #[test]
    fn templates_are_not_environments() {
        assert_eq!(name_of(".env.example"), None);
        assert_eq!(name_of(".env.sample"), None);
        assert_eq!(name_of(".env.template"), None);
    }

    #[test]
    fn a_path_that_is_not_an_env_file_has_no_name() {
        assert_eq!(name_of("config.toml"), None);
        assert_eq!(name_of(""), None);
    }
}
