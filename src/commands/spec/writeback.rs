//! Keeping `[vars]` current when `evnx add` or `evnx init` introduces a variable.
//!
//! # The failure this prevents
//!
//! Before this, adopting a spec and then adding anything left the contract
//! describing a project that no longer existed:
//!
//! ```console
//! $ evnx add service redis --yes      # writes REDIS_PASSWORD to .env.example
//! $ grep -c REDIS .evnx.toml
//! 0
//! $ evnx validate
//! ✓ All checks passed
//! ```
//!
//! `REDIS_PASSWORD` was not required, not format-checked, and not declared
//! secret — so `evnx scan` would not treat it as one either. The contract had
//! quietly stopped covering the project, and nothing said so.
//!
//! # ⚠️ It only ever appends
//!
//! A name already in `[vars]` is left exactly as it is. Someone who wrote
//! `required = false` by hand must not have it reset by an unrelated
//! `evnx add`, and `init` staying a one-shot command is a decision this module
//! does not get to revisit.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::core::config;
use crate::schema::models::VarCollection;
use crate::utils::ui;

/// What happened, so callers can report it without re-deriving it.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No `.evnx.toml` at all — the project has not opted into anything.
    NoConfig,
    /// A config exists but declares no contract.
    NoSpec { config: PathBuf },
    /// Every name was already declared.
    AlreadyDeclared,
    /// Names appended to `[vars]`.
    Wrote { names: Vec<String>, config: PathBuf },
}

/// Append any newly-introduced variables to the project's contract.
///
/// Returns what it did rather than printing, so the decision about how loudly to
/// say it stays with the command the user actually ran.
pub fn record(start: &Path, vars: &VarCollection) -> Result<Outcome> {
    let loaded = config::load(start)?;
    let Some(config_path) = loaded.source else {
        return Ok(Outcome::NoConfig);
    };

    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;

    // ⚠️ Textual, like `spec init`'s guard. A `[vars]` section that failed to
    // parse still counts as present: appending beside it would produce a file
    // nothing can read, which is worse than declining.
    if !has_vars_section(&text) {
        return Ok(Outcome::NoSpec {
            config: config_path,
        });
    }

    let declared = loaded.config.vars;
    let mut new_names: Vec<String> = vars
        .vars
        .keys()
        .filter(|k| !declared.contains_key(*k))
        .cloned()
        .collect();
    new_names.sort();

    if new_names.is_empty() {
        return Ok(Outcome::AlreadyDeclared);
    }

    let mut block = String::new();
    for name in &new_names {
        let meta = &vars.vars[name];
        block.push_str(&format!("\n[vars.{name}]\n"));
        if let Some(d) = &meta.description {
            block.push_str(&format!("description = {}\n", super::toml_string(d)));
        }
        block.push_str(&format!("required = {}\n", meta.required));
        // The same inference `evnx spec init` uses, so a variable's entry does
        // not depend on which command happened to introduce it.
        if let Some(f) = super::infer::format_of(name, &meta.example_value) {
            block.push_str(&format!(
                "format = {}\n",
                super::toml_string(&super::format_name(&f))
            ));
        }
        if super::infer::looks_secret(name) {
            block.push_str("secret = true\n");
        }
    }

    let mut out = text;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&block);
    std::fs::write(&config_path, out)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(Outcome::Wrote {
        names: new_names,
        config: config_path,
    })
}

/// Say what happened, at the volume it deserves.
///
/// ⚠️ Silent when there is no `.evnx.toml`. The overwhelming majority of
/// projects have not adopted a spec, and telling them on every `evnx add` that a
/// feature they never opted into was skipped is how a useful notice becomes
/// noise people filter out.
///
/// A project that *has* a config but no `[vars]` is a different case: it has
/// opted into the file, so pointing at `evnx spec init` is an answer rather than
/// an advertisement. That goes to **stderr**, so it never lands in the middle of
/// a piped stdout, and it is never an error — `evnx add` succeeding is not
/// contingent on the project keeping a contract.
pub fn report(outcome: &Outcome, verbose: bool) {
    match outcome {
        Outcome::NoConfig => {}
        Outcome::NoSpec { config } => {
            eprintln!(
                "  {}  {} has no [vars] section — the new variables are not under contract",
                ui::glyph::WARN.yellow(),
                short(config)
            );
            eprintln!("     run `evnx spec init` to declare them");
        }
        Outcome::AlreadyDeclared => {
            if verbose {
                eprintln!("  every variable was already declared in [vars]");
            }
        }
        Outcome::Wrote { names, config } => {
            eprintln!(
                "  {} added {} variable(s) to [vars] in {}",
                ui::glyph::OK.green(),
                names.len(),
                short(config)
            );
            if verbose {
                for n in names {
                    eprintln!("     {n}");
                }
            }
        }
    }
}

