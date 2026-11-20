//! Reading many variables out of one file (`evnx add import`).
//!
//! One reader per format, each returning [`Found`]: where a value came from
//! (`db.host`, `services.web.environment.PORT`), the name it would get, and the
//! value or the reason it is refused. Everything after that is shared:
//!
//! * [`plan`] applies `--only` and `--exclude`, checks every name, and finds two
//!   sources that make one name;
//! * [`render_env`] and [`render_example`] write what `.env` and the template
//!   get: the values, and placeholders.
//!
//! The rules (owner decision 2026-11-13, all formats in v1): names are made
//! UPPER_SNAKE (`db.host` and `db-host` both become `DB_HOST`); nested keys are
//! joined with `__`; two sources that make one name are refused, not merged;
//! arrays, `null` and binary values are refused, naming the key.
//!
//! ⛔ A value is a [`Secret`]. No error, mapping or report in this module holds
//! one: a reason names a key or a line, never text from the file.

use anyhow::{Result, bail};
use indexmap::IndexMap;

use super::secret::Secret;

mod structured;
mod text;

pub use text::normalise;

/// What `--from` and the file's extension choose between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Env,
    Properties,
    Ini,
    Json,
    Yaml,
    Csv,
    WebConfig,
}

impl Format {
    /// The format a file name says, or `None`.
    pub fn detect(file: &str) -> Option<Format> {
        let base = std::path::Path::new(file)
            .file_name()?
            .to_string_lossy()
            .to_lowercase();
        if base == ".env" || base.starts_with(".env.") || base.ends_with(".env") {
            return Some(Format::Env);
        }
        match base.rsplit_once('.')?.1 {
            "json" => Some(Format::Json),
            "yml" | "yaml" => Some(Format::Yaml),
            "csv" => Some(Format::Csv),
            "properties" => Some(Format::Properties),
            "ini" | "cfg" | "conf" => Some(Format::Ini),
            "config" | "xml" => Some(Format::WebConfig),
            _ => None,
        }
    }

    /// How the format is spelled in `--from` and in messages.
    pub fn name(self) -> &'static str {
        match self {
            Format::Env => "env",
            Format::Properties => "properties",
            Format::Ini => "ini",
            Format::Json => "json",
            Format::Yaml => "yaml",
            Format::Csv => "csv",
            Format::WebConfig => "web-config",
        }
    }
}

/// One value in the source file.
#[derive(Debug, Clone)]
pub struct Found {
    /// Where it is in the file, as the file spells it: a key, `section.key`,
    /// `a.b.c`, `services.web.environment.PORT`, `line 3`.
    pub source: String,
    /// The name it would get (already normalised; may still be invalid).
    pub name: String,
    /// The value, or why it is refused (a reason without any value in it).
    pub value: std::result::Result<Secret, String>,
}

/// A value that will be written.
#[derive(Debug, Clone)]
pub struct Item {
    /// Position in the file, to list rows in the file's order.
    pub index: usize,
    pub source: String,
    pub name: String,
    pub value: Secret,
}

/// A source that cannot be imported, and why.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub index: usize,
    pub source: String,
    pub name: String,
    pub reason: String,
}

/// Two or more sources that make one name.
#[derive(Debug, Clone)]
pub struct Collision {
    pub name: String,
    pub sources: Vec<String>,
}

/// What a file holds, after `--only` and `--exclude`.
#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<Item>,
    pub refused: Vec<Refusal>,
    pub collisions: Vec<Collision>,
    /// Left out by `--only` or `--exclude`.
    pub filtered_out: usize,
    /// Things to tell the user that do not stop the run.
    pub notes: Vec<String>,
}

impl Plan {
    /// Nothing stops the run: no refusal and no collision.
    pub fn is_clean(&self) -> bool {
        self.refused.is_empty() && self.collisions.is_empty()
    }
}

