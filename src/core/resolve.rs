//! Which env file, which template: one rule for every command (WP-5).
//!
//! # Why this exists
//!
//! Every command answered "which `.env`, which template" itself, from three
//! helpers that disagreed: `env_name::select` for the env file,
//! `Config::example_path` (validate, diff, sync, spec) and `Template::of`
//! (add, init, remove, doctor) for the template. Neither template rule looked
//! at the environment, so in a project with `.env.prod` and
//! `.env.prod.example`, `sync --env-name prod` paired `.env.prod` with the
//! shared `.env.example` and copied twenty production-only names into it
//! (FLD-4, found on a real server).
//!
//! # The rule
//!
//! Env file: `--env FILE` > `--env-name NAME` > `[defaults] env_name` > `.env`.
//!
//! Template, by the usual precedence ladder (command line > config file >
//! convention > built-in default, as git, cargo and Docker Compose read
//! theirs; owner decision D1, 2026-11-16):
//!
//! 1. `--example FILE`;
//! 2. the environment's own template (`.env.<name>.example`, `.sample`,
//!    `.template`) for a name given **on the command line**, by `--env-name`
//!    or `--env .env.<name>`;
//! 3. `[defaults] example`;
//! 4. the environment's own template for `[defaults] env_name`;
//! 5. the project template: the first of `.env.example`, `.env.sample`,
//!    `.env.template` that exists, else `.env.example`.
//!
//! A choice made at one level is never overridden by a lower one.
//!
//! # What it does not do
//!
//! It does not read either file, and it does not decide whether a missing env
//! file is an error: `validate` needs it, `add` creates it. [`not_found`] is
//! the one message every command prints when it needed the file.

use std::path::Path;

use anyhow::Result;

use super::config::{Config, Template};
use super::env_name::{self, DEFAULT_FILE};

/// Where the env file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvFrom {
    /// `--env FILE`.
    File,
    /// `--env-name NAME`.
    Name,
    /// `[defaults] env_name`.
    Configured,
    /// Nothing given: `.env`.
    Default,
}

/// Where the template came from: the rungs of the ladder in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateFrom {
    /// `--example FILE`.
    Flag,
    /// `.env.<name>.example` for a name given on the command line.
    MatchedFlag,
    /// `[defaults] example`.
    Configured,
    /// `.env.<name>.example` for `[defaults] env_name`.
    MatchedConfigured,
    /// The project template (`.env.example`, `.sample`, `.template`).
    Project,
}

/// The flags a command was given. All optional.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flags<'a> {
    pub env: Option<&'a str>,
    pub env_name: Option<&'a str>,
    pub example: Option<&'a str>,
}

/// Whether the env file must already exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// The command reads it. A missing *named* file is an error here, with the
    /// environments that do exist; a missing `.env` is left to the command,
    /// which prints [`not_found`].
    Existing,
    /// The command may create it (`add`, `init`, `restore`).
    MayCreate,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// As a person types it from the folder: `.env.prod`, or the `--env` path.
    pub env: String,
    pub env_from: EnvFrom,
    /// As a person types it from the folder.
    pub template: String,
    pub template_from: TemplateFrom,
    /// The environment in effect: from `--env-name`, the `--env` file's name,
    /// or `[defaults] env_name`.
    pub name: Option<String>,
    /// Environments that have their own template, other than the one in
    /// effect. Said when the project template is used despite them.
    pub other_templates: Vec<String>,
}

impl Resolved {
    /// Nothing was chosen: `.env` with the project template, and no
    /// environment has a template of its own.
    pub fn is_plain(&self) -> bool {
        self.env_from == EnvFrom::Default
            && self.template_from == TemplateFrom::Project
            && self.other_templates.is_empty()
    }

