//! Adopt a project that already has env files (FLD-10, WP-6).
//!
//! # When
//!
//! The field test's shape: no `.env`, one or more `.env.<name>` files, and no
//! `[defaults] env_name` saying which one the project means. `init` used to
//! offer "Generate .env files?" there, never naming the files it found, and
//! wrote a contract from `.env.example` with every name required.
//!
//! A plain `.env` is not this case: `init` already keeps it and its template
//! (DQ-31), and that flow is unchanged.
//!
//! # What
//!
//! - `[defaults] env_name = "<name>"` in the project's `.evnx.toml`, so every
//!   command reads `.env.<name>` with no flag. Its own template
//!   (`.env.<name>.example`) then follows from WP-5's rule, so no `example`
//!   key is written.
//! - The contract, from that template (or the project template), with the
//!   shared `required` rule (WP-6 Q3).
//! - The `.gitignore` entries git does not already have.
//!
//! No env file and no template is changed or created (WP-6 Q1, Q2).

use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::Select;

use crate::core::env_name;
use crate::utils::ui;

use super::shared::{self, Outcome, Target, WriteOptions};

/// What is in the folder, for the decision and the "Found" line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Found {
    /// The names after `.env.`: `prod` for `.env.prod`. Sorted.
    pub named: Vec<String>,
    /// Templates present: the project's and each environment's. Sorted.
    pub templates: Vec<String>,
}

impl Found {
    pub(crate) fn in_dir(dir: &Path) -> Found {
        let mut templates = crate::core::resolve::environment_templates(dir);
        for t in env_name::TEMPLATE_SUFFIXES {
            let name = format!(".env.{t}");
            if dir.join(&name).is_file() {
                templates.push(name);
            }
        }
        templates.sort();
        Found {
            named: env_name::available(dir)
                .into_iter()
                .filter(|n| !is_local_override(n))
                .collect(),
            templates,
        }
    }

    /// `  Found   .env.prod (52 names)   .env.prod.example   .env.example`
    fn line(&self, dir: &Path) -> String {
        let mut parts: Vec<String> = self
            .named
            .iter()
            .map(|n| {
                let file = format!(".env.{n}");
                match crate::commands::add::shared::read_assignments(&dir.join(&file)) {
                    Ok(Some(map)) => {
                        format!(
                            "{file} ({})",
                            crate::utils::string::count(map.len(), "name")
                        )
                    }
                    _ => file,
                }
            })
            .collect();
        parts.extend(self.templates.iter().cloned());
        format!("  {}   {}", "Found".bold(), parts.join("   "))
    }
}

/// `.env.local` and `.env.<name>.local`: the per-machine overrides Next.js,
/// Vite and others read on top of `.env`. Not an environment a project would
/// make its default (T-I296), so never adopted.
fn is_local_override(name: &str) -> bool {
    name == "local" || name.ends_with(".local")
}

/// Whether `init` should offer adoption for `target`: see the module docs.
/// `--env-name` may choose among the files, but not name a new one.
pub(crate) fn applies(target: &Target, found: &Found) -> bool {
    !found.named.is_empty()
        && !target.root.join(".env").exists()
        && target.config.defaults.env_name.is_none()
        && target
            .env_name
            .as_deref()
            .is_none_or(|n| found.named.iter().any(|f| f == n))
}

/// What the person chose at the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    Adopt,
    Generate,
    Nothing,
}

/// The menu, in a terminal. Adopt is the default: it changes no env file.
pub(crate) fn ask(target: &Target, found: &Found) -> Result<Choice> {
    eprintln!("{}", found.line(&target.root));
    eprintln!("  This project already has env files.");
    let items = [
        "Adopt     — record them in .evnx.toml (changes no env file)",
        "Generate  — new files from a stack, as for a new project",
        "Nothing   — leave the project as it is",
    ];
    let picked = Select::with_theme(&crate::utils::prompt::MENU)
        .with_prompt("What should evnx do?")
        .items(&items)
        .default(0)
        .interact_opt()?;
    Ok(match picked {
        Some(0) => Choice::Adopt,
        Some(1) => Choice::Generate,
        _ => Choice::Nothing,
    })
}

/// The environment to adopt: `--env-name`, the only one there, or, with
/// several, the one picked in a terminal. Without a terminal, several is
/// refused with the flag that chooses.
fn which(target: &Target, found: &Found, opts: WriteOptions) -> Result<String> {
    if let Some(n) = &target.env_name {
        return Ok(n.clone());
    }
    if let [only] = found.named.as_slice() {
        return Ok(only.clone());
    }
    if opts.yes || !shared::can_ask() {
        let files: Vec<String> = found.named.iter().map(|n| format!(".env.{n}")).collect();
        return Err(crate::core::verdict::stopped(format!(
            "{} are here, and none is the default yet.\n\n\
             Choose the one this project reads by default:\n  \
             evnx init --yes --env-name {}",
            files.join(", "),
            found.named[0]
        )));
    }
    let items: Vec<String> = found.named.iter().map(|n| format!(".env.{n}")).collect();
    let picked = Select::with_theme(&crate::utils::prompt::MENU)
        .with_prompt("Which env file should commands read by default?")
        .items(&items)
        .default(0)
        .interact()?;
    Ok(found.named[picked].clone())
}