/// Read `text` as `format`. `section` picks one INI section, top-level key,
/// compose service or workflow job; its name is not part of the variable names.
///
/// # Errors
///
/// The text is not valid in that format, or `section` is not in it. The
/// message names a line or a key, never a value.
pub fn read(format: Format, text: &str, section: Option<&str>) -> Result<Vec<Found>> {
    match format {
        Format::Env => text::env(text, section),
        Format::Properties => text::properties(text, section),
        Format::Ini => text::ini(text, section),
        Format::Csv => text::csv(text, section),
        Format::Json => structured::json(text, section),
        Format::Yaml => structured::yaml(text, section),
        Format::WebConfig => bail!(
            "web.config is not supported yet: this build has no XML reader. \
             Copy the <appSettings> entries into a .env or a CSV (name,value) \
             and import that."
        ),
    }
}

/// Apply the filters and sort what is left into items, refusals and
/// collisions.
pub fn plan(found: Vec<Found>, format: Format, only: &[String], exclude: &[String]) -> Plan {
    let mut plan = Plan::default();
    let include = (!only.is_empty()).then_some(only);
    let leave = (!exclude.is_empty()).then_some(exclude);
    let mut dollar = 0usize;
    for (index, f) in found.into_iter().enumerate() {
        if !crate::core::glob::admits(&f.name, include, leave) {
            plan.filtered_out += 1;
            continue;
        }
        let value = match f.value {
            Ok(v) => v,
            Err(reason) => {
                plan.refused.push(Refusal {
                    index,
                    source: f.source,
                    name: f.name,
                    reason,
                });
                continue;
            }
        };
        if !crate::core::parser::is_valid_key(&f.name) {
            plan.refused.push(Refusal {
                index,
                source: f.source,
                name: f.name,
                reason:
                    "not a valid variable name (letters, digits and _, not starting with a digit)"
                        .to_string(),
            });
            continue;
        }
        if matches!(format, Format::Env | Format::Properties) && value.expose().contains('$') {
            dollar += 1;
        }
        plan.items.push(Item {
            index,
            source: f.source,
            name: f.name,
            value,
        });
    }

    // Two sources, one name: refused, listing both.
    let mut by_name: IndexMap<&str, Vec<&str>> = IndexMap::new();
    for i in &plan.items {
        by_name.entry(&i.name).or_default().push(&i.source);
    }
    let collisions: Vec<Collision> = by_name
        .into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(name, sources)| Collision {
            name: name.to_string(),
            sources: sources.into_iter().map(str::to_string).collect(),
        })
        .collect();
    plan.collisions = collisions;
    if dollar > 0 {
        plan.notes.push(format!(
            "{dollar} {} a `$`: written as literal text, not expanded.",
            if dollar == 1 {
                "value has"
            } else {
                "values have"
            }
        ));
    }
    plan
}

/// The lines `.env` gets for `names`, under a heading naming the source.
pub fn render_env(items: &[Item], wanted: &dyn Fn(&str) -> bool, from: &str) -> String {
    let mut out = format!("\n# ── Imported from {from} ──\n");
    for i in items.iter().filter(|i| wanted(&i.name)) {
        out.push_str(&super::envfile::line(&i.name, i.value.expose()));
        out.push('\n');
    }
    out
}

/// The lines the template gets: the same names, with placeholders.
pub fn render_example(items: &[Item], wanted: &dyn Fn(&str) -> bool, from: &str) -> String {
    let mut out = format!("\n# ── Imported from {from} ──\n");
    for i in items.iter().filter(|i| wanted(&i.name)) {
        let ph = super::placeholder::generate_placeholder(
            &i.name,
            None,
            &super::placeholder::PlaceholderConfig::default(),
        );
        out.push_str(&super::envfile::line(&i.name, &ph));
        out.push('\n');
    }
    out
}

/// `add`'s flow wants the values as text, name to value.
pub fn values(items: &[Item]) -> IndexMap<String, Option<String>> {
    items
        .iter()
        .map(|i| (i.name.clone(), Some(i.value.expose().to_string())))
        .collect()
}

