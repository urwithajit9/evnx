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
pub(crate) const SKIP_DIRS: &[&str] = &[
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
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "py", "go", "rb", "php", "java",
    "kt", "ex", "exs", "vue", "svelte", "astro",
];

/// Component files: scanned for direct access (`import.meta.env.X` in a
/// `<script>` block) but never for the config-file literal pass.
///
/// ⚠️ That pass is the whole precision story (see the module docs), measured on
/// ordinary source files. A `.vue` or `.svelte` file is mostly markup, where a
/// quoted `"SUBMIT"` or `'PRIMARY'` is a label, not a variable; letting
/// `config.vue` in would add noise the measurement never covered (I-005).
const DIRECT_ONLY_EXTENSIONS: &[&str] = &["vue", "svelte", "astro"];

/// A hard ceiling, so a mistaken `evnx init --from-source /` cannot walk a whole
/// filesystem before anybody notices.
const MAX_FILES: usize = 5_000;

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
    /// Directory symlinks not followed (N-05): a link back up the tree used to
    /// be walked until the OS refused, so one real file was "scanned" 41 times,
    /// and a link out of the project scanned somebody else's files.
    pub symlinked_dirs_skipped: usize,
    /// The walk stopped at [`MAX_FILES`]. Said out loud, because a partial scan
    /// reported as complete would call variables "unused" that it never saw.
    pub truncated: bool,
}

/// Values `import.meta.env` carries that the framework fills itself (I-246):
/// Vite's built-ins, and Astro's on top of them. Never set in a `.env`.
const IMPORT_META_BUILTINS: &[&str] = &[
    "MODE",
    "BASE_URL",
    "PROD",
    "DEV",
    "SSR",
    "SITE",
    "ASSETS_PREFIX",
];

