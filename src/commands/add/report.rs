//! What `add` says at the end: the dry-run listing, the result lines and the
//! `--json` document, through the one renderer (WP-17, RUST-8).
//!
//! ⛔ Names, counts and the template's own values only. A [`PlanView`] is made
//! from the plan, never from `.env`: nothing here can show a value the user
//! typed.

use anyhow::Result;
use colored::*;

use super::shared::{Conflict, Plan};
use crate::core::output::{Out, Report};
use crate::utils::string::count;
use crate::utils::ui;

/// Which of the two texts a run ends with.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    /// `--dry-run`: what would be done, file by file.
    DryRun,
    /// A write (or nothing to write): the result lines, with the conflicts
    /// unless the question already showed them.
    Done { show_conflicts: bool },
}

/// The parts of a [`Plan`] the text reads, as names and counts.
#[derive(Debug, Clone)]
pub struct PlanView {
    to_example: Vec<String>,
    to_env: Vec<String>,
    requested: usize,
    present: Vec<String>,
    conflicts: Vec<Conflict>,
    env_kept: Vec<String>,
    env_exists: bool,
    create_env_file: bool,
    template: String,
    env_file: String,
    is_empty: bool,
    env_only: Vec<String>,
}

impl PlanView {
    pub fn of(plan: &Plan) -> Self {
        let names = |v: &crate::schema::models::VarCollection| v.vars.keys().cloned().collect();
        Self {
            to_example: names(&plan.to_example),
            to_env: names(&plan.to_env),
            requested: plan.requested.vars.len(),
            present: plan.present.clone(),
            conflicts: plan.conflicts.clone(),
            env_kept: plan.env_kept.clone(),
            env_exists: plan.env_exists,
            create_env_file: plan.create_env_file,
            template: plan.template.name.clone(),
            env_file: plan.env_file.clone(),
            is_empty: plan.is_empty(),
            env_only: plan.env_only().into_iter().map(String::from).collect(),
        }
    }

    fn sorted(names: &[String]) -> Vec<&str> {
        let mut n: Vec<&str> = names.iter().map(String::as_str).collect();
        n.sort_unstable();
        n
    }
}

/// `add`'s final result.
#[derive(Debug)]
pub struct AddReport {
    view: PlanView,
    /// What is counted: `variable`, or `custom variable`.
    noun: String,
    /// What finishes the sentence: `for PostgreSQL`.
    what: String,
    /// Given values (`add custom`).
    has_values: bool,
    step: Step,
    /// The `--json` document; `Null` when the run prints text.
    json: serde_json::Value,
}

impl AddReport {
    pub fn new(
        plan: &Plan,
        what: String,
        noun: &str,
        has_values: bool,
        step: Step,
        json: Option<serde_json::Value>,
    ) -> Self {
        Self {
            view: PlanView::of(plan),
            noun: noun.to_string(),
            what,
            has_values,
            step,
            json: json.unwrap_or(serde_json::Value::Null),
        }
    }
}

impl serde::Serialize for AddReport {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.json.serialize(s)
    }
}

impl Report for AddReport {
    fn text(&self, _out: &Out) -> Result<()> {
        match self.step {
            Step::DryRun => print_dry_run(&self.view, &self.what),
            Step::Done { show_conflicts } => {
                if show_conflicts {
                    print_conflicts(&self.view);
                }
                if self.has_values {
                    report_values(&self.view, &self.noun);
                } else {
                    report_added(&self.view, &self.noun, &self.what);
                }
            }
        }
        Ok(())
    }
}

/// `--dry-run`: what the run would do, file by file. Printed whatever `-q`
/// says: it is the output that was asked for.
fn print_dry_run(plan: &PlanView, what: &str) {
    let names = |v: &[String]| PlanView::sorted(v).join(", ");
    let what = what.trim();
    println!(
        "Dry run{}: nothing was written.",
        if what.is_empty() {
            String::new()
        } else {
            format!(" ({what})")
        }
    );
    if plan.to_example.is_empty() {
        println!("  {}: nothing to add", plan.template);
    } else {
        println!(
            "  {}: would add {}: {}",
            plan.template,
            count(plan.to_example.len(), "variable"),
            names(&plan.to_example)
        );
    }
    if !plan.present.is_empty() {
        println!("  already there, would skip: {}", plan.present.join(", "));
    }
    for c in &plan.conflicts {
        println!(
            "  different value, would keep: {} (existing: \"{}\", new: \"{}\")",
            c.var_name, c.existing_value, c.new_value
        );
    }
    for name in &plan.env_kept {
        println!(
            "  already set to a different value in {}, would keep: {name}",
            plan.env_file
        );
    }
    if plan.env_exists {
        if !plan.to_env.is_empty() {
            println!(
                "  {}: would add {}: {}",
                plan.env_file,
                count(plan.to_env.len(), "variable"),
                names(&plan.to_env)
            );
        }
    } else if plan.create_env_file && !plan.to_env.is_empty() {
        println!(
            "  {}: would be created with {} (with .gitignore first): {}",
            plan.env_file,
            count(plan.to_env.len(), "variable"),
            names(&plan.to_env)
        );
    } else if !plan.is_empty {
        println!(
            "  {}: would be created from {} (with .gitignore first)",
            plan.env_file, plan.template
        );
    }
}