    /// The line commands print on stderr: `.env.prod ↔ .env.prod.example
    /// (template matched to --env-name prod)`. `None` for a plain project
    /// unless `verbose` (owner decision D3).
    pub fn line(&self, verbose: bool) -> Option<String> {
        if self.is_plain() && !verbose {
            return None;
        }
        let mut why = Vec::new();
        match self.env_from {
            EnvFrom::File => why.push("--env".to_string()),
            EnvFrom::Configured => why.push("env from [defaults] env_name".to_string()),
            EnvFrom::Name | EnvFrom::Default => {}
        }
        let name = self.name.as_deref().unwrap_or_default();
        why.push(match self.template_from {
            TemplateFrom::Flag => "template from --example".to_string(),
            TemplateFrom::MatchedFlag if self.env_from == EnvFrom::File => {
                format!("template matched to {}", self.env)
            }
            TemplateFrom::MatchedFlag => format!("template matched to --env-name {name}"),
            TemplateFrom::Configured => "template from [defaults] example".to_string(),
            TemplateFrom::MatchedConfigured => {
                format!("template matched to [defaults] env_name = \"{name}\"")
            }
            TemplateFrom::Project if self.name.is_some() => {
                format!("no .env.{name}.example, so the project template")
            }
            TemplateFrom::Project if !self.other_templates.is_empty() => format!(
                "project template; {} {} to other environments",
                self.other_templates.join(", "),
                if self.other_templates.len() == 1 {
                    "belongs"
                } else {
                    "belong"
                }
            ),
            TemplateFrom::Project => "project template".to_string(),
        });
        Some(format!(
            "  files     {} ↔ {}  ({})",
            self.env,
            self.template,
            why.join("; ")
        ))
    }
}

/// Resolve the env file and template for a command run in `dir`.
///
/// `cfg` is the project's `.evnx.toml` as loaded, with `[defaults] example`
/// already rebased to `dir` (`config::load`).
pub fn resolve(dir: &Path, flags: Flags, cfg: &Config, need: Need) -> Result<Resolved> {
    let configured_name = cfg.defaults.env_name.as_deref();

    // The env file, and the environment name each source implies.
    let (env, env_from, flag_name, config_name) = if let Some(file) = flags.env {
        let implied = env_name::name_of(file).map(str::to_string);
        (file.to_string(), EnvFrom::File, implied, None)
    } else if let Some(name) = flags.env_name {
        let file = env_file(dir, name, need)?;
        let name = env_name::name_of(&file).map(str::to_string);
        (file, EnvFrom::Name, name, None)
    } else if let Some(name) = configured_name {
        let file = env_file(dir, name, need)?;
        let name = env_name::name_of(&file).map(str::to_string);
        (file, EnvFrom::Configured, None, name)
    } else {
        (in_dir(dir, DEFAULT_FILE), EnvFrom::Default, None, None)
    };

    let (template, template_from) = if let Some(example) = flags.example {
        (example.to_string(), TemplateFrom::Flag)
    } else if let Some(own) = flag_name
        .as_deref()
        .and_then(|n| env_name::template_for(dir, n))
    {
        (in_dir(dir, &own), TemplateFrom::MatchedFlag)
    } else if let Some(example) = &cfg.defaults.example {
        (example.clone(), TemplateFrom::Configured)
    } else if let Some(own) = config_name
        .as_deref()
        .and_then(|n| env_name::template_for(dir, n))
    {
        (in_dir(dir, &own), TemplateFrom::MatchedConfigured)
    } else {
        (
            in_dir(dir, &Template::of(dir, cfg).name),
            TemplateFrom::Project,
        )
    };

    let name = flag_name.or(config_name);
    let chosen = Path::new(&template).file_name().and_then(|f| f.to_str());
    let own_prefix = name.as_ref().map(|n| format!(".env.{n}."));
    let other_templates = environment_templates(dir)
        .into_iter()
        .filter(|t| Some(t.as_str()) != chosen)
        .filter(|t| {
            own_prefix
                .as_ref()
                .is_none_or(|p| !t.starts_with(p.as_str()))
        })
        .collect();

    Ok(Resolved {
        env,
        env_from,
        template,
        template_from,
        name,
        other_templates,
    })
}

/// `.env.<name>` for a command that reads it (must exist) or writes it.
fn env_file(dir: &Path, name: &str, need: Need) -> Result<String> {
    Ok(match need {
        Need::Existing => env_name::resolve(dir, Some(name))?
            .to_string_lossy()
            .into_owned(),
        Need::MayCreate => in_dir(dir, &env_name::file_name(Some(name))?),
    })
}

