//! `evnx init --with <a,b,c>` — name the pieces directly.
//!
//! The same resolve → preview → confirm → write shape as [`super::blueprint`],
//! over an explicit component list rather than a named stack. A blueprint is
//! structurally one of these lists, so this is the general case and blueprints
//! are the shorthand.

use super::shared::{aborted, write, Outcome, Target, WriteOptions};
use crate::schema::models::{VarCollection, VarMetadata, VarSource};
use crate::schema::{component, formatter};
use crate::utils::string::pluralize;
use crate::utils::ui::{self, glyph};
use anyhow::{Context, Result};
use colored::Colorize;

/// Generate from an explicit component list.
pub fn handle(
    target: &Target,
    opts: WriteOptions,
    verbose: bool,
    components: &[String],
) -> Result<Outcome> {
    // A stack named where components were expected says which flag takes it
    // (I-235): "unknown component" sent people looking for a typo.
    for id in components {
        let id = id.trim();
        if component::find(id)?.is_none() && crate::schema::loader::get_blueprint(id).is_some() {
            anyhow::bail!("{id} is a blueprint: use --blueprint {id}");
        }
    }

    // ⚠️ Resolve before anything is printed or written. An unknown name fails
    // here, with suggestions, rather than after a preview has implied it worked.
    let vars = component::resolve(components).context("Failed to resolve components")?;

    if verbose {
        println!(
            "{} {} components → {} variables",
            "[DEBUG]".dimmed(),
            components.len(),
            vars.vars.len()
        );
    }

    if !opts.yes {
        println!();
        for id in components {
            if let Some(c) = component::find(id)? {
                println!(
                    "  {}  {:<22} {}",
                    glyph::OK.green(),
                    c.display_name,
                    c.kind.label().dimmed()
                );
            }
        }
        println!("\n{}", formatter::generate_preview(&vars).dimmed());

        super::shared::require_terminal(&format!(
            "evnx init --with {} --yes",
            components.join(",")
        ))?;
        let confirm = super::shared::yes_no()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            return Ok(aborted());
        }
    }

    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;
    write(
        target,
        &vars,
        &example_content,
        &template_content,
        opts,
        &|| {
            ui::success(format!(
                "Created {} with {} from {}",
                target.template_name(),
                pluralize(vars.vars.len(), "variable", "variables"),
                pluralize(components.len(), "component", "components")
            ))
        },
    )
}

/// Print every component `--with` accepts, grouped by kind.
///
/// This is the list the error message points at, so it has to be reachable
/// without a project, without a prompt, and without side effects.
pub fn list() -> Result<()> {
    print_catalogue()?;
    println!(
        "\n  {}  evnx init --with {}\n",
        glyph::ARROW.dimmed(),
        "nextjs,postgresql,stripe".dimmed()
    );
    Ok(())
}

/// The catalogue, grouped by kind, aliases beside their component. Shared by
/// `init --list-components` and `add --list`, so the two cannot drift (7f).
pub fn print_catalogue() -> Result<()> {
    let all = component::catalogue()?;

    for kind in [
        component::Kind::Framework,
        component::Kind::Service,
        component::Kind::Infrastructure,
    ] {
        let of_kind: Vec<_> = all.iter().filter(|c| c.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }

        println!("\n  {}", kind.label().to_uppercase().bold());
        let mut group = "";
        for c in of_kind {
            if c.group != group {
                group = &c.group;
                println!("  {}", group.dimmed());
            }
            // Aliases are listed beside their component, or nobody finds them.
            let also: Vec<&str> = component::ALIASES
                .iter()
                .filter(|(_, id)| *id == c.id)
                .map(|(alias, _)| *alias)
                .collect();
            let name = if also.is_empty() {
                c.display_name.clone()
            } else {
                format!("{} (also: {})", c.display_name, also.join(", "))
            };
            println!("    {:<24} {}", c.id, name.dimmed());
        }
    }
    Ok(())
}

