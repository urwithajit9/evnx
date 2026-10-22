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
use std::path::Path;

use crate::core::config;
use crate::core::spec::{Format, VarSpec};
use crate::core::Parser;
use crate::docs;
use crate::schema::component;
use crate::schema::models::VarSource;
use crate::utils::ui;

/// Where the variables come from.
pub enum Source {
    /// `--with django,postgresql`: the component catalogue.
    Components(Vec<String>),
    /// The project's own files.
    Project(Inputs),
}

/// The project's files, resolved the way `validate` resolves them: `--env`,
/// `--env-name`, `[defaults] env_name`; `--example`, `[defaults] example`
/// (I-189, DEC-SP2).
pub struct Inputs {
    /// The environment file with the sample values.
    pub env: String,
    /// What named `env`, when something did. ⚠️ A named file that does not exist
    /// is an error, not an empty file (I-182, DEC-SP8); the default `.env` may
    /// be absent. A name (`--env-name`) was already checked when it resolved.
    pub env_named_by: Option<&'static str>,
    /// The template that decides membership.
    pub example: String,
    /// What named `example`: `--example` or `[defaults] example`.
    pub example_named_by: Option<&'static str>,
}

/// Where a generated entry came from, for the comment above it.
enum Origin {
    /// Named by the component catalogue.
    Component(String),
    /// Seen in the project's own files.
    Project,
    /// In the environment file but not in the template (I-183, DEC-SP9).
    OnlyIn(String),
}

struct Entry {
    name: String,
    spec: VarSpec,
    origin: Origin,
}

