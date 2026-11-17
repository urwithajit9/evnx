//! `evnx set`: change one variable in place.
//!
//! The file is edited by [`crate::core::env_set`]: one line changes, with its
//! position, comments, `export` prefix and quote style kept; a name that is not
//! there is appended. The write is [`safe_write::replace_all`], as `remove` and
//! `align` write: atomic, the file's mode kept, one `.bak` left beside it.
//!
//! # Where the value comes from
//!
//! 1. `NAME=VALUE` on the command line. For a secret-looking name this works
//!    and prints a warning (the shell keeps its history).
//! 2. Standard input, when it is piped.
//! 3. A prompt, when there is a terminal; a secret-looking name is typed with
//!    no echo.
//! 4. Otherwise nothing: exit 4, naming stdin.
//!
//! ⛔ A value is never printed on the command's own initiative when the name
//! looks secret or the value does ([`classify::holds_secret`]): not in the
//! `old → new` line, a warning, a message about the `.bak`, or `--json`.
//! `--json` carries no value at all.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::core::{classify, config, env_name, env_set, resolve, safe_write};
use crate::utils::path::label;
use crate::utils::prompt::Declined;
use crate::utils::ui;

/// Everything `evnx set` takes.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `NAME` or `NAME=VALUE`, as typed.
    pub assignment: String,
    pub dir: String,
    pub env: Option<String>,
    pub env_name: Option<String>,
    pub if_missing: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub json: bool,
}

/// What the run did to the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Appended: the name was not in the file.
    Added,
    /// Its value was replaced.
    Changed,
    /// It already held that value: nothing was written.
    Unchanged,
    /// `--if-missing`, and it is there: left as it is.
    Skipped,
}

/// `--format json`: names and statuses, never a value.
#[derive(Serialize)]
struct Report<'a> {
    dry_run: bool,
    file: &'a str,
    name: &'a str,
    status: Status,
    /// The name or the value looks like a credential, so no output shows it.
    secret: bool,
    backup: Option<String>,
}

/// The env file a command in `project` works on: `--env FILE` (relative to the
/// project folder), else `--env-name NAME`, else `[defaults] env_name`, else
/// `.env`. A missing file is the command's "not found", exit 2.
pub fn locate(
    project: &Path,
    env: Option<&str>,
    name: Option<&str>,
    command: &str,
) -> Result<PathBuf> {
    if !project.is_dir() {
        bail!("{} is not a folder (-C)", label(&project.to_string_lossy()));
    }
    let loaded = config::load(project)?;
    let path = match (env, name) {
        (Some(file), _) => {
            let p = Path::new(file);
            if p.is_absolute() || project == Path::new(".") {
                p.to_path_buf()
            } else {
                project.join(p)
            }
        }
        (None, Some(n)) => env_name::resolve(project, Some(n))?,
        (None, None) => match loaded.config.defaults.env_name.as_deref() {
            Some(n) => env_name::resolve(project, Some(n))?,
            None => env_name::resolve(project, None)?,
        },
    };
    if !path.is_file() {
        let shown = label(&path.to_string_lossy());
        if path.exists() {
            bail!("{shown} is not a file");
        }
        return Err(crate::core::verdict::missing_file(resolve::not_found(
            project, &shown, command,
        )));
    }
    Ok(path)
}

/// Read an env file as text.
pub fn read_text(path: &Path) -> Result<String> {
    let shown = label(&path.to_string_lossy());
    let bytes = std::fs::read(path).with_context(|| format!("reading {shown}"))?;
    String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("{shown} is not UTF-8 text, so nothing was changed"))
}

/// `NAME` and the value when `NAME=VALUE` was typed.
///
/// ⛔ Errors name the part before `=` only, and never an argument that has no
/// `=` and is not shaped like a name: someone who typed `evnx set 'TOKEN abc'`
/// has put a secret in it.
fn split(arg: &str) -> Result<(&str, Option<&str>)> {
    let (name, value) = match arg.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (arg, None),
    };
    if name.is_empty() {
        bail!("give the name to set: evnx set NAME=VALUE");
    }
    if env_set::check_name(name).is_err() {
        if value.is_none() && name.chars().any(char::is_whitespace) {
            bail!(
                "that is not a variable name: names hold letters, digits, _ . - [ and ] \
                 (to set a value, write NAME=VALUE with no spaces around the =)"
            );
        }
        bail!(
            "'{name}' is not a variable name: names hold letters, digits, _ . - [ and ] \
             (for example PORT or DATABASE_URL)"
        );
    }
    Ok((name, value))
}

/// The value, from standard input or a prompt.
fn ask_for_value(name: &str, secret: bool) -> Result<String> {
    use std::io::IsTerminal as _;
    let stdin_piped = !std::io::stdin().is_terminal();
    if stdin_piped {
        let mut text = String::new();
        std::io::stdin()
            .lock()
            .read_to_string(&mut text)
            .context("reading the value from stdin (it must be UTF-8 text)")?;
        // One line ending is the end of the command that wrote it, not part
        // of the value.
        if let Some(t) = text.strip_suffix('\n') {
            text = t.strip_suffix('\r').unwrap_or(t).to_string();
        }
        if !text.is_empty() {
            return Ok(text);
        }
    } else if crate::utils::term::can_prompt() {
        let prompt = format!("Value for {name}");
        let typed = if secret {
            dialoguer::Password::new()
                .with_prompt(prompt)
                .interact()
                .context("reading the value")?
        } else {
            dialoguer::Input::<String>::new()
                .with_prompt(prompt)
                .interact_text()
                .context("reading the value")?
        };
        return Ok(typed);
    }
    Err(crate::core::verdict::stopped_needs_yes(format!(
        "{name} has no value, and there is no terminal to ask for one.\n\n\
         \x20 Pipe it in:        printf %s \"$VALUE\" | evnx set {name}\n\
         \x20 Or give it here:   evnx set {name}=VALUE"
    )))
}

