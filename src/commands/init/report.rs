//! What one run of init writes, worked out before it writes: printed instead
//! of writing under `--dry-run`, and as the result under `--json` (IN-16,
//! DQ-21).
//!
//! ⚠️ Worked out from the same state and the same rules the writes follow —
//! the template rule (DQ-28), the gitignore entries git does not already have,
//! `.env` created or caught up (DQ-29), the contract created, appended or
//! recorded (IN-15) — so a dry run lists what the real run then does
//! (T-INIT-35). The JSON has `add --json`'s fields plus `template`,
//! `gitignore`, `contract` and `notices`, which `add` prints too (T-INIT-36).

use std::path::Path;

use anyhow::Result;

use crate::core::gitignore;
use crate::schema::models::VarCollection;
use crate::utils::ui;

use super::contract::Contract;
use super::shared::Target;

/// What happens to the template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateAction {
    Create,
    Replace,
    /// Names appended (`--from-source` beside a template).
    Append,
    /// Kept as it is (a fresh clone, DQ-31; nothing new from the source).
    Keep,
}

/// What happens to `.env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvAction {
    /// Written from the generated template.
    Create,
    /// Copied from the project's template.
    Copy,
    /// Given the names it lacks (DQ-29).
    Append,
    Unchanged,
}

/// The contract half: which file, what happens to it, which names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractPlan {
    pub action: &'static str,
    pub file: Option<std::path::PathBuf>,
    pub added: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Planned {
    pub template: String,
    pub template_action: TemplateAction,
    pub added: Vec<String>,
    pub env: EnvAction,
    pub env_added: Vec<String>,
    pub gitignore: Vec<String>,
    pub contract: ContractPlan,
    pub notices: Vec<String>,
}

/// The `.gitignore` entries init would add.
pub(crate) fn gitignore_pending(root: &Path) -> Vec<String> {
    gitignore::pending_entries(root)
}

fn sorted(vars: &VarCollection) -> Vec<String> {
    let mut names: Vec<String> = vars.vars.keys().cloned().collect();
    names.sort();
    names
}

/// The contract [`super::contract::record`] would keep.
pub(crate) fn contract_pending(
    target: &Target,
    written: &VarCollection,
    whole: &VarCollection,
) -> Result<ContractPlan> {
    Ok(match super::contract::plan(target)? {
        Contract::Create(path) => ContractPlan {
            action: "created",
            file: Some(path),
            added: sorted(whole),
        },
        Contract::Append(path) => ContractPlan {
            action: "appended",
            file: Some(path),
            added: sorted(whole),
        },
        Contract::Record(path) => {
            let (outcome, _) = crate::commands::spec::writeback::pending(&target.root, written)?;
            match outcome {
                crate::commands::spec::writeback::Outcome::Wrote { names, .. } => ContractPlan {
                    action: "recorded",
                    file: Some(path),
                    added: names,
                },
                _ => ContractPlan {
                    action: "unchanged",
                    file: Some(path),
                    added: Vec::new(),
                },
            }
        }
    })
}

/// A generated template written over nothing (or, with `--force`, over the
/// existing one): [`super::shared::write_files`]' decisions.
pub(crate) fn plan_write(target: &Target, vars: &VarCollection) -> Result<Planned> {
    let root = &target.root;
    let env_path = root.join(".env");
    let (env, env_added) = if env_path.exists() {
        let have = crate::commands::add::shared::read_assignments(&env_path)?.unwrap_or_default();
        let missing: Vec<String> = sorted(vars)
            .into_iter()
            .filter(|n| !have.contains_key(n))
            .collect();
        if missing.is_empty() {
            (EnvAction::Unchanged, missing)
        } else {
            (EnvAction::Append, missing)
        }
    } else {
        (EnvAction::Create, sorted(vars))
    };
    Ok(Planned {
        template: target.template_name().to_string(),
        template_action: if target.template_path().exists() {
            TemplateAction::Replace
        } else {
            TemplateAction::Create
        },
        added: sorted(vars),
        env,
        env_added,
        gitignore: gitignore_pending(root),
        contract: contract_pending(target, vars, vars)?,
        notices: super::shared::notice_lines(target),
    })
}

/// The template kept as it is (DQ-31, or nothing new from the source):
/// `.env` copied from it when missing.
pub(crate) fn plan_keep(target: &Target, whole: &VarCollection) -> Result<Planned> {
    let root = &target.root;
    let (env, env_added) = if root.join(".env").exists() {
        (EnvAction::Unchanged, Vec::new())
    } else {
        (EnvAction::Copy, sorted(whole))
    };
    Ok(Planned {
        template: target.template_name().to_string(),
        template_action: TemplateAction::Keep,
        added: Vec::new(),
        env,
        env_added,
        gitignore: gitignore_pending(root),
        contract: contract_pending(target, &VarCollection::default(), whole)?,
        notices: super::shared::notice_lines(target),
    })
}

