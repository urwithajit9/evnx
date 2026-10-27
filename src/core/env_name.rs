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
        return Ok(in_dir(dir, DEFAULT_FILE));
    };

    let name = checked(name)?;
    let path = in_dir(dir, &format!(".env.{name}"));
    if path.is_file() {
        return Ok(path);
    }

    bail!("{}", not_found_message(dir, name));
}

/// The file a command that **writes** should write: `.env`, or `.env.<name>`
/// for a valid name. Unlike [`resolve`] it does not need the file to exist —
/// `init` and `add` create it (RM-17).
pub fn file_name(name: Option<&str>) -> Result<String> {
    Ok(match name {
        None => DEFAULT_FILE.to_string(),
        Some(name) => format!(".env.{}", checked(name)?),
    })
}

/// `name`, trimmed, if it is one path segment a person could mean as an
/// environment.
fn checked(name: &str) -> Result<&str> {
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
    Ok(name)
}

/// `file` inside `dir`, written the way a person would type it: in the current
/// directory that is `.env.production`, not `./.env.production` (I-144, O-12).
/// Every message that names the file then agrees with the one for plain `.env`.
fn in_dir(dir: &Path, file: &str) -> PathBuf {
    if dir == Path::new(".") {
        PathBuf::from(file)
    } else {
        dir.join(file)
    }
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
/// The `.env.example` family and copies evnx leaves behind are left out: neither
/// is a thing you would pass to `--env-name`.
pub fn available(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| kind(n) == FileKind::Env)
        .filter_map(|n| n.strip_prefix(".env.").map(str::to_owned))
        .collect();

    names.sort();
    names.dedup();
    names
}

/// What follows `.env.` in a template: a file meant to be committed, holding
/// names and no values.
///
/// ⚠️ One list. `available`, [`name_of`], `doctor` and `scan` each kept their
/// own copy of these three names (R-doctor). `core::gitignore::ENV_ENTRIES`
/// spells them out as negations, and a test there holds it to this list.
pub const TEMPLATE_SUFFIXES: &[&str] = &["example", "sample", "template"];

/// Extensions that make a file named after `.env` structured data rather than
/// dotenv: `.env.yaml`, `.env.json`. Read as text, never parsed as
/// assignments (moved from `scan`, R-doctor DEC-D19).
pub const STRUCTURED_EXTENSIONS: &[&str] = &[
    ".yaml", ".yml", ".json", ".toml", ".ini", ".xml", ".md", ".py", ".js", ".ts",
];

/// What a file is, by its name alone (DEC-D3, DEC-D20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `.env` or `.env.<name>`: holds real values.
    Env,
    /// `.env.example`, `.env.sample`, `.env.template`.
    Template,
    /// A copy of an env file ([`is_copy`]). Plaintext or an `evnx backup`;
    /// only the content can tell ([`crate::core::envfile::is_evnx_backup`]).
    Copy,
    /// Not an env file, though the name starts with `.env`: `.envrc` (a direnv
    /// script, often committed on purpose), `.environment`, `.env-notes.md`.
    Other,
}

/// [`FileKind`] of a file name (not a path).
///
/// ⚠️ `.env` or `.env.` and something, never just "starts with `.env`".
/// `doctor` took every `.env*` name, so a committed `.envrc` was a security
/// error with exit 1, and its `layout python` line "invalid syntax" (I-173).
pub fn kind(file_name: &str) -> FileKind {
    if file_name == ".env" {
        return FileKind::Env;
    }
    if file_name == ".env~" {
        return FileKind::Copy;
    }
    match file_name.strip_prefix(".env.") {
        None | Some("") => FileKind::Other,
        Some(rest) if TEMPLATE_SUFFIXES.contains(&rest) => FileKind::Template,
        Some(rest) if is_copy(rest) => FileKind::Copy,
        Some(_) => FileKind::Env,
    }
}

/// Whether `file_name` is structured data named after `.env` (`.env.yaml`):
/// see [`STRUCTURED_EXTENSIONS`].
pub fn is_structured(file_name: &str) -> bool {
    STRUCTURED_EXTENSIONS
        .iter()
        .any(|ext| file_name.ends_with(ext))
}

