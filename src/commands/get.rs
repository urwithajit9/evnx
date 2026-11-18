//! `evnx get`: print one value from an env file, without sourcing it.
//!
//! `PORT=$(evnx get PORT)` is the use. The file is read by the same parser as
//! every other command, so the value is what `evnx run` would pass on: quotes
//! removed, escapes resolved, `${NAME}` expanded.
//!
//! A name that looks secret, or a value that does ([`classify::holds_secret`]),
//! is refused without `--reveal`, piped or not: a script that captures output
//! is exactly where a secret ends up in a log. `--exists` prints nothing and
//! answers in the exit code, so a script can test for a secret without
//! reading it.
//!
//! Exit codes: 0 printed (or present), 1 the name is not in the file, 2 could
//! not run (no such file, a line the parser cannot read), 4 a secret without
//! `--reveal`.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::core::parser::{Parser, ParserConfig};
use crate::core::verdict::Verdict;
use crate::core::{classify, env_set};
use crate::utils::path::label;

/// Everything `evnx get` takes.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub name: String,
    pub dir: String,
    pub env: Option<String>,
    pub env_name: Option<String>,
    pub exists: bool,
    pub reveal: bool,
    pub json: bool,
}

/// `--format json`. The value is there only for a name that is not secret, or
/// with `--reveal`.
#[derive(Serialize)]
struct Report<'a> {
    file: &'a str,
    name: &'a str,
    found: bool,
    secret: bool,
    value: Option<&'a str>,
}

/// Run `evnx get`.
pub fn run(opts: Options) -> Result<Verdict> {
    env_set::check_name(&opts.name)?;
    let project = Path::new(&opts.dir);
    let path = super::set::locate(
        project,
        opts.env.as_deref(),
        opts.env_name.as_deref(),
        "get",
    )?;
    let file = label(&path.to_string_lossy());
    let content = super::set::read_text(&path)?;

    // Read the way a process would get it: expanded.
    let doc = Parser::new(ParserConfig::default()).parse_document(&content);
    let entry = doc.entries.iter().rev().find(|e| e.key == opts.name);
    // A line the parser cannot read may be the one asked for, or may have
    // swallowed it: say so rather than "not found".
    let unreadable = |lines: Option<(usize, usize)>| {
        doc.problems
            .iter()
            .find(|p| match lines {
                Some((a, b)) => p.span.end_line >= a && p.span.line <= b,
                None => true,
            })
            .map(|p| p.describe())
    };
    if let Some(why) = unreadable(entry.map(|e| (e.span.line, e.span.end_line))) {
        return Err(crate::core::verdict::stopped(format!(
            "{} cannot be read from {file}: {why}\n  `evnx validate` lists every problem.",
            opts.name
        )));
    }

    let Some(entry) = entry else {
        if opts.json {
            emit(&opts, &file, false, false, None)?;
        } else if !opts.exists {
            eprintln!("{} is not in {file}.", opts.name);
        }
        return Ok(Verdict::Findings);
    };
    if opts.exists {
        return Ok(Verdict::Clean);
    }

    let secret = classify::holds_secret(&opts.name, entry.value.expose());
    if secret && !opts.reveal {
        if opts.json {
            emit(&opts, &file, true, true, None)?;
        }
        return Err(crate::core::verdict::stopped_needs_yes(format!(
            "{} looks like a secret, so evnx did not print it.\n\n\
             \x20 To print it anyway:   evnx get {} --reveal\n\
             \x20 To test that it is set:   evnx get {} --exists",
            opts.name, opts.name, opts.name
        )));
    }
    if opts.json {
        emit(&opts, &file, true, secret, Some(entry.value.expose()))?;
    } else {
        // The value and one line ending: `$(evnx get X)` strips it, and a
        // shell or `read` sees a whole line.
        let mut out = entry.value.expose().as_bytes().to_vec();
        out.push(b'\n');
        crate::utils::term::write_stdout(&out)
            .map_err(|e| anyhow::anyhow!("writing to stdout: {e}"))?;
    }
    Ok(Verdict::Clean)
}

fn emit(opts: &Options, file: &str, found: bool, secret: bool, value: Option<&str>) -> Result<()> {
    crate::core::json::emit(
        &Report {
            file,
            name: &opts.name,
            found,
            secret,
            value,
        },
        1,
        None,
    )
}