/// The `.env.<name>.example|sample|template` files in `dir`, sorted.
pub fn environment_templates(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            env_name::kind(n) == env_name::FileKind::Template
                && n.strip_prefix(".env.")
                    .is_some_and(|rest| !env_name::TEMPLATE_SUFFIXES.contains(&rest))
        })
        .collect();
    out.sort();
    out
}

/// `file` inside `dir`, as a person types it: no `./` in the working folder.
fn in_dir(dir: &Path, file: &str) -> String {
    if dir == Path::new(".") || dir.as_os_str().is_empty() {
        file.to_string()
    } else {
        dir.join(file).to_string_lossy().into_owned()
    }
}

/// The one "env file not found" message (FLD-6), for `command` run in `dir`
/// where `env` was needed and is missing.
///
/// Lists the environment files that are there, the one-off flag for each, and
/// the `[defaults]` block that makes it stick. With none, the create hint.
pub fn not_found(dir: &Path, env: &str, command: &str) -> String {
    let present = env_name::available(dir);
    let mut msg = format!("{env} not found");
    if present.is_empty() {
        msg.push_str(
            ".\n\n  Create it with `evnx init`, or point at another file with --env FILE.",
        );
        return msg;
    }
    let files: Vec<String> = present.iter().map(|n| format!(".env.{n}")).collect();
    msg.push_str(&format!(".\n\n  Found: {}\n", files.join(", ")));
    let first = &present[0];
    msg.push_str(&format!(
        "  Use it once:    evnx {command} --env-name {first}\n"
    ));
    let mut block =
        format!("[defaults]\n                                    env_name = \"{first}\"");
    if let Some(own) = env_name::template_for(dir, first) {
        block.push_str(&format!(
            "\n                                    example  = \"{own}\""
        ));
    }
    msg.push_str(&format!("  Use it always:  add to .evnx.toml →  {block}"));
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

    fn cfg(env_name: Option<&str>, example: Option<&str>) -> Config {
        let mut c = Config::default();
        c.defaults.env_name = env_name.map(str::to_string);
        c.defaults.example = example.map(str::to_string);
        c
    }

    fn file(path: &str) -> &str {
        Path::new(path).file_name().unwrap().to_str().unwrap()
    }

    fn run(d: &TempDir, flags: Flags, c: &Config) -> Resolved {
        resolve(d.path(), flags, c, Need::Existing).unwrap()
    }

    /// The plain project is unchanged and prints no line (D3).
    #[test]
    fn a_plain_project_is_dot_env_and_its_template() {
        let d = project(&[".env", ".env.example"]);
        let r = run(&d, Flags::default(), &Config::default());
        assert_eq!((file(&r.env), file(&r.template)), (".env", ".env.example"));
        assert!(r.is_plain());
        assert_eq!(r.line(false), None);
        assert!(r.line(true).is_some());
    }

    /// FLD-4: the environment's own template wins over the shared one.
    #[test]
    fn fld4_env_name_pairs_with_its_own_template() {
        let d = project(&[".env.prod", ".env.prod.example", ".env.example"]);
        let flags = Flags {
            env_name: Some("prod"),
            ..Flags::default()
        };
        let r = run(&d, flags, &Config::default());
        assert_eq!(file(&r.env), ".env.prod");
        assert_eq!(file(&r.template), ".env.prod.example");
        assert_eq!(r.template_from, TemplateFrom::MatchedFlag);
        let line = r.line(false).unwrap();
        assert!(
            line.contains("template matched to --env-name prod"),
            "{line}"
        );
    }

    /// The `--env` file's own name counts as given on the command line.
    #[test]
    fn an_env_file_flag_pairs_with_its_own_template() {
        let d = project(&[".env.prod", ".env.prod.example", ".env.example"]);
        let env = d.path().join(".env.prod").to_string_lossy().into_owned();
        let flags = Flags {
            env: Some(&env),
            ..Flags::default()
        };
        let r = run(&d, flags, &Config::default());
        assert_eq!(file(&r.template), ".env.prod.example");
        assert_eq!(r.env_from, EnvFrom::File);
    }

    /// D1: the ladder. A flag-level name beats `[defaults] example`, which
    /// beats `[defaults] env_name`'s own template, which beats the project's.
    #[test]
    fn d1_the_precedence_ladder() {
        let d = project(&[
            ".env.prod",
            ".env.prod.example",
            ".env.dev",
            ".env.dev.example",
            ".env.example",
            "shared.example",
        ]);
        let shared = d
            .path()
            .join("shared.example")
            .to_string_lossy()
            .into_owned();
        let flags = |name| Flags {
            env_name: name,
            ..Flags::default()
        };

        // 1. --example beats everything.
        let r = run(
            &d,
            Flags {
                env_name: Some("prod"),
                example: Some(".env.example"),
                ..Flags::default()
            },
            &cfg(Some("dev"), Some(&shared)),
        );
        assert_eq!(
            (r.template.as_str(), r.template_from),
            (".env.example", TemplateFrom::Flag)
        );

        // 2. --env-name's own template beats [defaults] example.
        let r = run(&d, flags(Some("prod")), &cfg(Some("dev"), Some(&shared)));
        assert_eq!(file(&r.template), ".env.prod.example");

        // 3. [defaults] example beats [defaults] env_name's own template.
        let r = run(&d, flags(None), &cfg(Some("dev"), Some(&shared)));
        assert_eq!(
            (file(&r.env), file(&r.template)),
            (".env.dev", "shared.example")
        );
        assert_eq!(r.template_from, TemplateFrom::Configured);

        // 4. [defaults] env_name's own template beats the project's.
        let r = run(&d, flags(None), &cfg(Some("dev"), None));
        assert_eq!(file(&r.template), ".env.dev.example");
        assert_eq!(r.template_from, TemplateFrom::MatchedConfigured);

        // 5. A name without a template of its own: the project's, said so.
        let d = project(&[".env.qa", ".env.example"]);
        let r = run(&d, flags(Some("qa")), &Config::default());
        assert_eq!(file(&r.template), ".env.example");
        let line = r.line(false).unwrap();
        assert!(line.contains("no .env.qa.example"), "{line}");
    }

    /// The project template is used while another environment has its own:
    /// the line says so, so the pairing is never silent.
    #[test]
    fn other_environments_templates_are_mentioned() {
        let d = project(&[".env", ".env.example", ".env.prod.example"]);
        let r = run(&d, Flags::default(), &Config::default());
        assert!(!r.is_plain());
        assert_eq!(r.other_templates, vec![".env.prod.example".to_string()]);
        let line = r.line(false).unwrap();
        assert!(
            line.contains(".env.prod.example belongs to other environments"),
            "{line}"
        );
    }

    /// A command that creates the file may name one that does not exist yet.
    #[test]
    fn a_writer_may_name_a_new_environment() {
        let d = project(&[".env.example"]);
        let flags = Flags {
            env_name: Some(".env.staging"),
            ..Flags::default()
        };
        assert!(resolve(d.path(), flags, &Config::default(), Need::Existing).is_err());
        let r = resolve(d.path(), flags, &Config::default(), Need::MayCreate).unwrap();
        assert_eq!(file(&r.env), ".env.staging");
    }

    /// FLD-6: the shared message names what is there and how to use it.
    #[test]
    fn fld6_not_found_names_the_files_that_are_there() {
        let d = project(&[".env.prod", ".env.prod.example"]);
        let msg = not_found(d.path(), ".env", "sync");
        assert!(msg.contains("Found: .env.prod"), "{msg}");
        assert!(msg.contains("evnx sync --env-name prod"), "{msg}");
        assert!(msg.contains("env_name = \"prod\""), "{msg}");
        assert!(msg.contains("example  = \".env.prod.example\""), "{msg}");
        assert!(
            !msg.contains("prod.example,"),
            "a template is not an env file: {msg}"
        );

        let empty = project(&[]);
        let msg = not_found(empty.path(), ".env", "sync");
        assert!(msg.contains("evnx init"), "{msg}");
    }
}
