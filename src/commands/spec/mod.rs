//! `evnx spec` — generate the variable contract.
//!
//! Slice 2 of the spec-first work. Slice 1 taught `.evnx.toml` to *read*
//! `[vars.*]`; this writes a first one, because the proposal's own objection to
//! spec-first was that it "asks the user to write a file before getting value".
//!
//! Nothing has to be written by hand: a project already states most of its
//! contract, in `.env.example` (which names are expected), in `.env` (what the
//! values look like), and in evnx's component catalogue (what a known variable
//! is for).

pub mod infer;
pub mod writeback;

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::core::spec::{Format, VarSpec};
use crate::core::Parser;
use crate::docs;
use crate::schema::component;
use crate::schema::models::VarSource;
use crate::utils::ui;

/// Where a generated entry came from, for the comment above it.
enum Origin {
    /// Named by the component catalogue.
    Component(String),
    /// Seen in the project's own files.
    Project,
}

struct Entry {
    spec: VarSpec,
    origin: Origin,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    with: Option<Vec<String>>,
    env: String,
    example: String,
    stdout: bool,
    force: bool,
    verbose: bool,
) -> Result<()> {
    ui::print_header("evnx spec", Some("Generate the variable contract"));

    let entries = if let Some(ids) = with {
        from_components(&ids)?
    } else {
        from_project(Path::new(&env), Path::new(&example))?
    };

    if entries.is_empty() {
        ui::warning("nothing to write — no variables found");
        ui::print_next_steps(&[
            "Run `evnx spec init --with <components>` to start from the catalogue",
            "Or create .env.example first, with `evnx init`",
        ]);
        return Ok(());
    }

    let rendered = render(&entries);

    if stdout {
        print!("{rendered}");
        return Ok(());
    }

    let path = PathBuf::from(crate::core::config::PROJECT_FILE);
    write_into(&path, &rendered, force)?;

    ui::success(format!(
        "wrote {} variable(s) to {}",
        entries.len(),
        path.display()
    ));
    if verbose {
        for (name, e) in &entries {
            if let Origin::Component(c) = &e.origin {
                println!("  {name}  ({c})");
            }
        }
    }
    // ⚠️ Said plainly, because everything above is inference. A generated
    // `format` that is wrong turns a working project red on the next `validate`.
    ui::print_next_steps(&[
        "Read the generated entries — every field is a guess from your files",
        "Mark optional variables with `required = false`",
        "Commit .evnx.toml; the spec is the contract",
    ]);
    ui::print_docs_hint(&docs::SPEC);
    Ok(())
}

/// Build entries from the component catalogue — `--with django,postgres`.
///
/// This is the higher-quality source: `VarMetadata` already carries `required`
/// and `description`, so nothing has to be guessed except the format.
fn from_components(ids: &[String]) -> Result<BTreeMap<String, Entry>> {
    let collection = component::resolve(ids)?;
    let mut out = BTreeMap::new();

    for (name, meta) in &collection.vars {
        let spec = VarSpec {
            required: Some(meta.required),
            secret: infer::looks_secret(name).then_some(true),
            format: infer::format_of(name, &meta.example_value),
            description: meta.description.clone(),
            environments: None,
            ..Default::default()
        };
        // ⚠️ From `meta.source`, not `component::declaring`. The resolver records
        // which of the components *the user named* contributed this variable;
        // `declaring` searches the whole catalogue and returns the first match,
        // which labelled DATABASE_URL "from NestJS" in a Django project.
        let origin = match &meta.source {
            VarSource::Framework(id) | VarSource::Service(id) | VarSource::Infrastructure(id) => {
                component::find(id)?
                    .map(|c| Origin::Component(c.display_name))
                    .unwrap_or_else(|| Origin::Component(id.clone()))
            }
            VarSource::BlueprintOverride => Origin::Project,
        };
        out.insert(name.clone(), Entry { spec, origin });
    }
    Ok(out)
}

/// Build entries from `.env.example` (which names) and `.env` (what they look
/// like).
///
/// ⚠️ `.env.example` decides membership and `required`, because that is what it
/// already means: `validate` treats every line in the template as required
/// today. So a project generating its first spec keeps the behaviour it had,
/// and relaxes it per variable afterwards.
fn from_project(env: &Path, example: &Path) -> Result<BTreeMap<String, Entry>> {
    let parser = Parser::default();

    let template = if example.exists() {
        parser
            .parse_file(example)
            .with_context(|| format!("Failed to parse {}", example.display()))?
            .vars
    } else {
        Default::default()
    };

    let actual = if env.exists() {
        parser
            .parse_file(env)
            .with_context(|| format!("Failed to parse {}", env.display()))?
            .vars
    } else {
        Default::default()
    };

    if template.is_empty() && actual.is_empty() {
        bail!(
            "found no variables in {} or {}.\n\
             \x20 Run `evnx init` first, or name the files with --env and --example.",
            example.display(),
            env.display()
        );
    }

    // Names from the template when there is one; otherwise from `.env`.
    let names: Vec<String> = if template.is_empty() {
        actual.keys().cloned().collect()
    } else {
        template.keys().cloned().collect()
    };

    let mut out = BTreeMap::new();
    for name in names {
        // A real value says more than a template placeholder, so prefer it.
        let sample = actual
            .get(&name)
            .or_else(|| template.get(&name))
            .map(String::as_str)
            .unwrap_or("");

        let spec = VarSpec {
            required: Some(true),
            secret: infer::looks_secret(&name).then_some(true),
            format: infer::format_of(&name, sample),
            description: None,
            environments: None,
            ..Default::default()
        };
        // ⚠️ No `# from <component>` comment here, deliberately.
        // `component::declaring` returns the **first** component in the
        // catalogue that declares a name, and common names like DATABASE_URL and
        // PORT are declared by many. Attributing one of them to whichever
        // framework happened to be iterated first produced comments like
        // "# from NestJS" in an Express project. When the user names the
        // components with --with, the attribution is real; here it is a guess
        // dressed as a fact.
        out.insert(
            name,
            Entry {
                spec,
                origin: Origin::Project,
            },
        );
    }
    Ok(out)
}

