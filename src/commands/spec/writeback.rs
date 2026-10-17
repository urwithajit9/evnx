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
use crate::core::spec::VarSpec;
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
    Wrote {
        names: Vec<String>,
        config: PathBuf,
        /// `config` is a symlink and this file was written instead (DEC-SP13).
        through: Option<PathBuf>,
    },
}

/// Append any newly-introduced variables to the project's contract.
///
/// Returns what it did rather than printing, so the decision about how loudly to
/// say it stays with the command the user actually ran.
///
/// ⚠️ The config in force for `start` — the one `config::load` finds at or
/// above it, which is also where `spec init` writes (SH-LOC). It never creates
/// one: a project that has not opted in is left alone.
pub fn record(start: &Path, vars: &VarCollection) -> Result<Outcome> {
    let loaded = config::load(start)?;
    let Some(config_path) = loaded.source else {
        return Ok(Outcome::NoConfig);
    };

    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;

    // ⚠️ Any form of `[vars]` counts, an inline table included (DEC-SP7). One
    // that cannot be extended is then refused by `config::save` with the reason,
    // rather than reported as "no [vars] section".
    if !config::has_vars_section(&text) {
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
        // The same inference and the same renderer `evnx spec init` uses, so a
        // variable's entry does not depend on which command introduced it. A
        // format the example value fails is left out (I-180).
        let g = super::infer::guess(name, &meta.example_value);
        let spec = VarSpec {
            required: Some(meta.required),
            secret: g.secret.then_some(true),
            format: g.format,
            description: meta.description.clone(),
            ..Default::default()
        };
        block.push_str(&super::render_entry(name, &spec, None));
    }

    let mut out = text;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&block);
    let written = config::save(&config_path, &out)?;

    Ok(Outcome::Wrote {
        names: new_names,
        config: config_path,
        through: written.through_link.then_some(written.target),
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
                ui::short_path(config)
            );
            eprintln!("     run `evnx spec init` to declare them");
        }
        Outcome::AlreadyDeclared => {
            if verbose {
                eprintln!("  every variable was already declared in [vars]");
            }
        }
        Outcome::Wrote {
            names,
            config,
            through,
        } => {
            // A success, not a finding: `-q` hides it (add DQ-A6).
            if ui::is_quiet() {
                return;
            }
            eprintln!(
                "  {} added {} variable(s) to [vars] in {}",
                ui::glyph::OK.green(),
                names.len(),
                ui::short_path(config)
            );
            if let Some(target) = through {
                eprintln!(
                    "     wrote through {} -> {}",
                    ui::short_path(config),
                    target.display()
                );
            }
            if verbose {
                for n in names {
                    eprintln!("     {n}");
                }
            }
        }
    }
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

    /// I-180: the writeback uses `spec init`'s inference, so a format the
    /// example value fails is not written.
    #[test]
    fn t_i180_a_format_the_example_fails_is_left_out() {
        let d = project(Some("[vars.EXISTING]\nrequired = true\n"));
        record(d.path(), &collection(&[("APP_PORT", "0", true)])).unwrap();
        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(after.contains("[vars.APP_PORT]"), "{after}");
        assert!(!after.contains("format"), "{after}");
    }

    /// DEC-SP7: an inline `vars = { … }` is a spec too. It cannot be extended,
    /// so the write is refused with the reason and the file is left alone —
    /// it used to be reported as "no [vars] section".
    #[test]
    fn t_i179_an_inline_vars_table_is_refused_not_ignored() {
        let before = "vars = { A = { required = false } }\n";
        let d = project(Some(before));
        let err = record(d.path(), &collection(&[("B", "x", true)])).unwrap_err();
        assert!(format!("{err:#}").contains("inline"), "{err:#}");
        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert_eq!(after, before);
    }

    /// SH-LOC: from below the project root, the root's config is the one.
    #[test]
    fn t_i177_a_subdirectory_records_into_the_root_config() {
        let d = project(Some("[vars.A]\nrequired = true\n"));
        let sub = d.path().join("apps/web");
        std::fs::create_dir_all(&sub).unwrap();
        let out = record(&sub, &collection(&[("B", "x", true)])).unwrap();
        assert!(matches!(out, Outcome::Wrote { .. }), "{out:?}");
        assert!(!sub.join(".evnx.toml").exists());
        let after = std::fs::read_to_string(d.path().join(".evnx.toml")).unwrap();
        assert!(after.contains("[vars.B]"), "{after}");
    }

    /// DEC-SP13: a symlinked config is written through, and the outcome says
    /// where.
    #[cfg(unix)]
    #[test]
    fn t_sp13_a_symlinked_config_is_written_through() {
        let d = project(None);
        std::fs::write(d.path().join("real.toml"), "[vars.A]\nrequired = true\n").unwrap();
        std::os::unix::fs::symlink("real.toml", d.path().join(".evnx.toml")).unwrap();
        let out = record(d.path(), &collection(&[("B", "x", true)])).unwrap();
        match out {
            Outcome::Wrote { through, .. } => {
                assert_eq!(through, Some(d.path().join("real.toml")))
            }
            other => panic!("expected Wrote, got {other:?}"),
        }
        assert!(std::fs::symlink_metadata(d.path().join(".evnx.toml"))
            .unwrap()
            .file_type()
            .is_symlink());
        let real = std::fs::read_to_string(d.path().join("real.toml")).unwrap();
        assert!(real.contains("[vars.B]"), "{real}");
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