/// A copy of an env file, not an environment: what follows `.env.` is one
/// evnx (or a person) names copies with (I-272).
///
/// | Suffix | Left by |
/// |---|---|
/// | `backup`, `backup.N`, `production.backup` | `evnx backup` and its rotation |
/// | `bak`, `bak.N`, `production.bak` | `validate --fix`, `restore --yes` ([`crate::core::safe_write::backup`]) |
/// | `restored`, `production.restored` | `restore`, for content that is not a `.env` |
/// | `1`, `production.2` | numbered copies |
/// | `old`, `save`, a trailing `~` | a person, or an editor |
///
/// ⚠️ Until 0.10.11 only `backup` was left out, so `--env-name`'s "Available"
/// list read `1, bak`, and `validate`'s known environments took `bak` for one.
/// `old`, `save` and `~` joined with `doctor`'s list (DEC-D20), which reads
/// these copies because a plaintext one exposes what `.env` does (I-160).
fn is_copy(name: &str) -> bool {
    name.contains("backup")
        || name.ends_with('~')
        || name.split('.').any(|part| {
            matches!(part, "bak" | "restored" | "old" | "save")
                || (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// The first candidate `name` abbreviates: `prod` → `production`.
///
/// One rule for the `--env-name` error and the contract's environment check
/// (I-042), so both read `.env.prod` the same way.
pub fn abbreviation_of<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .find(|c| c.starts_with(name) && *c != name)
}

/// The environment `name` was probably meant to be: one it abbreviates, else
/// the closest by spelling (`prodution` → `production`).
pub fn meant<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    abbreviation_of(name, candidates.iter().copied()).or_else(|| {
        crate::utils::string::closest(name, candidates.iter().map(|c| (*c, *c)))
            .first()
            .copied()
    })
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

    // An abbreviation of a real name (`prod`), else the closest spelling
    // (`prodution`): the one rule the contract's environment check uses too
    // (RM-17). Files present come first, then the suggested names.
    // The name itself is no suggestion: `.env.staging` is missing, so
    // "did you mean 'staging'" would point back at the same file.
    let mut candidates: Vec<&str> = present
        .iter()
        .map(String::as_str)
        .filter(|c| *c != name)
        .collect();
    for s in SUGGESTED {
        if *s != name && !candidates.contains(s) {
            candidates.push(s);
        }
    }
    if let Some(guess) = meant(name, &candidates) {
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

    #[test]
    fn t_i144_a_file_in_the_current_directory_has_no_dot_slash() {
        assert_eq!(
            in_dir(Path::new("."), ".env.production"),
            PathBuf::from(".env.production")
        );
        assert_eq!(
            in_dir(Path::new("app"), ".env.production"),
            PathBuf::from("app/.env.production")
        );
        assert_eq!(
            in_dir(Path::new("./app"), ".env"),
            PathBuf::from("./app/.env")
        );
    }
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

    /// RM-17: the writing side takes the same names, and refuses the same ones.
    #[test]
    fn rm17_file_name_validates_without_needing_the_file() {
        assert_eq!(file_name(None).unwrap(), ".env");
        assert_eq!(file_name(Some(" staging ")).unwrap(), ".env.staging");
        assert!(file_name(Some("../x")).is_err());
        assert!(file_name(Some("")).is_err());
    }

    /// RM-17: a misspelling gets a suggestion too, not only an abbreviation.
    #[test]
    fn rm17_a_misspelt_name_is_matched_by_spelling() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.production"), "A=1\n").unwrap();
        let err = resolve(dir.path(), Some("prodution"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("did you mean 'production'"), "{err}");
        let err = resolve(dir.path(), Some("zzzzzzzz"))
            .unwrap_err()
            .to_string();
        assert!(!err.contains("did you mean"), "{err}");
        // A suggested name that is missing is not suggested back to itself.
        let err = resolve(dir.path(), Some("staging"))
            .unwrap_err()
            .to_string();
        assert!(!err.contains("did you mean 'staging'"), "{err}");
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

    /// I-272: copies evnx leaves behind are not environments.
    #[test]
    fn t_i272_copies_are_not_environments() {
        let dir = project(&[
            ".env.production",
            ".env.bak",
            ".env.bak.2",
            ".env.production.bak",
            ".env.restored",
            ".env.1",
            ".env.production.2",
            ".env.backup.3",
            ".env.v2",
        ]);

        assert_eq!(available(dir.path()), vec!["production", "v2"]);
    }

    /// DEC-D20: one copy rule with `doctor`, so `.old`, `.save` and `~` are
    /// copies here too.
    #[test]
    fn t_dec_d20_old_and_save_are_copies_not_environments() {
        let dir = project(&[".env.production", ".env.old", ".env.production.save"]);
        assert_eq!(available(dir.path()), vec!["production"]);
    }

    /// I-173, DEC-D3: `.env` or `.env.<name>`; the rest only looks like one.
    #[test]
    fn t_i173_which_names_are_env_files() {
        use FileKind::*;
        for (name, want) in [
            (".env", Env),
            (".env.local", Env),
            (".env.production", Env),
            (".env.production.local", Env),
            (".env.yaml", Env),
            (".env.example", Template),
            (".env.sample", Template),
            (".env.template", Template),
            (".env.backup", Copy),
            (".env.backup.2", Copy),
            (".env.production.backup", Copy),
            (".env.bak", Copy),
            (".env.old", Copy),
            (".env.save", Copy),
            (".env.restored", Copy),
            (".env.1", Copy),
            (".env~", Copy),
            (".env.production~", Copy),
            (".envrc", Other),
            (".environment", Other),
            (".env-notes.md", Other),
            (".envoy.yaml", Other),
            (".env.", Other),
            ("app.env", Other),
            ("env", Other),
        ] {
            assert_eq!(kind(name), want, "{name}");
        }
    }

    #[test]
    fn structured_names_are_not_dotenv() {
        for name in [".env.yaml", ".env.yml", ".env.json", ".env.toml"] {
            assert!(is_structured(name), "{name}");
        }
        for name in [".env", ".env.local", ".env.production"] {
            assert!(!is_structured(name), "{name}");
        }
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
    if rest.is_empty() || TEMPLATE_SUFFIXES.contains(&rest) {
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