fn short(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Is there a `[vars.…]` or `[vars]` table already?
fn has_vars_section(src: &str) -> bool {
    src.lines()
        .map(str::trim)
        .any(|l| (l.starts_with("[vars.") || l == "[vars]") && !l.starts_with('#'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::models::{VarMetadata, VarSource};
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn collection(pairs: &[(&str, &str, bool)]) -> VarCollection {
        let mut vars = HashMap::new();
        for (k, v, required) in pairs {
            vars.insert(
                k.to_string(),
                VarMetadata {
                    example_value: v.to_string(),
                    description: None,
                    category: None,
                    required: *required,
                    source: VarSource::Service("test".into()),
                },
            );
        }
        VarCollection { vars }
    }

    fn project(config: Option<&str>) -> TempDir {
        let d = TempDir::new().unwrap();
        if let Some(c) = config {
            std::fs::write(d.path().join(".evnx.toml"), c).unwrap();
        }
        // `config::find` stops at a directory containing `.git`.
        std::fs::create_dir(d.path().join(".git")).unwrap();
        d
    }

    /// A project that never opted in is left alone entirely.
    #[test]
    fn no_config_means_no_write_and_nothing_to_say() {
        let d = project(None);
        let out = record(d.path(), &collection(&[("A", "1", true)])).unwrap();
        assert_eq!(out, Outcome::NoConfig);
    }

    /// A config without a contract gets a pointer, not a file edit.
    #[test]
    fn a_config_without_vars_is_not_given_one() {
        let d = project(Some("[scan]\nseverity = \"high\"\n"));
        let out = record(d.path(), &collection(&[("A", "1", true)])).unwrap();
        assert!(matches!(out, Outcome::NoSpec { .. }), "{out:?}");

        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(!after.contains("[vars"), "a spec was created uninvited");
    }

    #[test]
    fn new_variables_are_appended_with_inferred_fields() {
        let d = project(Some("[vars.EXISTING]\nrequired = true\n"));
        let out = record(
            d.path(),
            &collection(&[
                ("REDIS_URL", "redis://localhost:6379/0", true),
                ("REDIS_PASSWORD", "hunter2", false),
                ("REDIS_PORT", "6379", true),
            ]),
        )
        .unwrap();

        match out {
            Outcome::Wrote { ref names, .. } => {
                assert_eq!(names, &["REDIS_PASSWORD", "REDIS_PORT", "REDIS_URL"])
            }
            other => panic!("expected Wrote, got {other:?}"),
        }

        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(after.contains("[vars.REDIS_URL]"), "{after}");
        assert!(after.contains("format = \"url\""), "{after}");
        assert!(after.contains("format = \"port\""), "{after}");
        // The inference `spec init` uses, so an entry does not depend on which
        // command introduced it.
        assert!(after.contains("secret = true"), "REDIS_PASSWORD not marked");
        assert!(after.contains("required = false"), "{after}");

        // And it still parses.
        let reloaded = config::load(d.path()).unwrap();
        assert_eq!(reloaded.config.vars.len(), 4);
    }

    /// ⚠️ A hand-edited entry must survive an unrelated `evnx add`.
    #[test]
    fn an_already_declared_variable_is_never_rewritten() {
        let d = project(Some(
            "[vars.REDIS_URL]\nrequired = false\ndescription = \"mine\"\n",
        ));
        let out = record(d.path(), &collection(&[("REDIS_URL", "redis://h", true)])).unwrap();
        assert_eq!(out, Outcome::AlreadyDeclared);

        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(
            after.contains("required = false"),
            "hand edit lost:\n{after}"
        );
        assert_eq!(after.matches("[vars.REDIS_URL]").count(), 1);
    }

    #[test]
    fn everything_else_in_the_config_survives() {
        let before = "# keep me\n[scan]\nseverity = \"high\"\n\n[vars.A]\nrequired = true\n";
        let d = project(Some(before));
        record(d.path(), &collection(&[("B", "2", true)])).unwrap();
        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(
            after.starts_with(before),
            "existing content changed:\n{after}"
        );
    }
}
