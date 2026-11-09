//! Which environment files have which names (N14-12, plan §9 phase 3).
//!
//! `validate` and `spec check` judge one file at a time, so a key `.env` and
//! `.env.production` have and `.env.staging` lacks passed both; only
//! `sync --check` and `diff` showed it. This is the shared question: for every
//! name, which environment files hold it and which do not.
//!
//! Only names are read. A file that cannot be read whole (not UTF-8, a line the
//! parser refuses) is left out rather than guessed at: a claim about a key
//! "absent from" a file evnx could not read would be a false finding.
//!
//! The question needs two files. With fewer there is nothing to compare, so
//! callers get no findings and a single-file project sees no change.

use std::collections::BTreeSet;
use std::path::Path;

use crate::core::env_name;
use crate::core::parser::{Parser, ParserConfig};
use crate::core::spec::Spec;

/// One environment file and the names it assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvNames {
    /// The file name as a person types it from its folder: `.env.staging`.
    pub file: String,
    /// The environment it is (`staging`); `None` for the plain `.env`.
    pub environment: Option<String>,
    pub names: BTreeSet<String>,
}

impl EnvNames {
    pub fn new(file: &str, names: impl IntoIterator<Item = String>) -> Self {
        EnvNames {
            file: file.to_string(),
            environment: env_name::name_of(file).map(str::to_string),
            names: names.into_iter().collect(),
        }
    }
}

/// A name that some environment files hold and others do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    pub name: String,
    /// Files that have it, in the order given.
    pub present_in: Vec<String>,
    /// Files that lack it, and are expected to have it.
    pub absent_from: Vec<String>,
}

/// Whether `file_name` (not a path) is an environment file this check covers.
pub fn is_environment_file(file_name: &str) -> bool {
    env_name::kind(file_name) == env_name::FileKind::Env
}

/// Every environment file in `dir`: `.env` first, then `.env.<name>` sorted.
/// A file that cannot be read whole is skipped. `skip` names one file to leave
/// out (the one the caller already holds).
pub fn read_dir(dir: &Path, skip: Option<&Path>) -> Vec<EnvNames> {
    let mut files = Vec::new();
    let mut candidates = vec![env_name::DEFAULT_FILE.to_string()];
    candidates.extend(
        env_name::available(dir)
            .into_iter()
            .map(|n| format!(".env.{n}")),
    );
    for file in candidates {
        let path = dir.join(&file);
        if !path.is_file() {
            continue;
        }
        if skip.is_some_and(|s| crate::core::safe_write::same_file(s, &path)) {
            continue;
        }
        if let Some(names) = read_names(&path) {
            files.push(EnvNames::new(&file, names));
        }
    }
    files
}

/// The names `path` assigns, or `None` when it cannot be read whole.
pub fn read_names(path: &Path) -> Option<BTreeSet<String>> {
    let text = String::from_utf8(std::fs::read(path).ok()?).ok()?;
    let doc = Parser::new(ParserConfig {
        allow_expansion: false,
        ..ParserConfig::default()
    })
    .parse_document(&text);
    doc.problems
        .is_empty()
        .then(|| doc.entries.into_iter().map(|e| e.key).collect())
}

/// The names `files` hold in some but not all of them, sorted by name.
///
/// With a contract, a file is *expected* to lack a name the contract declares
/// optional (`required = false`) or restricts to other environments
/// (`environments = [...]`): its author said so. An undeclared name has no such
/// say, and a declared required one that applies is expected everywhere.
///
/// `skip_absent` lets a caller drop one `(file, name)` it already reports in
/// another way (`validate`'s `missing_variable` for the file it checks).
pub fn partial(
    files: &[EnvNames],
    spec: &Spec,
    skip_absent: &dyn Fn(&EnvNames, &str) -> bool,
) -> Vec<Partial> {
    if files.len() < 2 {
        return Vec::new();
    }
    let all: BTreeSet<&String> = files.iter().flat_map(|f| f.names.iter()).collect();
    let mut out = Vec::new();
    for name in all {
        let present_in: Vec<String> = files
            .iter()
            .filter(|f| f.names.contains(name))
            .map(|f| f.file.clone())
            .collect();
        let absent_from: Vec<String> = files
            .iter()
            .filter(|f| !f.names.contains(name))
            .filter(|f| match spec.get(name) {
                Some(v) => v.is_required() && v.applies_to(f.environment.as_deref()),
                None => true,
            })
            .filter(|f| !skip_absent(f, name))
            .map(|f| f.file.clone())
            .collect();
        if !present_in.is_empty() && !absent_from.is_empty() {
            out.push(Partial {
                name: name.clone(),
                present_in,
                absent_from,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::spec::VarSpec;

    fn f(file: &str, names: &[&str]) -> EnvNames {
        EnvNames::new(file, names.iter().map(|s| s.to_string()))
    }

    fn none(_: &EnvNames, _: &str) -> bool {
        false
    }

    #[test]
    fn a_name_in_some_files_and_not_others_is_partial() {
        let files = [
            f(".env", &["A", "B"]),
            f(".env.staging", &["A"]),
            f(".env.production", &["A", "B"]),
        ];
        let p = partial(&files, &Spec::new(), &none);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "B");
        assert_eq!(p[0].present_in, [".env", ".env.production"]);
        assert_eq!(p[0].absent_from, [".env.staging"]);
    }

    #[test]
    fn one_file_has_nothing_to_compare() {
        assert!(partial(&[f(".env", &["A"])], &Spec::new(), &none).is_empty());
    }

    #[test]
    fn a_name_in_every_file_or_none_is_not_partial() {
        let files = [f(".env", &["A"]), f(".env.staging", &["A"])];
        assert!(partial(&files, &Spec::new(), &none).is_empty());
    }

    #[test]
    fn the_contract_can_say_a_file_is_expected_to_lack_a_name() {
        let files = [f(".env", &["A", "O", "P"]), f(".env.staging", &["A"])];
        let mut spec = Spec::new();
        spec.insert(
            "O".into(),
            VarSpec {
                required: Some(false),
                ..VarSpec::default()
            },
        );
        spec.insert(
            "P".into(),
            VarSpec {
                environments: Some(vec!["production".into()]),
                ..VarSpec::default()
            },
        );
        assert!(partial(&files, &spec, &none).is_empty());
        // Required and applying everywhere: expected, so reported.
        spec.insert("O".into(), VarSpec::default());
        let p = partial(&files, &spec, &none);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "O");
    }

    #[test]
    fn a_caller_can_drop_what_it_reports_another_way() {
        let files = [f(".env", &["A"]), f(".env.staging", &[])];
        assert!(partial(&files, &Spec::new(), &|file, _| file.file == ".env.staging").is_empty());
    }
}