/// `--from-source` beside a template: `plan` is add's, which the append
/// follows; `whole` is the template after it.
pub(crate) fn plan_append(
    target: &Target,
    plan: &crate::commands::add::shared::Plan,
    whole: &VarCollection,
) -> Result<Planned> {
    let root = &target.root;
    let (env, env_added) = if plan.env_exists {
        if plan.to_env.vars.is_empty() {
            (EnvAction::Unchanged, Vec::new())
        } else {
            (EnvAction::Append, sorted(&plan.to_env))
        }
    } else {
        (EnvAction::Copy, sorted(whole))
    };
    Ok(Planned {
        template: target.template_name().to_string(),
        template_action: TemplateAction::Append,
        added: sorted(&plan.to_example),
        env,
        env_added,
        gitignore: gitignore_pending(root),
        contract: contract_pending(target, &plan.requested, whole)?,
        notices: super::shared::notice_lines(target),
    })
}

/// `--dry-run`: what the run would write, file by file. Printed whatever `-q`
/// says: it is the output that was asked for.
pub(crate) fn print_dry_run(p: &Planned) {
    let names = |n: &[String]| n.join(", ");
    let count = |n: usize| crate::utils::string::count(n, "variable");
    println!("Dry run: nothing was written.");
    let t = &p.template;
    match p.template_action {
        TemplateAction::Create => {
            println!(
                "  {t}: would be created with {}: {}",
                count(p.added.len()),
                names(&p.added)
            )
        }
        TemplateAction::Replace => println!(
            "  {t}: would be replaced (--force) with {}: {}",
            count(p.added.len()),
            names(&p.added)
        ),
        TemplateAction::Append => {
            println!(
                "  {t}: would add {}: {}",
                count(p.added.len()),
                names(&p.added)
            )
        }
        TemplateAction::Keep => println!("  {t}: kept as it is"),
    }
    match p.env {
        EnvAction::Create => println!(
            "  .env: would be created (0600) with {}",
            count(p.env_added.len())
        ),
        EnvAction::Copy => println!("  .env: would be created (0600) as a copy of {t}"),
        EnvAction::Append => println!(
            "  .env: would add {} it lacks: {}",
            count(p.env_added.len()),
            names(&p.env_added)
        ),
        EnvAction::Unchanged => println!("  .env: unchanged"),
    }
    if p.gitignore.is_empty() {
        println!("  .gitignore: unchanged");
    } else {
        println!("  .gitignore: would add {}", p.gitignore.join(", "));
    }
    let file = p
        .contract
        .file
        .as_deref()
        .map(ui::short_path)
        .unwrap_or_else(|| ".evnx.toml".to_string());
    let n = p.contract.added.len();
    match p.contract.action {
        "created" if n == 0 => println!("  {file}: would be created with an empty [vars]"),
        "created" => println!("  {file}: would be created with {} in [vars]", count(n)),
        "appended" if n == 0 => println!("  {file}: would get an empty [vars]"),
        "appended" | "recorded" => {
            println!(
                "  {file}: would add {} to [vars]: {}",
                count(n),
                names(&p.contract.added)
            )
        }
        _ => println!("  {file}: unchanged"),
    }
    for notice in &p.notices {
        println!("  · {notice}");
    }
}

/// `--json`: the result (or, with `--dry-run`, the plan) as one document.
/// Names only; nothing from `.env`, ever.
///
/// Fields shared with `add --json` (DQ-21): `dry_run`, `written`, `added`,
/// `skipped`, `conflicts`, `env_added`, `env_created`, `env_committable`,
/// `template`, `gitignore`, `contract` (`action`, `file`, `added`), `notices`.
pub(crate) fn print_json(p: &Planned, dry_run: bool, root: &Path) {
    let env_committable = !dry_run && !gitignore::env_file_is_protected(root, ".env");
    let doc = serde_json::json!({
        "dry_run": dry_run,
        "written": !dry_run,
        "added": p.added,
        "skipped": Vec::<String>::new(),
        "conflicts": Vec::<serde_json::Value>::new(),
        "env_added": p.env_added,
        "env_created": matches!(p.env, EnvAction::Create | EnvAction::Copy),
        "env_committable": env_committable,
        "template": p.template,
        "gitignore": p.gitignore,
        "contract": {
            "action": p.contract.action,
            "file": p.contract.file.as_deref().map(ui::short_path),
            "added": p.contract.added,
        },
        "notices": p.notices,
    });
    println!("{doc:#}");
}
