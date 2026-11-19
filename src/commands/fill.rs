//! `evnx set --fill`: ask for every value that is still empty or a placeholder.
//!
//! The step after `evnx init --yes`, in place of `$EDITOR .env`. It walks the
//! names in the env file, in file order, whose value is empty or a placeholder
//! (the rule `validate` uses: `YOUR_KEY_HERE`, `changeme`, `xxx`, ...), and asks
//! for each. A secret-looking name is typed with no echo; Enter skips.
//!
//! * Nothing is written until the last answer, and then in one atomic write
//!   with one `.bak`. Ctrl-C before that writes nothing.
//! * A name that is only in the template is not added here: it is not in the
//!   file, and `evnx sync --direction reverse` is the command that adds it. The run says so.
//! * `--only <glob>` narrows the names asked for. `--dry-run` lists them and
//!   needs no terminal. Without a terminal, or under `--yes` or `--json`, there
//!   is nobody to answer: exit 4, naming `--dry-run` and `evnx set NAME`.
//! * `--json` is `{run, dry_run, file, filled, skipped, remaining, would_ask,
//!   missing, backup}`: names only, never a value.
//!
//! ⛔ A value is never printed.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::commands::set::{self, Options};
use crate::core::classify;
use crate::core::secret::Secret;
use crate::core::{config, env_set, glob, resolve, safe_write};
use crate::utils::path::label;
use crate::utils::ui;

/// `--json`.
#[derive(Serialize)]
struct Report {
    dry_run: bool,
    file: String,
    /// Names given a value.
    filled: Vec<String>,
    /// Names Enter was pressed for.
    skipped: Vec<String>,
    /// Names still empty or a placeholder in the file afterwards (`--only`
    /// does not hide them).
    remaining: Vec<String>,
    /// With `--dry-run`: the names a real run would ask for.
    would_ask: Vec<String>,
    /// Names the template lists and the file does not have: `evnx sync`.
    missing: Vec<String>,
    backup: Option<String>,
}

/// Whether `name`'s current value still has to be filled in.
fn needs_value(name: &str, value: &Secret) -> bool {
    crate::commands::validate::policy::is_placeholder(name, value.expose())
        || value.expose().trim().is_empty()
}

/// Run `evnx set --fill`.
pub fn run(opts: Options) -> Result<()> {
    if opts.json {
        ui::set_quiet(true);
    }
    let project = Path::new(&opts.dir);
    let path = set::locate(
        project,
        opts.env.as_deref(),
        opts.env_name.as_deref(),
        "set",
    )?;
    let file = label(&path.to_string_lossy());
    let original = set::read_text(&path)?;
    let meta = std::fs::metadata(&path).with_context(|| format!("reading {file}"))?;
    if meta.permissions().readonly() && !opts.dry_run {
        bail!("{file} is read-only, so nothing was changed");
    }

    let all = env_set::assignments(&original, &file)?;
    let wanted = |name: &str| opts.only.is_empty() || glob::admits(name, Some(&opts.only), None);
    let open: Vec<&(String, Secret)> = all.iter().filter(|(n, v)| needs_value(n, v)).collect();
    let asking: Vec<&str> = open
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| wanted(n))
        .collect();
    let missing = missing_from_template(&opts, project, &all);

    if !opts.only.is_empty() && asking.is_empty() && !open.is_empty() {
        ui::say(format!(
            "No empty or placeholder name in {file} matches --only {}.",
            opts.only.join(",")
        ));
    }

    // ── Dry run: the names, and no terminal needed.
    if opts.dry_run {
        let report = Report {
            dry_run: true,
            file: file.clone(),
            filled: vec![],
            skipped: vec![],
            remaining: open.iter().map(|(n, _)| n.clone()).collect(),
            would_ask: asking.iter().map(|s| s.to_string()).collect(),
            missing,
            backup: None,
        };
        if opts.json {
            return crate::core::json::emit(&report, 1, None);
        }
        println!("Dry run: nothing was written.");
        if asking.is_empty() {
            println!("  {file}: no empty or placeholder value to ask for.");
        } else {
            println!("  {file}: would ask for {}:", count(asking.len()));
            for n in &asking {
                let how = if classify::is_sensitive_key(n) {
                    "typed with no echo"
                } else {
                    "typed"
                };
                println!("    {n}  ({how})");
            }
        }
        say_missing(&report.missing);
        return Ok(());
    }

    if asking.is_empty() {
        let report = Report {
            dry_run: false,
            file: file.clone(),
            filled: vec![],
            skipped: vec![],
            remaining: open.iter().map(|(n, _)| n.clone()).collect(),
            would_ask: vec![],
            missing,
            backup: None,
        };
        if opts.json {
            return crate::core::json::emit(&report, 1, None);
        }
        ui::say(format!("{file}: no empty or placeholder value to fill in."));
        say_missing(&report.missing);
        return Ok(());
    }

    // ── Asking needs someone to ask.
    let reason = if opts.json {
        Some("--json never asks")
    } else if opts.yes {
        Some("--yes cannot answer for you")
    } else if !crate::utils::term::can_prompt() {
        Some("there is no terminal to ask on")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(crate::core::verdict::stopped_needs_yes(format!(
            "--fill asks for {} in {file}, and {reason}.\n\n\
             \x20 See which:         evnx set --fill --dry-run\n\
             \x20 One at a time:     evnx set {}=VALUE   (or pipe it: printf %s \"$V\" | evnx set {})",
            count(asking.len()),
            asking[0],
            asking[0],
        )));
    }

    // ── Ask. Everything is kept in memory until the end.
    println!(
        "{file}: {} to fill in. Enter skips one; Ctrl-C writes nothing.",
        count(asking.len())
    );
    let mut given: Vec<(String, Secret)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (i, name) in asking.iter().enumerate() {
        let prompt = format!("Value for {name} ({} of {})", i + 1, asking.len());
        let typed = ask(&prompt, classify::is_sensitive_key(name))?;
        if typed.is_empty() {
            skipped.push(name.to_string());
        } else {
            given.push((name.to_string(), typed));
        }
    }

    // ── One edit, one write.
    let mut backup = None;
    if !given.is_empty() {
        let text = env_set::set_many(&original, &given, &file)?;
        let files = [safe_write::Replacement {
            path: &path,
            label: &file,
            original: original.as_bytes(),
            content: text.as_bytes(),
            private: true,
            backup: true,
        }];
        let kept = safe_write::replace_all(&files, &mut |_, _| {})?;
        backup = kept
            .into_iter()
            .next()
            .flatten()
            .map(|b| (label(&b.path.to_string_lossy()), b.undo()));
    }

    let after = if given.is_empty() {
        all
    } else {
        env_set::assignments(&set::read_text(&path)?, &file)?
    };
    let remaining: Vec<String> = after
        .iter()
        .filter(|(n, v)| needs_value(n, v))
        .map(|(n, _)| n.clone())
        .collect();
    let report = Report {
        dry_run: false,
        file: file.clone(),
        filled: given.iter().map(|(n, _)| n.clone()).collect(),
        skipped,
        remaining,
        would_ask: vec![],
        missing,
        backup: backup.as_ref().map(|(p, _)| p.clone()),
    };
    if opts.json {
        return crate::core::json::emit(&report, 1, None);
    }

    if report.filled.is_empty() {
        ui::say("Nothing entered; nothing was written.");
    } else {
        ui::success(format!(
            "{file}: filled {} (values hidden)",
            report.filled.join(", ")
        ));
        if let Some((p, undo)) = &backup {
            ui::say(format!("  backup {p}   undo: {undo}"));
        }
    }
    if !report.skipped.is_empty() {
        ui::say(format!("  skipped: {}", report.skipped.join(", ")));
    }
    say_missing(&report.missing);
    summarise(&opts, project, &report.remaining);
    Ok(())
}