/// Scan a project for variable names its code reads.
pub fn scan(root: &Path) -> Result<Scan> {
    // Direct access, anywhere. High confidence, low recall — it is the 14%.
    let direct = Regex::new(
        r#"(?x)
        env::var(?:_os)? \s* \( \s* "(?<a>[A-Z][A-Z0-9_]{2,})"  # Rust
        | process\.env\.(?<b>[A-Z][A-Z0-9_]{2,})               # Node, dotted
        | process\.env\[\s*["'](?<c>[A-Z][A-Z0-9_]{2,})["']    # Node, indexed
        | os\.environ(?:\.get)?[\[\(]\s*["'](?<d>[A-Z][A-Z0-9_]{2,})["']  # Python
        | os\.Getenv\( \s* "(?<e>[A-Z][A-Z0-9_]{2,})"          # Go
        | ENV\[\s*["'](?<f>[A-Z][A-Z0-9_]{2,})["']             # Ruby
        | \benv\( \s* ['"](?<g>[A-Z][A-Z0-9_]{2,})['"]         # PHP / Laravel
        | import\.meta\.env\.(?<h>[A-Z][A-Z0-9_]{2,})          # Vite, Astro, SvelteKit
        | import\.meta\.env\[\s*["'](?<i>[A-Z][A-Z0-9_]{2,})["'] # ... indexed
        | Deno\.env\.get\( \s* ["'](?<j>[A-Z][A-Z0-9_]{2,})["']  # Deno
        | Bun\.env\.(?<k>[A-Z][A-Z0-9_]{2,})                    # Bun, dotted
        | Bun\.env\[\s*["'](?<l>[A-Z][A-Z0-9_]{2,})["']        # Bun, indexed
        | \bgetenv\( \s* ["'](?<m>[A-Z][A-Z0-9_]{2,})["']       # Python os.getenv, PHP, C
        | ENV\.fetch\( \s* ["'](?<n>[A-Z][A-Z0-9_]{2,})["']     # Ruby
        | \b(?:option_)?env! \s* \( \s* "(?<o>[A-Z][A-Z0-9_]{2,})" # Rust, at compile time
        "#,
    )?;

    // Any SCREAMING_SNAKE literal, but only where configuration lives. This is
    // the pass that actually works; see the module docs for the measurements.
    let literal = Regex::new(r#"["'](?<name>[A-Z][A-Z0-9_]{2,})["']"#)?;

    // Reads a line at a time cannot see (I-236, DQ-22): read over the whole
    // file, reported at the line the name is on.
    let statements = Statements::new()?;

    // Keyed by name so the first sighting wins and repeats collapse.
    let mut found: BTreeMap<String, SourceVar> = BTreeMap::new();
    let mut mentioned: std::collections::BTreeSet<String> = Default::default();
    let mut walk = Walk::default();
    collect(root, &mut walk);
    let files = &walk.files;

    for path in files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let direct_only = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| DIRECT_ONLY_EXTENSIONS.contains(&e));
        let is_config = looks_like_config(path) && !direct_only;

        for (n, line) in text.lines().enumerate() {
            let at = format!("{relative}:{}", n + 1);

            for caps in direct.captures_iter(line) {
                // The one group that matched: each alternative has its own.
                if let Some(name) = caps.iter().skip(1).flatten().next() {
                    // I-239: a name the code reads is a name the code mentions.
                    // Without this, `process.env.X` — no quotes, so the literal
                    // pass never sees it — was "named nowhere in the source".
                    mentioned.insert(name.as_str().to_string());
                    // I-246: the framework fills these; nobody sets them. Still
                    // "mentioned", so a template that lists one is not called
                    // unused — only never proposed.
                    let via_import_meta = caps.name("h").is_some() || caps.name("i").is_some();
                    if via_import_meta && IMPORT_META_BUILTINS.contains(&name.as_str()) {
                        continue;
                    }
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

        // After the file's own lines, so a name a line already found keeps
        // that location; before the next file, so file order still decides.
        for read in statements.reads(&text) {
            mentioned.insert(read.name.clone());
            if read.import_meta && IMPORT_META_BUILTINS.contains(&read.name.as_str()) {
                continue;
            }
            let line = text[..read.offset].matches('\n').count() + 1;
            found.entry(read.name.clone()).or_insert(SourceVar {
                location: format!("{relative}:{line}"),
                name: read.name,
                context: "read from the environment".into(),
            });
        }
    }

    Ok(Scan {
        found: found.into_values().collect(),
        mentioned: mentioned.into_iter().collect(),
        files_scanned: files.len(),
        symlinked_dirs_skipped: walk.symlinked_dirs_skipped,
        truncated: walk.truncated,
    })
}

/// A read found over a whole file rather than one line.
struct Read {
    name: String,
    /// Byte offset of the name in the file, for its line number.
    offset: usize,
    /// Through `import.meta.env`, where the framework's built-ins live.
    import_meta: bool,
}

/// The reads a line-at-a-time match cannot see (I-236, DQ-22).
///
/// ⚠️ `const { API_KEY, DB_URL: db, PORT = 3000 } = process.env` names its
/// variables before the object they come from, often over several lines, and
/// `process\n  .env.X` splits one member access across two. Neither was
/// found, so `--from-source` proposed nothing for code written that way.
/// Destructuring counts only from `process.env`, `import.meta.env` and
/// `Bun.env` — a `{ A } = settings` is not a read — and a rest element
/// (`...rest`) or a nested pattern names no variable.
struct Statements {
    destructure: Regex,
    split: Regex,
    key: Regex,
}

impl Statements {
    fn new() -> Result<Statements> {
        Ok(Statements {
            // `{ … }`, an optional TypeScript annotation, `=`, the object.
            destructure: Regex::new(
                r"(?x)
                \{ (?<body>[^{}]*) \}
                \s* (?: : [^=;{}]* )?
                = \s* (?<src> process\s*\.\s*env | import\s*\.\s*meta\s*\.\s*env | Bun\s*\.\s*env ) \b
                ",
            )?,
            // A dotted read with whitespace (a line break) around a dot, or
            // optional chaining (`process.env?.X`).
            split: Regex::new(
                r"(?x)
                \b (?<src> process\s*\.\s*env | import\s*\.\s*meta\s*\.\s*env | Bun\s*\.\s*env )
                \s* \?? \. \s* (?<name>[A-Z][A-Z0-9_]{2,}) \b
                ",
            )?,
            // One element of the pattern: its key, quoted or not, before any
            // `: rename` or `= default`.
            key: Regex::new(r#"^\s*["']?(?<name>[A-Z][A-Z0-9_]{2,})["']?\s*(?:[:=]|$)"#)?,
        })
    }

    fn reads(&self, text: &str) -> Vec<Read> {
        let mut out = Vec::new();
        for caps in self.destructure.captures_iter(text) {
            let body = caps.name("body").expect("group in the pattern");
            let import_meta = caps["src"].contains("meta");
            let mut at = body.start();
            for element in body.as_str().split(',') {
                if let Some(key) = self.key.captures(element) {
                    let name = key.name("name").expect("group in the pattern");
                    out.push(Read {
                        name: name.as_str().to_string(),
                        offset: at + name.start(),
                        import_meta,
                    });
                }
                at += element.len() + 1;
            }
        }
        for caps in self.split.captures_iter(text) {
            let name = caps.name("name").expect("group in the pattern");
            out.push(Read {
                name: name.as_str().to_string(),
                offset: name.start(),
                import_meta: caps["src"].contains("meta"),
            });
        }
        out
    }
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

/// What a walk of the project found.
#[derive(Default)]
struct Walk {
    files: Vec<PathBuf>,
    symlinked_dirs_skipped: usize,
    truncated: bool,
}

/// Every source file under `root`, skipping dependency and build directories.
///
/// ⚠️ Directory symlinks are not followed (N-05). `path.is_dir()` follows them,
/// so `src/loop -> ..` was walked again and again until the OS gave up, and a
/// link to a directory outside the project was scanned as if it were inside it.
/// `DirEntry::file_type` does not follow, which is the distinction needed.
/// Symlinked *files* are still read: they cannot loop.
fn collect(dir: &Path, walk: &mut Walk) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // ⚠️ Name order, not the order the filesystem returns (it differs between
    // machines). It decides which file a name is reported at ("first sighting
    // wins") and which files survive `MAX_FILES` (I-243, G14).
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if walk.files.len() >= MAX_FILES {
            walk.truncated = true;
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_symlink() && path.is_dir() {
            walk.symlinked_dirs_skipped += 1;
        } else if kind.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') && name != ".config" {
                continue;
            }
            collect(&path, walk);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
        {
            walk.files.push(path);
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

    /// I-243 (G14): "first sighting wins", so the walk order decides which
    /// file a variable is reported at. It used to be the order the filesystem
    /// returned, which differs between machines; now it is name order.
    #[test]
    fn t_i243_location_is_the_first_file_by_name() {
        let order = [
            "m07.py", "m02.py", "m11.py", "m05.py", "m00.py", "m09.py", "m04.py", "m10.py",
            "m01.py", "m06.py", "m03.py", "m08.py",
        ];
        let files: Vec<(&str, &str)> = order
            .iter()
            .map(|n| (*n, r#"token = os.environ["API_TOKEN"]"#))
            .collect();
        let d = project(&files);
        let s = scan(d.path()).unwrap();
        let v = s.found.iter().find(|v| v.name == "API_TOKEN").unwrap();
        assert_eq!(v.location, "m00.py:1");
    }

    fn location_of<'a>(scan: &'a Scan, name: &str) -> &'a str {
        scan.found
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.location.as_str())
            .unwrap_or_else(|| panic!("{name} not found in {:?}", names(scan)))
    }

    /// I-236 (T-INIT-26, DQ-22): a destructured read, plain, renamed or
    /// defaulted, from each of the three objects; a rest element names
    /// nothing.
    #[test]
    fn t_i236_destructured_reads_are_found() {
        let d = project(&[
            (
                "a.js",
                "const { API_KEY, DB_URL: db, PORT = 3000, ...rest } = process.env;\n",
            ),
            ("b.ts", "const { 'QUOTED_KEY': q } = import.meta.env;\n"),
            ("c.ts", "let { BUN_ONE } = Bun.env;\n"),
        ]);
        let s = scan(d.path()).unwrap();
        for name in ["API_KEY", "DB_URL", "PORT", "QUOTED_KEY", "BUN_ONE"] {
            assert!(
                names(&s).contains(&name.to_string()),
                "{name}: {:?}",
                names(&s)
            );
        }
        assert_eq!(s.found.len(), 5, "{:?}", names(&s));
    }

    /// I-236: a pattern over several lines, and a TypeScript annotation; each
    /// name is reported at its own line.
    #[test]
    fn t_i236_a_destructuring_over_lines_reports_each_line() {
        let d = project(&[(
            "env.ts",
            "// config\nconst {\n  FIRST_ONE,\n  SECOND_ONE: second,\n}: Record<string, string> = process.env;\n",
        )]);
        let s = scan(d.path()).unwrap();
        assert_eq!(location_of(&s, "FIRST_ONE"), "env.ts:3");
        assert_eq!(location_of(&s, "SECOND_ONE"), "env.ts:4");
    }

    /// I-236: a member access split across lines, and optional chaining.
    #[test]
    fn t_i236_a_split_member_access_is_found() {
        let d = project(&[(
            "a.js",
            "const a = process\n  .env\n  .SPLIT_LINE;\nconst b = process.env?.OPTIONAL_ONE;\n",
        )]);
        let s = scan(d.path()).unwrap();
        assert_eq!(location_of(&s, "SPLIT_LINE"), "a.js:3");
        assert_eq!(location_of(&s, "OPTIONAL_ONE"), "a.js:4");
    }

    /// I-236: the access forms other languages use.
    #[test]
    fn t_i236_other_languages_are_found() {
        let d = project(&[
            ("a.py", "import os\nv = os.getenv(\"PY_GETENV\", \"x\")\n"),
            ("b.rb", "k = ENV.fetch(\"RUBY_FETCH\") { 'x' }\n"),
            ("c.php", "<?php $k = getenv('PHP_GETENV');\n"),
            ("d.rs", "const K: &str = env!(\"RUST_ENV\");\nconst O: Option<&str> = option_env!(\"RUST_OPTION_ENV\");\nlet v = std::env::var_os(\"RUST_VAR_OS\");\n"),
        ]);
        let s = scan(d.path()).unwrap();
        for name in [
            "PY_GETENV",
            "RUBY_FETCH",
            "PHP_GETENV",
            "RUST_ENV",
            "RUST_OPTION_ENV",
            "RUST_VAR_OS",
        ] {
            assert!(
                names(&s).contains(&name.to_string()),
                "{name}: {:?}",
                names(&s)
            );
        }
    }

    /// I-236: a destructuring from another object is not a read, and the
    /// framework's built-ins destructured from `import.meta.env` are not
    /// proposed (I-246) — but still count as mentioned.
    #[test]
    fn t_i236_only_env_objects_and_no_builtins() {
        let d = project(&[
            ("a.js", "const { NOT_ENV } = settings;\n"),
            (
                "b.ts",
                "const { MODE, DEV, VITE_REAL } = import.meta.env;\n",
            ),
        ]);
        let s = scan(d.path()).unwrap();
        assert_eq!(names(&s), vec!["VITE_REAL".to_string()]);
        assert!(s.mentioned.contains(&"MODE".to_string()));
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

    /// I-246: `import.meta.env` carries values the framework fills itself —
    /// Vite's `MODE`, `BASE_URL`, `PROD`, `DEV`, `SSR` and Astro's `SITE`,
    /// `ASSETS_PREFIX`. Nobody sets them, so they are never proposed; an Astro
    /// project got `DEV=` in its template. A real variable read the same way
    /// still is.
    #[test]
    fn t_i246_framework_builtins_are_not_proposed() {
        let d = project(&[(
            "src/pages/index.astro",
            "if (import.meta.env.DEV) {}\nconst m = import.meta.env.MODE;\n\
             const p = import.meta.env.PROD || import.meta.env.SSR;\n\
             const b = import.meta.env.BASE_URL + import.meta.env['SITE'];\n\
             const a = import.meta.env.ASSETS_PREFIX;\n\
             const k = import.meta.env.PUBLIC_API_URL;\n",
        )]);
        let s = scan(d.path()).unwrap();
        let found: Vec<&str> = s.found.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(found, ["PUBLIC_API_URL"], "{found:?}");
    }

    /// I-239 (T-INIT-29): a variable the code reads directly — and that the
    /// template declares — was listed as *already in the template* and as
    /// "named nowhere in the source" in the same report. Direct reads went
    /// into `found` only, and `unused` is judged against `mentioned`.
    #[test]
    fn t_i239_a_directly_read_variable_is_never_called_unused() {
        let d = project(&[
            ("web/app.js", "const a = process.env.DIRECT_ONE;"),
            ("web/vite.ts", "const b = import.meta.env.VITE_TWO;"),
            ("app/main.py", "c = os.environ.get('PY_THREE')"),
            (
                ".env.example",
                "DIRECT_ONE=\nVITE_TWO=\nPY_THREE=\nGONE_FOR_GOOD=\n",
            ),
        ]);
        let s = scan(d.path()).unwrap();
        let drift = compare(&s, &d.path().join(".env.example")).unwrap();
        for read in ["DIRECT_ONE", "VITE_TWO", "PY_THREE"] {
            assert!(
                !drift.unused.iter().any(|u| u == read),
                "{read} is read by the code: {:?}",
                drift.unused
            );
        }
        assert_eq!(drift.unused, ["GONE_FOR_GOOD"]);
        assert_eq!(drift.matched, 3);
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

    /// ⚠️ **The expectation is built with `join`, not written as a string.**
    ///
    /// `location` is `path.strip_prefix(root).display()` plus a line number, so
    /// its separator is the platform's: `src/config.rs:3` on Unix and
    /// `src\config.rs:3` on Windows. Both are right — this value is **display
    /// only**, printed by `init/components.rs` and read by a person, and a
    /// Windows user should see a Windows path.
    ///
    /// ⛔ So the hardcoded `"src/config.rs:3"` was the bug, not the output. It
    /// was the second of two failures on the first Windows CI run this repo has
    /// ever had, and the only one that was not already known.
    #[test]
    fn a_location_is_reported_so_a_proposal_can_be_checked() {
        let d = project(&[("src/config.rs", "\n\nlet x = env::var(\"MY_VAR\");")]);
        let s = scan(d.path()).unwrap();
        let expected = format!(
            "{}:3",
            std::path::Path::new("src").join("config.rs").display()
        );
        assert_eq!(s.found[0].location, expected, "{:?}", s.found[0]);
    }

    /// T-INIT-03 (I-005): the access forms Vite, Astro, SvelteKit, Deno and Bun
    /// use, found in an ordinary (non-config) file.
    #[test]
    fn modern_runtime_access_is_found_in_any_file() {
        let d = project(&[(
            "src/app.ts",
            r#"
            const a = import.meta.env.VITE_DOTTED;
            const b = import.meta.env["VITE_INDEXED"];
            const c = import.meta.env['VITE_SINGLE'];
            const e = Deno.env.get("DENO_VAR");
            const f = Bun.env.BUN_DOTTED;
            const g = Bun.env["BUN_INDEXED"];
            "#,
        )]);
        let found = names(&scan(d.path()).unwrap());
        for want in [
            "VITE_DOTTED",
            "VITE_INDEXED",
            "VITE_SINGLE",
            "DENO_VAR",
            "BUN_DOTTED",
            "BUN_INDEXED",
        ] {
            assert!(
                found.contains(&want.to_string()),
                "{want} missing: {found:?}"
            );
        }
    }

    /// Component files are read for direct access only: a quoted label in the
    /// markup of `config.vue` is not a variable.
    #[test]
    fn component_files_contribute_direct_reads_only() {
        let d = project(&[
            (
                "src/config.vue",
                "<script>const x = import.meta.env.VITE_IN_VUE</script>\n<b title=\"SUBMIT_FORM\"/>",
            ),
            ("src/App.svelte", "<script>const y = import.meta.env.PUBLIC_IN_SVELTE</script>"),
            ("src/Page.astro", "---\nconst z = import.meta.env.PUBLIC_IN_ASTRO\n---"),
            ("src/x.mts", "export const m = process.env.IN_MTS;"),
        ]);
        let s = scan(d.path()).unwrap();
        let found = names(&s);
        for want in [
            "VITE_IN_VUE",
            "PUBLIC_IN_SVELTE",
            "PUBLIC_IN_ASTRO",
            "IN_MTS",
        ] {
            assert!(
                found.contains(&want.to_string()),
                "{want} missing: {found:?}"
            );
        }
        assert!(!found.contains(&"SUBMIT_FORM".to_string()), "{found:?}");
        // Still counts as "mentioned", so it is never called unused.
        assert!(s.mentioned.contains(&"SUBMIT_FORM".to_string()));
    }

    /// N-05: a directory symlink is not followed, so a loop scans each real file
    /// once and a link out of the project is not walked.
    #[cfg(unix)]
    #[test]
    fn directory_symlinks_are_not_followed() {
        let outside = project(&[("leak.ts", "process.env.OUTSIDE_VAR")]);
        let d = project(&[("src/a.ts", "process.env.INSIDE_VAR")]);
        std::os::unix::fs::symlink("..", d.path().join("src/loop")).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("src/elsewhere")).unwrap();

        let s = scan(d.path()).unwrap();
        assert_eq!(s.files_scanned, 1);
        assert_eq!(s.symlinked_dirs_skipped, 2);
        assert_eq!(names(&s), vec!["INSIDE_VAR".to_string()]);
        assert!(!s.truncated);
    }

    /// The 5,000-file ceiling is reported, not silent.
    #[test]
    fn the_file_ceiling_is_reported() {
        let d = TempDir::new().unwrap();
        let src = d.path().join("src");
        fs::create_dir_all(&src).unwrap();
        for i in 0..=MAX_FILES {
            fs::write(src.join(format!("f{i}.js")), "").unwrap();
        }
        let s = scan(d.path()).unwrap();
        assert!(s.truncated);
        assert_eq!(s.files_scanned, MAX_FILES);
    }
}