/// One result line, and a second when `.env` caught up with the template.
fn report_added(plan: &PlanView, noun: &str, what: &str) {
    let added = plan.to_example.len();
    let skipped = plan.present.len() + plan.conflicts.len();
    let sentence = |n: usize| {
        let mut s = format!("{} {}", count(n, noun), what);
        s.truncate(s.trim_end().len());
        s
    };
    if added > 0 {
        let tail = if skipped > 0 {
            format!(", skipped {skipped} already present")
        } else {
            String::new()
        };
        ui::success(format!("Added {}{tail}", sentence(added)));
    } else if plan.to_env.is_empty() {
        ui::info(format!(
            "Nothing to add: {} already in {}",
            sentence(plan.requested),
            plan.template
        ));
    }
    if plan.env_exists && !plan.env_only.is_empty() {
        ui::info(format!(
            "Added {} to {} that {} already had: {}",
            count(plan.env_only.len(), "variable"),
            plan.env_file,
            plan.template,
            plan.env_only.join(", ")
        ));
    }
}

/// The result of a run that was given values (`add custom`): what each name
/// became, and a count of only the names written to the env file.
///
/// A name is written, kept (the env file has another value), or added to the
/// template only (the env file already has the value given).
fn report_values(plan: &PlanView, noun: &str) {
    let written = plan.to_env.len();
    let asked = plan.requested;
    let skipped = asked
        .saturating_sub(written)
        .saturating_sub(plan.env_kept.len());
    if written > 0 {
        let tail = if skipped > 0 {
            format!(", skipped {skipped} already present")
        } else {
            String::new()
        };
        ui::success(format!(
            "Added {} to {}{tail}",
            count(written, noun),
            plan.env_file
        ));
    } else if plan.env_kept.is_empty() {
        ui::info(format!(
            "Nothing to add: {} already in {}",
            count(asked, noun),
            plan.env_file
        ));
    } else {
        ui::info(format!("Nothing was written to {}", plan.env_file));
    }
    let mut template_only: Vec<&str> = plan
        .to_example
        .iter()
        .filter(|n| !plan.to_env.contains(n))
        .map(String::as_str)
        .collect();
    template_only.sort_unstable();
    if !template_only.is_empty() {
        ui::info(format!(
            "Added to {} only (the value is already in {}): {}",
            plan.template,
            plan.env_file,
            template_only.join(", ")
        ));
    }
}

/// The conflicts, as a warning and one line each. Values shown are
/// `.env.example`'s, never `.env`'s; a name the env file already sets to
/// another value is listed without either value.
pub fn print_conflicts(plan: &PlanView) {
    if !plan.env_kept.is_empty() {
        ui::warning(format!(
            "{} already set to a different value in {}: the existing value was kept",
            count(plan.env_kept.len(), "variable"),
            plan.env_file
        ));
        for name in &plan.env_kept {
            ui::warning_detail(format!(
                "  • {} (kept, not changed; edit {} to change it)",
                name.bold(),
                plan.env_file
            ));
        }
    }
    if plan.conflicts.is_empty() {
        return;
    }
    ui::warning(format!(
        "{} with a different value in {}: kept as it is",
        count(plan.conflicts.len(), "variable"),
        plan.template
    ));
    for c in &plan.conflicts {
        ui::warning_detail(format!(
            "  • {} (existing: \"{}\", new: \"{}\")",
            c.var_name.bold(),
            c.existing_value.dimmed(),
            c.new_value.dimmed()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::models::{VarMetadata, VarSource};

    /// The plan carries each name's value (`example_value`); the report keeps
    /// the names, so its `Debug` and JSON hold no part of it (WP-17).
    #[test]
    fn the_report_holds_no_value() {
        let canary = crate::core::output::canary::value();
        let mut plan = Plan::default();
        plan.to_env.vars.insert(
            "API_TOKEN".to_string(),
            VarMetadata {
                example_value: canary.clone(),
                description: None,
                category: None,
                required: true,
                source: VarSource::BlueprintOverride,
            },
        );
        plan.env_file = ".env".to_string();
        let report = AddReport::new(
            &plan,
            "for x".to_string(),
            "variable",
            true,
            Step::Done {
                show_conflicts: true,
            },
            Some(serde_json::json!({ "env_added": ["API_TOKEN"] })),
        );
        assert!(format!("{report:?}").contains("API_TOKEN"));
        crate::core::output::canary::assert_report_clean(&canary, &report);
    }
}