pub fn run(source: Source, stdout: bool, merge: bool, force: bool, verbose: bool) -> Result<()> {
    // ⛔ I-178, DEC-SP3. `--force` appended a second `[vars]` "to be merged by
    // hand"; with any repeated name TOML refuses the file, and every command
    // that reads the config exited 2 until someone did. The flag stays hidden
    // so that a script still using it gets this, not "unexpected argument".
    if force {
        bail!(
            "--force was removed: it appended a second [vars] section that TOML refuses to read.\n\
             \x20 Use --merge to add only the variables .evnx.toml does not declare yet;\n\
             \x20 existing entries are kept as they are."
        );
    }

    ui::print_header("evnx spec", Some("Generate the variable contract"));

    let (entries, left_out) = match source {
        Source::Components(ids) => from_components(&ids)?,
        Source::Project(inputs) => from_project(&inputs)?,
    };

    // ⚠️ Exit 2, not 0 (I-186, DEC-SP8): `spec init --with "$X" && git add
    // .evnx.toml` must not carry on with no file.
    if entries.is_empty() {
        bail!(
            "nothing to write: --with named no variables.\n\
             \x20 Name components (--with django,postgresql), or leave out --with to read\n\
             \x20 .env.example and .env."
        );
    }

    // I-180, DEC-SP4: a guessed format the project's own value fails is left
    // out, and said, so the first `validate` after this stays green. On
    // stderr: with `--stdout`, stdout is TOML only (DEC-SP1).
    for (name, format) in &left_out {
        ui::warning(format!(
            "{name}: left out `format = \"{}\"` — the sample value is not {}",
            format_name(format),
            format.describe()
        ));
    }

    if stdout {
        print!("{}", render(&entries, GENERATED));
        return Ok(());
    }

    // ⚠️ The project's one `.evnx.toml` — the one in force, else a new one at
    // the git root — never `./.evnx.toml` beside a config loaded from above
    // (I-177, DEC-SP2).
    let path = config::project_file(Path::new("."));
    let shown = ui::short_path(&path);
    let written = write_into(&path, &entries, merge)?;

    if written == 0 {
        ui::success(format!(
            "every variable is already declared in [vars] — {shown} unchanged"
        ));
        return Ok(());
    }
    if merge && written < entries.len() {
        ui::success(format!(
            "added {written} variable(s) to {shown}; {} already declared, kept as they are",
            entries.len() - written
        ));
    } else {
        ui::success(format!("wrote {written} variable(s) to {shown}"));
    }
    if verbose {
        for e in &entries {
            if let Origin::Component(c) = &e.origin {
                println!("  {}  ({c})", e.name);
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

/// Formats left out because the sample failed them, by variable.
type LeftOut = Vec<(String, Format)>;

/// The entry for `name` from a sample value, and the format left out if any.
fn entry(
    name: &str,
    sample: &str,
    required: bool,
    description: Option<String>,
) -> (VarSpec, Option<Format>) {
    let g = infer::guess(name, sample);
    let spec = VarSpec {
        required: Some(required),
        secret: g.secret.then_some(true),
        format: g.format,
        description,
        ..Default::default()
    };
    (spec, g.left_out)
}

/// Build entries from the component catalogue — `--with django,postgres`.
///
/// This is the higher-quality source: `VarMetadata` already carries `required`
/// and `description`, so nothing has to be guessed except the format.
fn from_components(ids: &[String]) -> Result<(Vec<Entry>, LeftOut)> {
    from_collection(&component::resolve(ids)?)
}

/// Entries for a resolved set of variables: [`from_components`]' half after
/// the catalogue lookup, shared with `init`'s contract (IN-15).
fn from_collection(
    collection: &crate::schema::models::VarCollection,
) -> Result<(Vec<Entry>, LeftOut)> {
    // The catalogue has no order of its own; alphabetical, as before.
    let mut names: Vec<&String> = collection.vars.keys().collect();
    names.sort();

    let mut out = Vec::new();
    let mut left_out = Vec::new();
    for name in names {
        let meta = &collection.vars[name];
        let (spec, dropped) = entry(
            name,
            &meta.example_value,
            meta.required,
            meta.description.clone(),
        );
        if let Some(f) = dropped {
            left_out.push((name.clone(), f));
        }
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
        out.push(Entry {
            name: name.clone(),
            spec,
            origin,
        });
    }
    Ok((out, left_out))
}

/// Build entries from `.env.example` (which names) and `.env` (what they look
/// like).
///
/// ⚠️ `.env.example` decides membership and `required`, because that is what it
/// already means: `validate` treats every line in the template as required
/// today. So a project generating its first spec keeps the behaviour it had,
/// and relaxes it per variable afterwards.
///
/// The template's order is kept, so its grouping survives; a name only in the
/// environment file follows, `required = false`, with a comment saying so —
/// it used to be dropped without a word (I-183, DEC-SP9).
fn from_project(inputs: &Inputs) -> Result<(Vec<Entry>, LeftOut)> {
    let template = read(&inputs.example, inputs.example_named_by)?;
    let actual = read(&inputs.env, inputs.env_named_by)?;

    if template.is_empty() && actual.is_empty() {
        bail!(
            "found no variables in {} or {}.\n\
             \x20 Run `evnx init` first, or name the files with --env and --example.",
            inputs.example,
            inputs.env
        );
    }

    // Names from the template when there is one; otherwise from `.env`.
    let (names, extras): (Vec<&String>, Vec<&String>) = if template.is_empty() {
        (actual.keys().collect(), Vec::new())
    } else {
        (
            template.keys().collect(),
            actual
                .keys()
                .filter(|k| !template.contains_key(*k))
                .collect(),
        )
    };

    let mut out = Vec::new();
    let mut left_out = Vec::new();
    let only_in = Path::new(&inputs.env)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| inputs.env.clone());

    for (name, required) in names
        .into_iter()
        .map(|n| (n, true))
        .chain(extras.into_iter().map(|n| (n, false)))
    {
        // A real value says more than a template placeholder, so prefer it.
        let sample = actual
            .get(name)
            .or_else(|| template.get(name))
            .map(String::as_str)
            .unwrap_or("");
        let (spec, dropped) = entry(name, sample, required, None);
        if let Some(f) = dropped {
            left_out.push((name.clone(), f));
        }
        // ⚠️ No `# from <component>` comment here, deliberately.
        // `component::declaring` returns the **first** component in the
        // catalogue that declares a name, and common names like DATABASE_URL and
        // PORT are declared by many. Attributing one of them to whichever
        // framework happened to be iterated first produced comments like
        // "# from NestJS" in an Express project. When the user names the
        // components with --with, the attribution is real; here it is a guess
        // dressed as a fact.
        out.push(Entry {
            name: name.clone(),
            spec,
            origin: if required {
                Origin::Project
            } else {
                Origin::OnlyIn(only_in.clone())
            },
        });
    }
    Ok((out, left_out))
}

/// The variables in `path`, in file order. A file nobody named may be absent
/// and reads as empty; a named one must exist (I-182, DEC-SP8).
fn read(path: &str, named_by: Option<&str>) -> Result<indexmap::IndexMap<String, String>> {
    let parser = Parser::default();
    match named_by {
        Some(by) => Ok(parser
            .parse_file_or_hint(path, &format!("{path} was named by {by}; check the path."))?
            .vars),
        None if Path::new(path).exists() => Ok(parser
            .parse_file(path)
            .with_context(|| format!("Failed to parse {path}"))?
            .vars),
        None => Ok(Default::default()),
    }
}

/// Render the `[vars]` block.
///
/// Hand-written rather than serialized so the output carries comments and a
/// stable field order. A spec is read by people.
const GENERATED: &str =
    "\n# ── Variable contract ───────────────────────────────────────────────\n\
     # Generated by `evnx spec init`. Every field below is inferred from this\n\
     # project's files — review it, then commit it.\n";

const MERGED: &str = "\n# Added by `evnx spec init --merge` — inferred from this project's\n\
     # files; review, then commit.\n";

fn render<'a>(entries: impl IntoIterator<Item = &'a Entry>, header: &str) -> String {
    let mut s = String::from(header);
    for e in entries {
        let comment = match &e.origin {
            Origin::Component(c) => Some(format!("from {c}")),
            Origin::OnlyIn(file) => Some(format!("only in {file}")),
            Origin::Project => None,
        };
        s.push_str(&render_entry(&e.name, &e.spec, comment.as_deref()));
    }
    s
}

/// One `[vars.NAME]` table — the one renderer, for `spec init` and the
/// writeback `add` uses. Field order: comment, description, required, format,
/// secret.
pub(crate) fn render_entry(name: &str, spec: &VarSpec, comment: Option<&str>) -> String {
    let mut s = format!("\n[vars.{name}]\n");
    if let Some(c) = comment {
        s.push_str(&format!("# {c}\n"));
    }
    if let Some(d) = &spec.description {
        s.push_str(&format!("description = {}\n", toml_string(d)));
    }
    s.push_str(&format!("required = {}\n", spec.required.unwrap_or(true)));
    if let Some(f) = &spec.format {
        s.push_str(&format!("format = {}\n", toml_string(&format_name(f))));
    }
    if spec.secret == Some(true) {
        s.push_str("secret = true\n");
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

/// Append entries to the project's `.evnx.toml`, creating it if it is not
/// there. Returns how many were written.
///
/// ⚠️ Appends rather than rewrites, so every other section, and every comment,
/// survives byte for byte.
///
/// Into a file that already has `[vars]`: refused, unless `merge` — then only
/// the names not yet declared are added (I-178, DEC-SP3).
///
/// ⛔ Nothing is written that does not parse: [`config::save`] checks, and a
/// failure leaves the file as it was (I-179).
fn write_into(path: &Path, entries: &[Entry], merge: bool) -> Result<usize> {
    write_into_with(path, entries, merge, GENERATED, MERGED)
}

/// [`write_into`] with the comment block the caller's entries go under.
fn write_into_with(
    path: &Path,
    entries: &[Entry],
    merge: bool,
    generated: &str,
    merged: &str,
) -> Result<usize> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
    };

    let parsed: toml::Table = existing.parse().with_context(|| {
        format!(
            "{} does not parse; fix it before adding to it — nothing was written",
            path.display()
        )
    })?;
    let declared: Vec<String> = parsed
        .get("vars")
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();

    let has_vars = config::has_vars_section(&existing);
    if has_vars && !merge {
        bail!(
            "{} already has a [vars] section.\n\
             \x20 Pass --merge to add only the variables it does not declare yet;\n\
             \x20 existing entries are kept as they are.",
            ui::short_path(path)
        );
    }

    let new: Vec<&Entry> = entries
        .iter()
        .filter(|e| !declared.contains(&e.name))
        .collect();
    if new.is_empty() {
        return Ok(0);
    }

    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&render(
        new.iter().copied(),
        if has_vars { merged } else { generated },
    ));

    let written = config::save(path, &out)?;
    // A symlinked config is written through to its target, and said (DEC-SP13).
    crate::core::safe_write::report(&written);
    Ok(new.len())
}

/// The comment above the `[vars]` entries `evnx init` writes.
const INIT_GENERATED: &str =
    "\n# ── Variable contract ───────────────────────────────────────────────\n\
     # Written by `evnx init` for the variables it set up: `evnx validate`\n\
     # checks the project against it. Review it, then commit it.\n";

/// `[vars]` for the variables `init` set up, written into the `.evnx.toml` at
/// `path` — created when it does not exist, appended to when it has no
/// `[vars]` yet (IN-15, DQ-18, DQ-27). Returns the names written.
///
/// ⚠️ The same entries `spec init --with` writes (catalogue `required` and
/// `description`, a format only when the sample fits, `secret` by the strict
/// name rule) through the same writer, so the contract does not depend on
/// which command made it. With nothing to declare, an empty `[vars]` is
/// written: the project has a contract, and `evnx add` records into it.
/// A file that already has `[vars]` is `writeback::record`'s, not this.
pub(crate) fn contract_into(
    path: &Path,
    vars: &crate::schema::models::VarCollection,
) -> Result<Vec<String>> {
    let (entries, left_out) = from_collection(vars)?;
    for (name, format) in &left_out {
        ui::warning(format!(
            "{name}: left out `format = \"{}\"` in [vars] — the sample value is not {}",
            format_name(format),
            format.describe()
        ));
    }
    if entries.is_empty() {
        let existing = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
        };
        if config::has_vars_section(&existing) {
            return Ok(Vec::new());
        }
        let mut out = existing;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(INIT_GENERATED);
        out.push_str("\n[vars]\n");
        let written = config::save(path, &out)?;
        crate::core::safe_write::report(&written);
        return Ok(Vec::new());
    }
    let names = entries.iter().map(|e| e.name.clone()).collect();
    write_into_with(path, &entries, true, INIT_GENERATED, MERGED)?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped_so_a_description_cannot_break_the_file() {
        assert_eq!(toml_string(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(toml_string("a\\b"), r#""a\\b""#);
        assert_eq!(toml_string("line\nbreak"), r#""line\nbreak""#);
    }

    #[test]
    fn rendered_output_parses_back_as_a_spec() {
        let e = [
            Entry {
                name: "DATABASE_URL".to_string(),
                spec: VarSpec {
                    required: Some(true),
                    secret: Some(true),
                    format: Some(Format::Url),
                    description: Some(r#"Postgres "primary""#.into()),
                    ..Default::default()
                },
                origin: Origin::Component("PostgreSQL".into()),
            },
            Entry {
                name: "MAX_RETRIES".to_string(),
                spec: VarSpec {
                    required: Some(false),
                    format: Some(Format::Int),
                    ..Default::default()
                },
                origin: Origin::OnlyIn(".env".into()),
            },
        ];

        let rendered = render(&e, GENERATED);

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
        assert!(
            rendered.contains("[vars.MAX_RETRIES]\n# only in .env\n"),
            "{rendered}"
        );
    }
}
