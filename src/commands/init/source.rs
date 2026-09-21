//! Find the variables the code actually reads.
//!
//! # Why the obvious approach does not work
//!
//! The obvious approach is to look for `env::var("X")`, `process.env.X`,
//! `os.environ["X"]`. Measured against this workspace's own server — 31 source
//! files, a 21-variable `.env.example` as ground truth — that finds **3 of 21**.
//!
//! The reason is visible at `config.rs:148`: config there goes through
//! `required_var!` / `optional_var!` macros, so the call site reads
//! `required_var!("DATABASE_URL")` and matches no `env::var` pattern. This
//! generalises — Django reads `os.environ` through `django-environ`, Node
//! projects wrap it in `config/index.js`, Laravel calls `env()` inside
//! `config/*.php`. **Direct access is the exception in any project past its
//! first week.**
//!
//! # What does work
//!
//! Scanning for the *names* rather than the access API, in the files where
//! configuration lives. Same corpus:
//!
//! | approach | recall | precision |
//! |---|---|---|
//! | `env::var("X")` only | 3/21 — 14% | 100% |
//! | every `SCREAMING_SNAKE` literal, whole tree | 21/21 — 100% | 62% |
//! | every `SCREAMING_SNAKE` literal, config files only | 20/21 — 95% | **100%** |
//!
//! Scoping is what buys the precision. Unscoped, the noise is error codes and
//! constants — `NOT_FOUND`, `VALIDATION_ERROR`, `CARGO_PKG_VERSION`,
//! `ABCDEFGHJKMNPQRSTUVWXYZ23456789` — which no token-shape rule separates from
//! real variable names. Where they *live* separates them cleanly.
//!
//! ⚠️ The one miss is `RUST_LOG`, read by `tracing_subscriber`'s `EnvFilter` and
//! named nowhere but a comment. Variables consumed entirely inside a library are
//! invisible to this in principle, as is dynamic access like
//! `process.env[key]`. This finds what the code names.

use anyhow::Result;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A variable the source appears to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceVar {
    pub name: String,
    /// `src/config.rs:171` — so a proposal can be checked.
    pub location: String,
    /// What matched: the access call, or the file's role.
    pub context: String,
}

/// Directories never worth reading: dependencies, build output, and evnx's own
/// artefacts.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".svelte-kit",
    "coverage",
];

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "rb", "php", "java", "kt", "ex",
    "exs",
];

/// What a scan saw: what to propose, and what the tree merely mentions.
pub struct Scan {
    /// High-precision: direct reads, plus names in config files. Worth proposing.
    pub found: Vec<SourceVar>,
    /// Every `SCREAMING_SNAKE` literal anywhere in the tree.
    ///
    /// ⚠️ Deliberately noisy, and used for one thing only: deciding what is
    /// **not** unused. Accusing a template entry of being dead is a claim that
    /// needs the weaker evidence, not the stronger — `RUST_LOG` is read by
    /// `EnvFilter::from_env("RUST_LOG")` in `main.rs`, which is neither a config
    /// file nor a recognised access pattern, and reporting it as unused was
    /// simply wrong.
    pub mentioned: Vec<String>,
    pub files_scanned: usize,
}

