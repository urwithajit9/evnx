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
/// Commands keep taking a single path, so this is the one place the two flags
/// meet. `--env` and `--env-name` are declared `conflicts_with` each other in the
/// CLI, so only one can arrive; if that ever changes, the name wins and the
/// explicit path is ignored — which is why they conflict rather than layer.
pub fn select(dir: &Path, explicit: &str, name: Option<&str>) -> Result<String> {
    match name {
        Some(_) => Ok(resolve(dir, name)?.to_string_lossy().into_owned()),
        None => Ok(explicit.to_owned()),
    }
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

        // No name: the explicit path is passed straight through, untouched.
        assert_eq!(select(dir.path(), ".env", None).unwrap(), ".env");
        assert_eq!(
            select(dir.path(), "custom.env", None).unwrap(),
            "custom.env"
        );

        // A name resolves, and keeps the directory it was resolved in.
        let picked = select(dir.path(), ".env", Some("production")).unwrap();
        assert!(picked.ends_with(".env.production"), "{picked}");
    }

    #[test]
    fn select_propagates_a_bad_name() {
        let dir = project(&[".env"]);
        assert!(select(dir.path(), ".env", Some("nope")).is_err());
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