/// The detected path: propose what the project already declares, and confirm.
///
/// ⚠️ Proposed, never applied. A `stripe` dependency in a test fixture would
/// otherwise add payment variables to a project with no payments, and the
/// package table is wrong the week a framework renames itself. The evidence is
/// printed beside every component so the proposal can be checked rather than
/// taken on faith.
pub fn handle_detected(
    target: &Target,
    opts: WriteOptions,
    verbose: bool,
    found: &[super::detect::Detected],
) -> Result<Outcome> {
    let ids: Vec<String> = found.iter().map(|d| d.component.clone()).collect();
    let vars = component::resolve(&ids).context("Failed to resolve detected components")?;

    // What detection saw and chose not to propose (I-240: sqlx with SQLite or
    // no database feature), so an absent database is explained, not a mystery.
    for note in super::detect::notes(&target.root) {
        ui::say(format!("  {}  {}", glyph::INFO.dimmed(), note.dimmed()));
    }

    // ⚠️ Same files as before (DQ-5), different words. An empty "Detected"
    // heading followed by "0 variables from 0 components" read like a
    // malfunction; it is a normal outcome for a project evnx cannot read, and
    // the user needs to know what to do instead (I-015).
    if found.is_empty() {
        ui::say(format!(
            "\n  {}  Nothing detected in this project. Writing an empty template.\n     {}\n",
            glyph::INFO.dimmed(),
            "Name components instead: evnx init --with <a,b,c>   (list: evnx init --list-components)"
                .dimmed()
        ));
        let example_content = formatter::format_env_example(&vars, true)?;
        let template_content = formatter::format_env_template(&vars)?;
        return write(
            target,
            &vars,
            &example_content,
            &template_content,
            opts,
            &|| {
                ui::success(format!(
                    "Created {} (no variables yet)",
                    target.template_name()
                ))
            },
        );
    }

    // The proposal is part of the question when init is about to ask; under
    // `--detect` (no question) it is a status report, which `-q` silences.
    let show = !opts.yes || !ui::is_quiet();
    if show {
        println!("\n  {}", "Detected".bold());
    }
    for d in found.iter().filter(|_| show) {
        let name = component::find(&d.component)?
            .map(|c| c.display_name)
            .unwrap_or_else(|| d.component.clone());
        // Truncated rather than allowed to push the evidence column out of
        // alignment: `valkey/valkey:9-alpine` is longer than the column and the
        // tail of an image tag is not what identifies it.
        let matched = if d.matched.chars().count() > 22 {
            format!("{}…", d.matched.chars().take(21).collect::<String>())
        } else {
            d.matched.clone()
        };
        println!(
            "  {}  {:<22} {:<23} {}",
            glyph::OK.green(),
            name,
            matched.dimmed(),
            d.evidence.dimmed()
        );
    }
    if show {
        println!(
            "\n  {} from {}\n",
            pluralize(vars.vars.len(), "variable", "variables").bold(),
            pluralize(found.len(), "component", "components").dimmed()
        );
    }

    // ⚠️ Refuse before prompting, rather than letting `dialoguer` fail with
    // "IO error: not a terminal" — which says nothing about what to do instead.
    // Detection is a *proposal*, so accepting it without a human is opting in,
    // and there has to be a way to say so.
    if !opts.yes && !super::shared::can_ask() {
        anyhow::bail!(
            "detected {} components but {}.\n\n\
             Accept them:   evnx init --detect\n\
             Name them:     evnx init --with <a,b,c>\n\
             Skip them:     evnx init --yes      (writes blank files)",
            found.len(),
            super::shared::no_question_reason("there is nobody to confirm with")
        );
    }

    if !opts.yes {
        let confirm = super::shared::yes_no()
            .with_prompt("Generate .env files with these variables?")
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            println!(
                "  {}  nothing written — name them yourself with {}",
                glyph::INFO.dimmed(),
                "evnx init --with <a,b,c>".cyan()
            );
            return Ok(Outcome::Aborted);
        }
    }

    if verbose {
        println!("{} {} detected components", "[DEBUG]".dimmed(), ids.len());
    }

    let example_content = formatter::format_env_example(&vars, true)?;
    let template_content = formatter::format_env_template(&vars)?;
    write(
        target,
        &vars,
        &example_content,
        &template_content,
        opts,
        &|| {
            ui::success(format!(
                "Created {} with {}",
                target.template_name(),
                pluralize(vars.vars.len(), "variable", "variables")
            ))
        },
    )
}