/// Render the `[vars]` block.
///
/// Hand-written rather than serialized so the output carries comments and a
/// stable field order. A spec is read by people.
fn render(entries: &BTreeMap<String, Entry>) -> String {
    let mut s = String::from(
        "\n# ── Variable contract ───────────────────────────────────────────────\n\
         # Generated by `evnx spec init`. Every field below is inferred from this\n\
         # project's files — review it, then commit it.\n",
    );

    for (name, entry) in entries {
        s.push_str(&format!("\n[vars.{name}]\n"));

        if let Origin::Component(c) = &entry.origin {
            s.push_str(&format!("# from {c}\n"));
        }
        if let Some(d) = &entry.spec.description {
            s.push_str(&format!("description = {}\n", toml_string(d)));
        }
        s.push_str(&format!(
            "required = {}\n",
            entry.spec.required.unwrap_or(true)
        ));
        if let Some(f) = &entry.spec.format {
            s.push_str(&format!("format = {}\n", toml_string(&format_name(f))));
        }
        if entry.spec.secret == Some(true) {
            s.push_str("secret = true\n");
        }
    }
    s
}

fn format_name(f: &Format) -> String {
    match f {
        Format::Url => "url".into(),
        Format::Int => "int".into(),
        Format::Port => "port".into(),
        Format::Bool => "bool".into(),
        Format::Email => "email".into(),
        Format::Pattern(re) => re.as_str().to_string(),
    }
}

/// A TOML basic string, escaped.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Append the block to `.evnx.toml`, creating the file if it is not there.
///
/// ⚠️ Appends rather than rewrites, so every other section, and every comment,
/// survives byte for byte. That also means an existing `[vars]` cannot be merged
/// into — which is why this refuses rather than producing a file with the key
/// twice, a shape TOML rejects on the next read.
fn write_into(path: &Path, block: &str, force: bool) -> Result<()> {
    let existing = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?
    } else {
        String::new()
    };

    if !force && has_vars_section(&existing) {
        bail!(
            "{} already has a [vars] section.\n\
             \x20 Appending a second one would produce a file TOML refuses to read.\n\
             \x20 Review the existing entries, or pass --force to append anyway and\n\
             \x20 merge them by hand.",
            path.display()
        );
    }

    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    std::fs::write(path, out).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Is there a `[vars.…]` or `[vars]` table already?
///
/// Deliberately textual: the file may not parse yet, and refusing to clobber
/// something unreadable is more important than being precise about it.
fn has_vars_section(src: &str) -> bool {
    src.lines()
        .map(str::trim)
        .any(|l| (l.starts_with("[vars.") || l == "[vars]") && !l.starts_with('#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_vars_section_is_detected_however_it_is_written() {
        assert!(has_vars_section("[vars.A]\n"));
        assert!(has_vars_section("  [vars.DATABASE_URL]  \n"));
        assert!(has_vars_section("[vars]\n"));
        assert!(!has_vars_section("[scan]\nseverity = \"high\"\n"));
        assert!(!has_vars_section("# [vars.A] was here\n"));
        // ⚠️ `[varsomething]` is a different table and must not trip the guard.
        assert!(!has_vars_section("[varsomething]\n"));
    }

    #[test]
    fn strings_are_escaped_so_a_description_cannot_break_the_file() {
        assert_eq!(toml_string(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(toml_string("a\\b"), r#""a\\b""#);
        assert_eq!(toml_string("line\nbreak"), r#""line\nbreak""#);
    }

    #[test]
    fn rendered_output_parses_back_as_a_spec() {
        let mut e = BTreeMap::new();
        e.insert(
            "DATABASE_URL".to_string(),
            Entry {
                spec: VarSpec {
                    required: Some(true),
                    secret: Some(true),
                    format: Some(Format::Url),
                    description: Some(r#"Postgres "primary""#.into()),
                    ..Default::default()
                },
                origin: Origin::Component("PostgreSQL".into()),
            },
        );
        e.insert(
            "MAX_RETRIES".to_string(),
            Entry {
                spec: VarSpec {
                    required: Some(false),
                    format: Some(Format::Int),
                    ..Default::default()
                },
                origin: Origin::Project,
            },
        );

        let rendered = render(&e);

        #[derive(serde::Deserialize)]
        struct W {
            vars: crate::core::spec::Spec,
        }
        let back: W = toml::from_str(&rendered)
            .unwrap_or_else(|err| panic!("rendered output does not parse: {err}\n{rendered}"));

        assert_eq!(back.vars["DATABASE_URL"].format, Some(Format::Url));
        assert_eq!(back.vars["DATABASE_URL"].secret, Some(true));
        assert_eq!(
            back.vars["DATABASE_URL"].description.as_deref(),
            Some(r#"Postgres "primary""#)
        );
        assert!(!back.vars["MAX_RETRIES"].is_required());
        // `secret` is left unstated rather than written as false — see core::spec.
        assert_eq!(back.vars["MAX_RETRIES"].secret, None);
    }
}