/// Scan a project for variable names its code reads.
pub fn scan(root: &Path) -> Result<Scan> {
    // Direct access, anywhere. High confidence, low recall — it is the 14%.
    let direct = Regex::new(
        r#"(?x)
        env::var \s* \( \s* "(?<a>[A-Z][A-Z0-9_]{2,})"        # Rust
        | process\.env\.(?<b>[A-Z][A-Z0-9_]{2,})               # Node, dotted
        | process\.env\[\s*["'](?<c>[A-Z][A-Z0-9_]{2,})["']    # Node, indexed
        | os\.environ(?:\.get)?[\[\(]\s*["'](?<d>[A-Z][A-Z0-9_]{2,})["']  # Python
        | os\.Getenv\( \s* "(?<e>[A-Z][A-Z0-9_]{2,})"          # Go
        | ENV\[\s*["'](?<f>[A-Z][A-Z0-9_]{2,})["']             # Ruby
        | \benv\( \s* ['"](?<g>[A-Z][A-Z0-9_]{2,})['"]         # PHP / Laravel
        "#,
    )?;

    // Any SCREAMING_SNAKE literal, but only where configuration lives. This is
    // the pass that actually works; see the module docs for the measurements.
    let literal = Regex::new(r#"["'](?<name>[A-Z][A-Z0-9_]{2,})["']"#)?;

    // Keyed by name so the first sighting wins and repeats collapse.
    let mut found: BTreeMap<String, SourceVar> = BTreeMap::new();
    let mut mentioned: std::collections::BTreeSet<String> = Default::default();
    let mut files = Vec::new();
    collect(root, &mut files);

    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let is_config = looks_like_config(path);

        for (n, line) in text.lines().enumerate() {
            let at = format!("{relative}:{}", n + 1);

            for caps in direct.captures_iter(line) {
                if let Some(name) = ["a", "b", "c", "d", "e", "f", "g"]
                    .iter()
                    .find_map(|k| caps.name(k))
                {
                    found.entry(name.as_str().to_string()).or_insert(SourceVar {
                        name: name.as_str().to_string(),
                        location: at.clone(),
                        context: "read from the environment".into(),
                    });
                }
            }

            for caps in literal.captures_iter(line) {
                let name = caps.name("name").unwrap().as_str();
                mentioned.insert(name.to_string());
                if is_config {
                    found.entry(name.to_string()).or_insert(SourceVar {
                        name: name.to_string(),
                        location: at.clone(),
                        context: "named in a config file".into(),
                    });
                }
            }
        }
    }

    Ok(Scan {
        found: found.into_values().collect(),
        mentioned: mentioned.into_iter().collect(),
        files_scanned: files.len(),
    })
}

/// Files where configuration is read, by convention across ecosystems.
///
/// ⚠️ This is the whole precision story. Applied to the entire tree the same
/// pattern picks up `NOT_FOUND` and `VALIDATION_ERROR`; applied here it picked
/// up nothing that was not a real variable.
fn looks_like_config(path: &Path) -> bool {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    const CONFIG_NAMES: &[&str] = &[
        "config",
        "configuration",
        "settings",
        "env",
        "environment",
        "constants",
        "secrets",
    ];

    CONFIG_NAMES
        .iter()
        .any(|c| name == *c || name.starts_with(&format!("{c}.")))
        || CONFIG_NAMES.contains(&parent.as_str())
}

/// Every source file under `root`, skipping dependency and build directories.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    // A hard ceiling, so a mistaken `evnx init --from-source /` cannot walk a
    // whole filesystem before anybody notices.
    const MAX_FILES: usize = 5_000;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') && name != ".config" {
                continue;
            }
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
        {
            out.push(path);
        }
    }
}

/// How the source compares to a template: what is missing, and what is unused.
///
/// ⚠️ The unused half is the thing nothing else in evnx can tell you.
/// `validate` treats `.env.example` as the definition of "required", which is
/// circular — it cannot notice an entry the code stopped reading three refactors
/// ago. Source gives it an independent ground truth.
pub struct Drift {
    /// Read by the code, absent from the template.
    pub missing: Vec<SourceVar>,
    /// In the template, named nowhere in the code.
    pub unused: Vec<String>,
    /// In both.
    pub matched: usize,
}

pub fn compare(scan: &Scan, template: &Path) -> Result<Drift> {
    let declared: Vec<String> = std::fs::read_to_string(template)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            l.split('=').next().map(|k| k.trim().to_string())
        })
        .filter(|k| !k.is_empty())
        .collect();

    let names: Vec<&str> = scan.found.iter().map(|v| v.name.as_str()).collect();

    Ok(Drift {
        missing: scan
            .found
            .iter()
            .filter(|v| !declared.contains(&v.name))
            .cloned()
            .collect(),
        // ⚠️ Judged against `mentioned`, not `found`. To call a variable unused
        // is to say the code never refers to it anywhere — a stronger claim than
        // "my high-precision pass did not pick it up", and one that needs the
        // broader evidence or it accuses variables that libraries read.
        unused: declared
            .iter()
            .filter(|d| !scan.mentioned.contains(d))
            .cloned()
            .collect(),
        matched: declared
            .iter()
            .filter(|d| names.contains(&d.as_str()))
            .count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn project(files: &[(&str, &str)]) -> TempDir {
        let d = TempDir::new().unwrap();
        for (name, body) in files {
            let path = d.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        d
    }

    fn names(scan: &Scan) -> Vec<String> {
        scan.found.iter().map(|v| v.name.clone()).collect()
    }

    #[test]
    fn direct_reads_are_found_in_any_file() {
        let d = project(&[
            (
                "src/db.rs",
                r#"let url = env::var("DATABASE_URL").unwrap();"#,
            ),
            (
                "app/handler.js",
                r#"const k = process.env.STRIPE_SECRET_KEY;"#,
            ),
            ("svc/main.py", r#"token = os.environ["API_TOKEN"]"#),
            ("cmd/serve.go", r#"port := os.Getenv("PORT_NUMBER")"#),
        ]);
        let found = names(&scan(d.path()).unwrap());
        for want in [
            "DATABASE_URL",
            "STRIPE_SECRET_KEY",
            "API_TOKEN",
            "PORT_NUMBER",
        ] {
            assert!(found.iter().any(|n| n == want), "{want} missing: {found:?}");
        }
    }

    /// ⚠️ The measurement this module exists for. Config reached through a macro
    /// matches no access pattern, and that is the normal case — 3 of 21 on this
    /// workspace's own server.
    #[test]
    fn names_in_config_files_are_found_without_a_recognised_access_pattern() {
        let d = project(&[(
            "src/config.rs",
            r#"
            let cfg = Config {
                database: required_var!("DATABASE_URL"),
                secret:   required_var!("JWT_SECRET"),
                backend:  optional!("STORAGE_BACKEND", "local"),
            };
            "#,
        )]);
        let found = names(&scan(d.path()).unwrap());
        for want in ["DATABASE_URL", "JWT_SECRET", "STORAGE_BACKEND"] {
            assert!(found.iter().any(|n| n == want), "{want} missing: {found:?}");
        }
    }

    /// ⚠️ Scoping is the whole precision story. The same pattern over the whole
    /// tree picks up error codes and constants; over config files it does not.
    #[test]
    fn constants_outside_config_files_are_not_proposed() {
        let d = project(&[(
            "src/errors.rs",
            r#"
            const NOT_FOUND: &str = "NOT_FOUND";
            const VALIDATION_ERROR: &str = "VALIDATION_ERROR";
            "#,
        )]);
        let found = names(&scan(d.path()).unwrap());
        assert!(found.is_empty(), "proposed non-variables: {found:?}");
    }

    /// ⚠️ To call a variable unused is a stronger claim than "my precise pass
    /// missed it", so it is judged against every mention in the tree. `RUST_LOG`
    /// reaches `tracing_subscriber` without appearing in a config file, and
    /// reporting it as dead was simply wrong.
    #[test]
    fn a_variable_mentioned_anywhere_is_not_called_unused() {
        let d = project(&[
            ("src/config.rs", r#"let u = required_var!("DATABASE_URL");"#),
            (
                "src/main.rs",
                r#"EnvFilter::from_env("RUST_LOG").unwrap();"#,
            ),
            (".env.example", "DATABASE_URL=\nRUST_LOG=\nGONE_FOR_GOOD=\n"),
        ]);
        let s = scan(d.path()).unwrap();
        let drift = compare(&s, &d.path().join(".env.example")).unwrap();

        assert!(
            !drift.unused.iter().any(|u| u == "RUST_LOG"),
            "RUST_LOG is read via a library: {:?}",
            drift.unused
        );
        assert!(
            drift.unused.iter().any(|u| u == "GONE_FOR_GOOD"),
            "a genuinely absent name should be reported: {:?}",
            drift.unused
        );
    }

    /// The thing no other evnx command can do: an independent ground truth.
    #[test]
    fn drift_separates_missing_from_matched() {
        let d = project(&[
            (
                "src/config.rs",
                r#"required_var!("DATABASE_URL"); required_var!("NEW_THING");"#,
            ),
            (".env.example", "DATABASE_URL=postgres://x\n"),
        ]);
        let s = scan(d.path()).unwrap();
        let drift = compare(&s, &d.path().join(".env.example")).unwrap();

        assert_eq!(drift.matched, 1);
        assert_eq!(drift.missing.len(), 1);
        assert_eq!(drift.missing[0].name, "NEW_THING");
    }

    #[test]
    fn dependency_and_build_directories_are_skipped() {
        let d = project(&[
            (
                "node_modules/pkg/config.js",
                r#"process.env.SHOULD_NOT_APPEAR"#,
            ),
            ("target/debug/config.rs", r#"env::var("ALSO_NOT")"#),
            ("src/config.rs", r#"env::var("REAL_ONE")"#),
        ]);
        let found = names(&scan(d.path()).unwrap());
        assert_eq!(found, vec!["REAL_ONE".to_string()], "{found:?}");
    }

    #[test]
    fn a_project_with_no_source_finds_nothing() {
        let d = project(&[("README.md", "# nothing here")]);
        let s = scan(d.path()).unwrap();
        assert!(s.found.is_empty());
    }

    #[test]
    fn a_location_is_reported_so_a_proposal_can_be_checked() {
        let d = project(&[("src/config.rs", "\n\nlet x = env::var(\"MY_VAR\");")]);
        let s = scan(d.path()).unwrap();
        assert_eq!(s.found[0].location, "src/config.rs:3", "{:?}", s.found[0]);
    }
}