/// `evnx init --from-source` — build the template from what the code reads.
///
/// ⚠️ This is the only path in evnx with an independent ground truth. Everywhere
/// else, "required" means "present in `.env.example`", which is circular: the
/// template cannot notice an entry the code stopped reading three refactors ago,
/// nor one it started reading last week. The source can.
pub fn handle_from_source(target: &Target, opts: WriteOptions, verbose: bool) -> Result<()> {
    let root = target.root.as_path();
    let name = target.template_name();
    let scan = super::source::scan(root).context("Failed to scan the project's source")?;
    let example = target.template_path();
    let drift = super::source::compare(&scan, &example)?;

    let skipped = if scan.symlinked_dirs_skipped > 0 {
        format!(
            " (skipped {}, not followed)",
            pluralize(
                scan.symlinked_dirs_skipped,
                "symlinked directory",
                "symlinked directories"
            )
        )
    } else {
        String::new()
    };
    ui::say(format!(
        "\n  scanned {}{}",
        pluralize(scan.files_scanned, "source file", "source files").dimmed(),
        skipped.dimmed()
    ));
    if scan.truncated {
        // A warning, so `-q` does not hide it: everything below is partial.
        ui::warning(format!(
            "stopped after {} source files; run it on a smaller directory with --path",
            scan.files_scanned
        ));
    }

    if scan.found.is_empty() {
        ui::say(format!(
            "\n  {}  no environment variables found in the source.\n     {}",
            glyph::INFO.dimmed(),
            "Name them instead: evnx init --with <a,b,c>".dimmed()
        ));
        if opts.json {
            println!(
                "{:#}",
                serde_json::json!({"dry_run": opts.dry_run, "written": false, "added": [],
                    "skipped": [], "conflicts": [], "env_added": [], "env_created": false,
                    "env_committable": false, "template": name, "gitignore": [],
                    "contract": {"action": "unchanged", "file": null, "added": []},
                    "notices": []})
            );
        }
        return Ok(());
    }

    // The metadata a name carries when the catalogue already knows it — so the
    // output stays annotated rather than a wall of bare `KEY=` lines.
    let mut vars = VarCollection::default();
    let mut annotated = 0usize;
    for v in &drift.missing {
        let known = component::declaring(&v.name)?;
        if known.is_some() {
            annotated += 1;
        }
        let from_catalogue = known
            .as_ref()
            .and_then(|c| component::resolve(std::slice::from_ref(&c.id)).ok())
            .and_then(|collection| collection.vars.get(&v.name).cloned());

        vars.vars.insert(
            v.name.clone(),
            from_catalogue.unwrap_or(VarMetadata {
                example_value: String::new(),
                description: Some(format!("read at {}", v.location)),
                category: None,
                required: false,
                source: VarSource::BlueprintOverride,
            }),
        );
    }

    // What is about to be added: part of the question when init asks, a status
    // report under `--yes`, which `-q` silences. The unused warnings below are
    // findings and always print.
    let show = !opts.yes || !ui::is_quiet();
    if show {
        println!(
            "\n  {}",
            format!(
                "{} read by the code",
                pluralize(scan.found.len(), "variable", "variables")
            )
            .bold()
        );
        for v in drift.missing.iter().take(12) {
            println!("     {:<28} {}", v.name, v.location.dimmed());
        }
        if drift.missing.len() > 12 {
            println!(
                "     {}",
                format!("… and {} more", drift.missing.len() - 12).dimmed()
            );
        }
    }

    let mut summary = vec![format!("{} already in {name}", drift.matched)];
    if !drift.missing.is_empty() {
        summary.push(format!("{} new", drift.missing.len()));
    }
    if !drift.unused.is_empty() && !scan.truncated {
        summary.push(format!("{} unused", drift.unused.len()));
    }
    if show {
        println!("\n  {}", summary.join("  ·  "));
    }

    // ⚠️ Reported, never removed. A variable the code does not name may still be
    // read by a library — `RUST_LOG` reaches `tracing_subscriber` without ever
    // appearing in a config file — so this is a question, not a verdict.
    // A partial scan cannot say a name is used nowhere, so it says nothing.
    for name in drift.unused.iter().filter(|_| !scan.truncated) {
        println!(
            "     {}  {} is in {} but named nowhere in the source",
            glyph::WARN.yellow(),
            name,
            target.template_name()
        );
    }

    if drift.missing.is_empty() {
        ui::say(format!(
            "\n  {}  {name} already covers the source.",
            glyph::OK.green()
        ));
        // Nothing to add, and still a project init set up: `.env` from the
        // template when there is none (N-06), and the contract.
        let whole = super::shared::template_vars(target)?;
        let planned = if opts.dry_run || opts.json {
            Some(super::report::plan_keep(target, &whole)?)
        } else {
            None
        };
        if !opts.dry_run {
            let done = super::shared::keep(target, &VarCollection::default(), &whole);
            super::shared::env_from_template(root, &target.template, &target.env_file, !opts.yes)?;
            done();
        }
        super::shared::emit(planned.as_ref(), opts, root);
        return Ok(());
    }

    if verbose {
        println!(
            "{} {annotated} of {} names matched a known component",
            "[DEBUG]".dimmed(),
            drift.missing.len()
        );
    }

    if !opts.yes && !super::shared::can_ask() {
        anyhow::bail!(
            "found {} variables the code reads that {name} does not declare, \
             but {}.\n\n\
             Accept them:  evnx init --from-source --yes",
            drift.missing.len(),
            super::shared::no_question_reason("there is nobody to confirm with")
        );
    }
    if !opts.yes {
        let confirm = super::shared::yes_no()
            .with_prompt(format!("Add these {} to {name}?", drift.missing.len()))
            .default(true)
            .interact_opt()?;
        if confirm != Some(true) {
            aborted();
            return Ok(());
        }
    }

    // ⚠️ Append, never replace. These are the variables the template is
    // *missing*, so writing them as a whole file would delete the ones it
    // already had — and an existing template is a tracked, hand-maintained
    // file. The first version of this did exactly that, turning a 21-variable
    // template into a 5-variable one.
    let added = || {
        ui::success(format!(
            "Added {} from the source",
            pluralize(drift.missing.len(), "variable", "variables")
        ))
    };
    if example.exists() {
        // Through `add`'s planner, so a name the user already set in `.env`
        // is not appended there again below their real value (N-11). Not its
        // `[vars]` write-back: init keeps the contract itself, below.
        let plan = crate::commands::add::shared::plan(root, &vars, target.env_name.as_deref())?;
        let mut whole = super::shared::template_vars(target)?;
        whole.vars.extend(plan.to_example.vars.clone());
        let planned = if opts.dry_run || opts.json {
            Some(super::report::plan_append(target, &plan, &whole)?)
        } else {
            None
        };
        if opts.dry_run {
            super::shared::emit(planned.as_ref(), opts, root);
            return Ok(());
        }
        // Protect before writing, as every init path does (I-008). `add` only
        // warns about `.gitignore`, because it runs in someone else's project;
        // `init` owns these rules (N-06, DQ-10).
        let gitignore_line = super::shared::update_gitignore(root)?;
        crate::commands::add::shared::apply_files(
            root,
            &plan,
            &|v| formatter::format_addition_from(v, "evnx init --from-source"),
            None,
            verbose,
            None, // init creates `.env` itself, just below
            true,
        )?;
        let done = super::shared::keep(target, &plan.requested, &whole);
        if let Some(line) = gitignore_line {
            ui::say(line);
        }
        // A template with no `.env` beside it: create one from it, asking first
        // when there is someone to ask (DQ-10, DQ-14).
        super::shared::env_from_template(root, &target.template, &target.env_file, !opts.yes)?;
        added();
        done();
        super::shared::emit(planned.as_ref(), opts, root);
    } else {
        let example_content = formatter::format_env_example(&vars, true)?;
        let template_content = formatter::format_env_template(&vars)?;
        // No template yet, so nothing to ask about: this writes `.gitignore`
        // first and the template and `.env` after it (I-008).
        write(
            target,
            &vars,
            &example_content,
            &template_content,
            opts,
            &added,
        )?;
    }
    Ok(())
}