/// Adopt: see the module docs. `found` is what [`Found::in_dir`] saw.
pub(crate) fn handle(target: &Target, opts: WriteOptions, found: &Found) -> Result<Outcome> {
    let name = which(target, found, opts)?;
    // The target as it will be once `[defaults]` is set: `.env.<name>` and
    // its own template when it has one.
    let adopted = Target::new(&target.root, target.config.clone(), target.verbose)?
        .with_env_name(Some(name.clone()))?;
    let adopted = Target {
        here: target.here,
        no_contract: target.no_contract,
        ..adopted
    };
    let has_template = adopted.template_path().is_file();
    let vars = if has_template {
        shared::template_vars(&adopted)?
    } else {
        crate::schema::models::VarCollection::default()
    };

    let planned = if opts.dry_run || opts.json {
        let mut p = super::report::plan_keep(&adopted, &vars)?;
        p.defaults = Some(name.clone());
        if !has_template {
            p.contract = super::report::ContractPlan {
                action: "unchanged",
                file: p.contract.file,
                added: Vec::new(),
            };
        }
        Some(p)
    } else {
        None
    };
    if opts.dry_run {
        if !opts.json {
            eprintln!("{}", found.line(&target.root));
        }
        shared::emit(planned.as_ref(), opts, &target.root)?;
        return Ok(Outcome::DryRun);
    }

    // Every file first, then every line (RM-03).
    let config = set_default(&adopted, &name)?;
    shared::warn_template_secrets(&adopted);
    let gitignore_line = shared::update_gitignore(&target.root)?;
    let template = adopted.template_name().to_string();
    let env = adopted.env_file.clone();
    let say = || {
        if let Some(line) = gitignore_line {
            ui::say(line);
        }
        ui::success(format!(
            "adopted {env}: {} sets [defaults] env_name = \"{name}\"",
            ui::short_path(&config)
        ));
        if has_template {
            ui::info(format!(
                "{template} is its template; no env file was changed"
            ));
            shared::say_names_env_lacks(&adopted, &env);
        } else {
            ui::warning(format!(
                "{env} has no template yet, so no [vars] were recorded. \
                 Create one from it:  evnx sync --direction reverse"
            ));
        }
    };
    if has_template {
        shared::finish(
            &adopted,
            &crate::schema::models::VarCollection::default(),
            &vars,
            say,
        )?;
    } else {
        say();
    }
    shared::emit(planned.as_ref(), opts, &target.root)?;
    Ok(Outcome::KeptExisting)
}

/// `[defaults] env_name = "<name>"` in the `.evnx.toml` this project's
/// contract goes to, created when there is none. Comments, order and every
/// other key survive (`toml_edit`, as `cloud::binding` does).
fn set_default(target: &Target, name: &str) -> Result<std::path::PathBuf> {
    let path = match super::contract::plan(target)? {
        super::contract::Contract::Create(p)
        | super::contract::Contract::Record(p)
        | super::contract::Contract::Append(p) => p,
    };
    let mut doc: toml_edit::DocumentMut = match std::fs::read_to_string(&path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if !doc.contains_key("defaults") {
        let mut table = toml_edit::Table::new();
        table.decor_mut().set_prefix(
            "# The env file every command reads when none is named. Its template\n\
             # is `.env.<name>.example` when that exists. Safe to commit.\n",
        );
        doc["defaults"] = toml_edit::Item::Table(table);
    }
    doc["defaults"]["env_name"] = toml_edit::value(name);
    let written = crate::core::config::save(&path, &doc.to_string())?;
    crate::core::safe_write::report(&written);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(files: &[&str]) -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        for f in files {
            std::fs::write(d.path().join(f), "A=\n").unwrap();
        }
        d
    }

    #[test]
    fn found_lists_environments_and_templates_but_not_local_overrides() {
        let d = dir(&[
            ".env.prod",
            ".env.prod.example",
            ".env.example",
            ".env.local",
            ".env.development.local",
        ]);
        let f = Found::in_dir(d.path());
        assert_eq!(f.named, ["prod"]);
        assert_eq!(f.templates, [".env.example", ".env.prod.example"]);
    }

    #[test]
    fn a_plain_env_or_a_default_already_set_is_not_adopted() {
        let d = dir(&[".env", ".env.prod"]);
        let t = Target::new(d.path(), Default::default(), false).unwrap();
        assert!(!applies(&t, &Found::in_dir(d.path())));

        let d = dir(&[".env.prod"]);
        let mut cfg = crate::core::config::Config::default();
        cfg.defaults.env_name = Some("prod".into());
        let t = Target::new(d.path(), cfg, false).unwrap();
        assert!(!applies(&t, &Found::in_dir(d.path())));
    }

    #[test]
    fn env_name_chooses_among_the_files_but_a_new_name_is_not_adoption() {
        let d = dir(&[".env.prod", ".env.staging"]);
        let found = Found::in_dir(d.path());
        let t = Target::new(d.path(), Default::default(), false).unwrap();
        assert!(applies(&t, &found));
        let t = t.with_env_name(Some("staging".into())).unwrap();
        assert!(applies(&t, &found));
        let t = Target::new(d.path(), Default::default(), false)
            .unwrap()
            .with_env_name(Some("qa".into()))
            .unwrap();
        assert!(!applies(&t, &found));
    }
}