/// `value` on one line, for `old → new`.
fn shown(value: &str) -> String {
    if value.is_empty() {
        return "(empty)".to_string();
    }
    value.replace('\r', "\\r").replace('\n', "\\n")
}

/// Run `evnx set`.
pub fn run(opts: Options) -> Result<()> {
    let (name, given) = split(&opts.assignment)?;
    if opts.json {
        ui::set_quiet(true);
    }
    let project = Path::new(&opts.dir);
    let path = locate(
        project,
        opts.env.as_deref(),
        opts.env_name.as_deref(),
        "set",
    )?;
    let file = label(&path.to_string_lossy());
    let original = read_text(&path)?;
    let meta = std::fs::metadata(&path).with_context(|| format!("reading {file}"))?;
    if meta.permissions().readonly() && !opts.dry_run {
        bail!("{file} is read-only, so nothing was changed");
    }

    let secret_name = classify::is_sensitive_key(name);
    let now = env_set::current(&original, name, &file)?;

    // ── --if-missing: a name that is there is left alone, and no value is
    //    asked for.
    if opts.if_missing && now.is_some() {
        let secret = secret_name
            || now
                .as_ref()
                .is_some_and(|c| classify::holds_secret(name, &c.value));
        return finish(
            &opts,
            &file,
            name,
            Status::Skipped,
            secret,
            None,
            &format!("{name} is already set in {file}; left as it is."),
        );
    }

    // ── The value.
    let value = match given {
        Some(v) => {
            if secret_name {
                ui::warning(
                    "The value is on the command line, so your shell history has it. \
                     Next time leave it off and type it when asked, or pipe it in: \
                     printf %s \"$VALUE\" | evnx set NAME",
                );
            }
            v.to_string()
        }
        None => ask_for_value(name, secret_name)?,
    };
    let secret = secret_name
        || classify::holds_secret(name, &value)
        || now
            .as_ref()
            .is_some_and(|c| classify::holds_secret(name, &c.value));

    // ── The edit, in memory.
    let edit = env_set::set(&original, name, &value, &file)?;
    if !edit.changed(&original) {
        return finish(
            &opts,
            &file,
            name,
            Status::Unchanged,
            secret,
            None,
            &format!("{name} in {file} already holds that value; nothing to change."),
        );
    }
    if edit.assignments > 1 {
        ui::warning(format!(
            "{name} is assigned {} times in {file}; the last one (line {}) is the one that counts, and the one changed.",
            edit.assignments,
            edit.line
        ));
    }
    let line = match (&edit.old, secret) {
        (Some(_), true) => format!("{name}  changed (value hidden)"),
        (Some(old), false) => format!("{name}  {} → {}", shown(old), shown(&value)),
        (None, true) => format!("{name}  added (value hidden)"),
        (None, false) => format!("{name}  added: {}", shown(&value)),
    };
    let status = if edit.added {
        Status::Added
    } else {
        Status::Changed
    };

    // ── Dry run.
    if opts.dry_run {
        if opts.json {
            return print_json(&opts, &file, name, status, secret, None);
        }
        println!("Dry run: nothing was written.");
        println!("  {file}: {line}");
        return Ok(());
    }

    // ── Ask, when a value is about to be replaced (as `remove` and `add` ask
    //    before they write).
    if !edit.added && !opts.yes {
        if opts.json {
            return Err(crate::core::verdict::stopped_needs_yes(
                "--json never asks, and this would replace a value: add --yes (or --dry-run).",
            ));
        }
        println!("  {file}: {line}");
        let question = format!("Replace {name} in {file}?");
        let example = format!("evnx set {name} --yes");
        if !crate::utils::prompt::confirm(&question, false, &example)? {
            println!("  Cancelled; nothing was changed.");
            return Err(Declined.into());
        }
    }

    // ── Write.
    let files = [safe_write::Replacement {
        path: &path,
        label: &file,
        original: original.as_bytes(),
        content: edit.text.as_bytes(),
        private: true,
        backup: true,
    }];
    let backups = safe_write::replace_all(&files, &mut |_, _| {})?;
    let kept = backups.into_iter().next().flatten();
    let backup = kept.as_ref().map(|b| label(&b.path.to_string_lossy()));

    if opts.json {
        return print_json(&opts, &file, name, status, secret, backup);
    }
    ui::success(format!("{file}: {line}"));
    if let Some(b) = &kept {
        ui::say(format!(
            "  backup {}   undo: {}",
            label(&b.path.to_string_lossy()),
            b.undo()
        ));
    }
    Ok(())
}

/// Report a run that wrote nothing: `--if-missing` skipped, or no change.
fn finish(
    opts: &Options,
    file: &str,
    name: &str,
    status: Status,
    secret: bool,
    backup: Option<String>,
    message: &str,
) -> Result<()> {
    if opts.json {
        return print_json(opts, file, name, status, secret, backup);
    }
    ui::say(message);
    Ok(())
}

fn print_json(
    opts: &Options,
    file: &str,
    name: &str,
    status: Status,
    secret: bool,
    backup: Option<String>,
) -> Result<()> {
    let report = Report {
        dry_run: opts.dry_run,
        file,
        name,
        status,
        secret,
        backup,
    };
    crate::core::json::emit(&report, 1, None)
}
