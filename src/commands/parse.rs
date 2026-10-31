//! `evnx parse` (RM-36, RM-88; D1, D2): what evnx reads from a `.env`, line by
//! line, and what is wrong with it.
//!
//! ⛔ **Names, never values, by default.** Each entry is its line, its key and
//! three flags. Values appear only with `--values`, which a terminal refuses
//! without `--yes`: printed there, they stay in the scrollback.

use anyhow::Result;
use serde::Serialize;

use crate::core::classify::{self, Strictness};
use crate::core::parser::Document;
use crate::core::Parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pretty,
    Json,
}

/// `--format json`. `schema_version` changes only when a field changes meaning.
#[derive(Serialize)]
struct Report {
    schema_version: u32,
    file: String,
    entries: Vec<EntryOut>,
    problems: Vec<ProblemOut>,
}

#[derive(Serialize)]
struct EntryOut {
    key: String,
    line: usize,
    empty: bool,
    placeholder: bool,
    secret: bool,
    /// Only with `--values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

#[derive(Serialize)]
struct ProblemOut {
    line: usize,
    message: String,
}

/// Read `env` (`-` for stdin) and print its entries. Exits 1 when the file has
/// problems; returns an error only when it could not be read.
pub fn run(env: &str, format: &str, values: bool, yes: bool) -> Result<()> {
    let format = crate::core::args::one_of(
        "format",
        format,
        &[
            ("pretty", Format::Pretty),
            ("text", Format::Pretty),
            ("json", Format::Json),
        ],
        &["pretty", "json"],
    )?;
    if values && !yes && std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        anyhow::bail!(
            "--values prints every value in plain text, and stdout is a terminal, \
             where it stays in the scrollback. Pipe it, or pass --yes."
        );
    }

    let (file, content) = if env == "-" {
        (
            "<stdin>".to_string(),
            crate::utils::term::read_piped("evnx parse -")?,
        )
    } else {
        let content = std::fs::read_to_string(env).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("{env} does not exist\n\n{}", Parser::missing_env_hint(env))
            } else {
                anyhow::Error::new(e).context(format!("could not read {env}"))
            }
        })?;
        (env.to_string(), content)
    };

    let doc = Parser::default().parse_document(&content);
    let report = report(&doc, file, values);
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Format::Pretty => print_pretty(&report),
    }
    if !report.problems.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn report(doc: &Document, file: String, values: bool) -> Report {
    let entries = doc
        .entries
        .iter()
        .map(|e| {
            let empty = e.value.is_empty();
            EntryOut {
                key: e.key.clone(),
                line: e.span.line,
                empty,
                // `validate`'s question, Strict: empty is its own flag.
                placeholder: !empty
                    && classify::is_placeholder(&e.key, &e.value, Strictness::Strict),
                secret: classify::holds_secret(&e.key, &e.value),
                value: values.then(|| e.value.clone()),
            }
        })
        .collect();
    let mut problems: Vec<ProblemOut> = doc
        .problems
        .iter()
        .map(|p| ProblemOut {
            line: p.span.line,
            message: p.describe(),
        })
        .collect();
    problems.sort_by_key(|p| p.line);
    Report {
        schema_version: 1,
        file,
        entries,
        problems,
    }
}

/// `LINE  KEY  flags`, then `value` with `--values`; problems on stderr.
fn print_pretty(report: &Report) {
    let flags: Vec<String> = report
        .entries
        .iter()
        .map(|e| {
            [
                (e.empty, "empty"),
                (e.placeholder, "placeholder"),
                (e.secret, "secret"),
            ]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect::<Vec<_>>()
            .join(",")
        })
        .collect();
    let keys = report
        .entries
        .iter()
        .map(|e| e.key.len())
        .max()
        .unwrap_or(0);
    let fl = flags.iter().map(String::len).max().unwrap_or(0);
    for (e, flags) in report.entries.iter().zip(&flags) {
        let line = match &e.value {
            // Escaped, so a multiline value stays on its row.
            Some(v) => format!(
                "{:>4}  {:<keys$}  {flags:<fl$}  {}",
                e.line,
                e.key,
                v.escape_debug()
            ),
            None => format!("{:>4}  {:<keys$}  {flags}", e.line, e.key),
        };
        println!("{}", line.trim_end());
    }
    for p in &report.problems {
        eprintln!("✗ {}", p.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rm88_flags_and_no_value_unless_asked() {
        let doc = Parser::default()
            .parse_document("PORT=8080\nAPI_KEY=\nSECRET_KEY=changeme\nTOKEN=ghp_s3cr3t\n");
        let r = report(&doc, ".env".into(), false);
        let row = |i: usize| {
            let e = &r.entries[i];
            (e.key.as_str(), e.line, e.empty, e.placeholder, e.secret)
        };
        assert_eq!(row(0), ("PORT", 1, false, false, false));
        assert_eq!(row(1), ("API_KEY", 2, true, false, true));
        assert_eq!(row(2), ("SECRET_KEY", 3, false, true, true));
        assert!(row(3).4);
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("changeme") && !json.contains("s3cr3t"),
            "{json}"
        );

        let r = report(&doc, ".env".into(), true);
        assert_eq!(r.entries[3].value.as_deref(), Some("ghp_s3cr3t"));
    }
}