/// One answer, wrapped at once. A secret-looking name is read with no echo.
fn ask(prompt: &str, secret: bool) -> Result<Secret> {
    let prompt = format!("{prompt}, Enter to skip");
    let typed = if secret {
        dialoguer::Password::new()
            .with_prompt(prompt)
            .allow_empty_password(true)
            .interact()
    } else {
        dialoguer::Input::<String>::new()
            .with_prompt(prompt)
            .allow_empty(true)
            .interact_text()
    }
    .map_err(|e| anyhow::anyhow!("stopped while asking ({e}); nothing was written"))?;
    Ok(Secret::new(typed))
}

fn count(n: usize) -> String {
    crate::utils::string::count(n, "value")
}

/// Names the template lists and the file lacks.
fn missing_from_template(
    opts: &Options,
    project: &Path,
    assigned: &[(String, Secret)],
) -> Vec<String> {
    let Ok(loaded) = config::load(project) else {
        return Vec::new();
    };
    let flags = resolve::Flags {
        env: opts.env.as_deref(),
        env_name: opts.env_name.as_deref(),
        example: None,
    };
    let Ok(r) = resolve::resolve(project, flags, &loaded.config, resolve::Need::MayCreate) else {
        return Vec::new();
    };
    let Ok(Some(template)) =
        crate::commands::add::shared::read_assignments(&project.join(&r.template))
    else {
        return Vec::new();
    };
    template
        .keys()
        .filter(|k| !assigned.iter().any(|(n, _)| n == *k))
        .cloned()
        .collect()
}

fn say_missing(missing: &[String]) {
    if missing.is_empty() {
        return;
    }
    ui::say(format!(
        "  {} in the template, not in the file: {}   (evnx sync --direction reverse adds them)",
        crate::utils::string::count(missing.len(), "name"),
        missing.join(", ")
    ));
}

/// After a write: `validate`'s summary when a template exists, else the
/// names still empty. Never changes the exit code of `set`.
fn summarise(opts: &Options, project: &Path, remaining: &[String]) {
    let Ok(loaded) = config::load(project) else {
        return;
    };
    let cfg = loaded.config;
    let flags = resolve::Flags {
        env: opts.env.as_deref(),
        env_name: opts.env_name.as_deref(),
        example: None,
    };
    let resolved = resolve::resolve(project, flags, &cfg, resolve::Need::Existing).ok();
    let template_exists = resolved
        .as_ref()
        .is_some_and(|r| project.join(&r.template).is_file());
    if let (true, Some(r)) = (template_exists, resolved) {
        println!();
        let run = crate::commands::validate::run(crate::commands::validate::Options {
            env: label(&project.join(&r.env).to_string_lossy()),
            input_format: crate::commands::validate::InputFormat::Dotenv,
            example: label(&project.join(&r.template).to_string_lossy()),
            strict: cfg.validate.strict.unwrap_or(false),
            fix: false,
            dry_run: false,
            rotate_weak_secrets: false,
            format: crate::cli::args::ValidateFormat::Pretty,
            exit_zero: true,
            fail_on: None,
            fail_on_configured: cfg.validate.fail_on.clone(),
            verbose: false,
            ignore: Vec::new(),
            ignore_configured: cfg.validate.ignore.clone().unwrap_or_default(),
            validate_formats: cfg.validate.validate_formats.unwrap_or(true),
            spec: cfg.vars.clone(),
        });
        if run.is_ok() {
            return;
        }
    }
    if !remaining.is_empty() {
        ui::say(format!(
            "  {} still empty: {}",
            remaining.len(),
            remaining.join(", ")
        ));
    }
}