/// A warning when the source file is tracked by git and holds a value that
/// looks like a credential: the values are already in the repository.
pub fn tracked_warning(source: &str, items: &[Item]) -> Option<String> {
    if source == "-" {
        return None;
    }
    let abs = std::fs::canonicalize(source).ok()?;
    let dir = abs.parent()?;
    let name = abs.file_name()?.to_string_lossy().into_owned();
    if !super::gitignore::in_work_tree(dir)
        || super::gitignore::exposure(dir, &name) != super::gitignore::Exposure::Tracked
    {
        return None;
    }
    let secrets = items
        .iter()
        .filter(|i| super::classify::holds_secret(&i.name, i.value.expose()))
        .count();
    if secrets == 0 {
        return None;
    }
    let shown = crate::utils::path::label(source);
    Some(format!(
        "{shown} is tracked by git and holds {secrets} secret-looking {}: they are already in the \
         repository. Untrack it (`git rm --cached {shown}`) and rotate them.",
        if secrets == 1 { "value" } else { "values" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str, name: &str, value: &str) -> Found {
        Found {
            source: source.into(),
            name: name.into(),
            value: Ok(Secret::new(value)),
        }
    }

    #[test]
    fn detect_follows_the_file_name() {
        for (file, want) in [
            (".env", Some(Format::Env)),
            (".env.production", Some(Format::Env)),
            ("prod.env", Some(Format::Env)),
            ("a/b/secrets.JSON", Some(Format::Json)),
            ("c.yml", Some(Format::Yaml)),
            ("c.yaml", Some(Format::Yaml)),
            ("x.csv", Some(Format::Csv)),
            ("app.properties", Some(Format::Properties)),
            ("php.ini", Some(Format::Ini)),
            ("web.config", Some(Format::WebConfig)),
            ("noext", None),
            ("notes.txt", None),
        ] {
            assert_eq!(Format::detect(file), want, "{file}");
        }
    }

    #[test]
    fn two_sources_one_name_is_a_collision_listing_both() {
        let p = plan(
            vec![
                found("db.host", "DB_HOST", "a"),
                found("db-host", "DB_HOST", "b"),
            ],
            Format::Json,
            &[],
            &[],
        );
        assert_eq!(p.collisions.len(), 1);
        assert_eq!(p.collisions[0].sources, ["db.host", "db-host"]);
        assert!(!p.is_clean());
    }

    #[test]
    fn filters_apply_to_names_and_an_invalid_name_is_refused() {
        let p = plan(
            vec![
                found("a", "DB_URL", "1"),
                found("b", "API_KEY", "2"),
                found("c", "1BAD", "3"),
            ],
            Format::Json,
            &["DB_*".to_string(), "1*".to_string()],
            &[],
        );
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.refused.len(), 1);
        assert_eq!(p.filtered_out, 1);
        let p = plan(
            vec![found("a", "DB_URL", "1"), found("b", "API_KEY", "2")],
            Format::Json,
            &[],
            &["API_*".to_string()],
        );
        assert_eq!(p.items[0].name, "DB_URL");
    }

    #[test]
    fn a_dollar_in_an_env_value_is_noted() {
        let p = plan(vec![found("A", "A", "${B}x")], Format::Env, &[], &[]);
        assert_eq!(p.notes.len(), 1);
        let p = plan(vec![found("A", "A", "${B}x")], Format::Json, &[], &[]);
        assert!(p.notes.is_empty());
    }

    #[test]
    fn web_config_is_not_supported_and_says_so() {
        let e = read(Format::WebConfig, "<x/>", None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("not supported yet"), "{e}");
    }

    #[test]
    fn rendering_quotes_values_and_gives_the_template_placeholders() {
        let items = vec![Item {
            index: 0,
            source: "k".into(),
            name: "CERT".into(),
            value: Secret::new("a b\nc"),
        }];
        let env = render_env(&items, &|_| true, "x.json");
        assert!(env.contains("# ── Imported from x.json ──"), "{env}");
        assert!(env.contains("CERT=\"a b\\nc\""), "{env}");
        let ex = render_example(&items, &|_| true, "x.json");
        assert!(!ex.contains("a b"), "{ex}");
        assert!(ex.contains("CERT="), "{ex}");
    }
}
