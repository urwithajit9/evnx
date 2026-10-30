//! One `.env`, several readers: which lines they read differently (RM-04,
//! phase 1).
//!
//! A `.env` has no specification, and loaders disagree on a few line shapes.
//! evnx's parser does **not** change: it reads as Docker Compose does. What
//! this adds is a report: the readers this project actually has, and the
//! lines they would read differently from evnx.
//!
//! ⚠️ Every reader's value comes from `assets/dialects.json`, measured by
//! running the real loader (`scripts/measure-dialects.py`). A case a reader
//! was not measured on produces no finding for it.

use indexmap::IndexMap;
use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

use crate::core::parser::{Document, Entry};

const DIALECTS_JSON: &str = include_str!("../assets/dialects.json");

#[derive(Debug, Deserialize)]
struct Table {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    // Read by the test that pins evnx's column, and by the measurement script.
    #[allow(dead_code)]
    fixture: String,
    #[allow(dead_code)]
    key: String,
    /// `null` when the reader leaves the key unset (python-dotenv on a BOM).
    values: IndexMap<String, Option<String>>,
    effect: String,
    fix: String,
}

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    serde_json::from_str(DIALECTS_JSON).expect("assets/dialects.json is valid; a test checks it")
});

/// A program that reads the project's `.env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Compose,
    PythonDotenv,
    /// Node `dotenv`; `expand` is whether `dotenv-expand` is used with it,
    /// `None` when that is not known (`dotenv-cli`), so no expansion finding.
    NodeDotenv {
        expand: Option<bool>,
    },
}

impl Reader {
    pub fn name(self) -> &'static str {
        match self {
            Reader::Compose => "Docker Compose",
            Reader::PythonDotenv => "python-dotenv",
            Reader::NodeDotenv { .. } => "Node dotenv",
        }
    }

    /// The `dialects.json` column measured for this reader, if one was.
    fn column(self, case: &str) -> Option<&'static str> {
        match self {
            Reader::Compose => Some("compose"),
            Reader::PythonDotenv => Some("python-dotenv"),
            Reader::NodeDotenv { expand: Some(true) } => Some("node-dotenv-expand"),
            Reader::NodeDotenv {
                expand: Some(false),
            } => Some("node-dotenv"),
            // Unknown expansion: measured the same both ways except for
            // `${…}`, so only that case has no answer.
            Reader::NodeDotenv { expand: None } => (case != "expansion").then_some("node-dotenv"),
        }
    }
}

/// The readers `dir`'s project has, besides evnx: a compose file, a Python
/// manifest naming a dotenv loader, a `package.json` naming `dotenv`.
pub fn detect(dir: &Path) -> Vec<Reader> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    let mut readers = Vec::new();
    // Any compose file reads ./.env for interpolation (owner, 2026-10-28:
    // `env_file:` is not parsed in phase 1).
    if crate::core::project::docker_files_in(dir)
        .iter()
        .any(|f| f.contains("compose"))
    {
        readers.push(Reader::Compose);
    }
    // pydantic-settings and dynaconf load `.env` through python-dotenv.
    let python = ["requirements.txt", "pyproject.toml", "Pipfile"]
        .iter()
        .map(|f| read(f))
        .any(|text| {
            ["python-dotenv", "pydantic-settings", "dynaconf"]
                .iter()
                .any(|dep| text.contains(dep))
        });
    if python {
        readers.push(Reader::PythonDotenv);
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&read("package.json")) {
        let has = |dep: &str| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|section| json.get(section).and_then(|d| d.get(dep)).is_some())
        };
        if has("dotenv") || has("dotenv-cli") {
            let expand = if has("dotenv-expand") {
                Some(true)
            } else if has("dotenv-cli") {
                None
            } else {
                Some(false)
            };
            readers.push(Reader::NodeDotenv { expand });
        }
    }
    readers
}

/// A line some of the project's readers read differently from evnx.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub line: usize,
    pub key: String,
    /// The `dialects.json` case.
    pub case: &'static str,
    /// The readers that disagree with evnx (and those that agree).
    pub differ: Vec<Reader>,
    pub agree: Vec<Reader>,
    /// What the readers in `differ` do, and the one-line fix.
    pub effect: &'static str,
    pub fix: &'static str,
}

/// Which measured case the `i`th entry of `doc` is, if any.
fn case_of_entry(doc: &Document, i: usize) -> Option<&'static str> {
    // A BOM is read into the first key by some readers, whatever its value.
    if i == 0 && doc.bom {
        return Some("bom");
    }
    classify(&doc.entries[i])
}

/// Which measured case an entry's own text is, if any. Only unquoted values
/// (and `${` in double quotes) are cases in phase 1.
///
/// ⚠️ An entry, not a raw line: a line inside a multiline value is part of a
/// value, so `X=#ff0000` there is not an assignment for any reader.
fn classify(e: &Entry) -> Option<&'static str> {
    // A key cannot contain `=`, so the first one ends it.
    let (_, raw) = e.raw.split_once('=')?;
    let v = raw.trim_start();
    match e.quote {
        Some('"') => return v.contains("${").then_some("expansion"),
        Some(_) => return None,
        None => {}
    }
    if v.starts_with('#') {
        return Some(if v.len() < raw.len() {
            "empty_comment"
        } else {
            "hash_start"
        });
    }
    // A `#` after whitespace is a comment for every measured reader.
    if let Some(i) = v.find('#') {
        if !v[..i].ends_with([' ', '\t']) {
            return Some("hash_inside");
        }
    }
    let value = v.split(" #").next().unwrap_or(v);
    value.contains("${").then_some("expansion")
}

/// The entries of `doc` that `readers` read differently from evnx. Empty
/// when no reader is detected, or every one agrees.
pub fn mismatches(doc: &Document, readers: &[Reader]) -> Vec<Mismatch> {
    if readers.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, e) in doc.entries.iter().enumerate() {
        let Some(case_id) = case_of_entry(doc, i) else {
            continue;
        };
        let Some(case) = TABLE.cases.iter().find(|c| c.id == case_id) else {
            continue;
        };
        let evnx = &case.values["evnx"];
        let (mut differ, mut agree) = (Vec::new(), Vec::new());
        for &reader in readers {
            match reader.column(case_id).and_then(|c| case.values.get(c)) {
                Some(v) if v != evnx => differ.push(reader),
                Some(_) => agree.push(reader),
                None => {}
            }
        }
        if differ.is_empty() {
            continue;
        }
        out.push(Mismatch {
            line: e.span.line,
            key: e.key.clone(),
            case: case.id.as_str(),
            differ,
            agree,
            effect: case.effect.as_str(),
            fix: case.fix.as_str(),
        });
    }
    out
}

/// `a`, `a and b`, `a, b and c`.
pub fn names(readers: &[Reader]) -> String {
    join(readers.iter().map(|r| r.name()).collect())
}

/// [`names`] with evnx first: `evnx and a`, `evnx, a and b`.
pub fn names_with_evnx(readers: &[Reader]) -> String {
    join(
        std::iter::once("evnx")
            .chain(readers.iter().map(|r| r.name()))
            .collect(),
    )
}

fn join(n: Vec<&str>) -> String {
    match n.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: Reader = Reader::NodeDotenv {
        expand: Some(false),
    };

    fn doc(content: &str) -> Document {
        crate::core::Parser::default().parse_document(content)
    }

    /// The case of the last entry `content` holds.
    fn case_of(content: &str) -> Option<&'static str> {
        let d = doc(content);
        case_of_entry(&d, d.entries.len().checked_sub(1)?)
    }

    #[test]
    fn names_join_three_readers_with_one_and() {
        let three = [Reader::Compose, Reader::PythonDotenv, NODE];
        assert_eq!(
            names(&three),
            "Docker Compose, python-dotenv and Node dotenv"
        );
        assert_eq!(
            names_with_evnx(&[Reader::PythonDotenv, NODE]),
            "evnx, python-dotenv and Node dotenv"
        );
        assert_eq!(names_with_evnx(&[NODE]), "evnx and Node dotenv");
    }

    /// The table parses, and evnx's column is what evnx's parser reads from
    /// each fixture: if the parser changed a value, this fails.
    #[test]
    fn rm04_the_table_matches_evnx_s_parser() {
        for case in &TABLE.cases {
            let vars = crate::core::Parser::default()
                .parse_content(&case.fixture)
                .unwrap();
            assert_eq!(
                vars.get(&case.key),
                case.values["evnx"].as_ref(),
                "{}: evnx's column is not what evnx reads",
                case.id
            );
        }
    }

    /// Each fixture's line is classified as its own case.
    #[test]
    fn rm04_every_fixture_is_its_case() {
        for case in TABLE.cases.iter().filter(|c| c.id != "export") {
            let line = case.fixture.lines().last().unwrap();
            assert_eq!(case_of(line), Some(case.id.as_str()), "{line}");
        }
        assert_eq!(case_of("V_C=x # c"), None);
        assert_eq!(case_of("A='#x'"), None);
        assert_eq!(case_of("A=\"x#y\""), None);
        assert_eq!(case_of("A=\"${B}\""), Some("expansion"));
    }

    #[test]
    fn rm04_only_readers_that_differ_are_reported() {
        let content = "PORT=8080\nAPI_KEY=   # fill this in later\n";
        // Compose and python-dotenv read it as evnx does: nothing to say.
        assert!(mismatches(&doc(content), &[Reader::Compose, Reader::PythonDotenv]).is_empty());
        let found = mismatches(&doc(content), &[Reader::Compose, NODE]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].key, "API_KEY");
        assert_eq!(found[0].differ, [NODE]);
        assert_eq!(found[0].agree, [Reader::Compose]);
        assert!(mismatches(&doc(content), &[]).is_empty());
    }

    /// A line inside a multiline value is value text, not an assignment, so a
    /// `#` there is not a case for any reader. Line-based, this reported
    /// `X` on line 2 as read differently by Node dotenv.
    #[test]
    fn b1_a_line_inside_a_multiline_value_is_not_classified() {
        let content = "P=\"a\nX=#ff0000\nb\"\nC=#ff0000\n";
        let found = mismatches(&doc(content), &[NODE]);
        let at: Vec<(usize, &str)> = found.iter().map(|m| (m.line, m.key.as_str())).collect();
        assert_eq!(at, [(4, "C")]);
    }

    /// A BOM is skipped by evnx, Compose and Node, and read into the first
    /// key by python-dotenv: only python-dotenv differs, on line 1 (P0.1).
    #[test]
    fn p0_1_a_bom_is_a_mismatch_for_python_dotenv_only() {
        let content = "\u{feff}COLOR=#ff0000\nB=1\n";
        let found = mismatches(
            &doc(content),
            &[Reader::Compose, Reader::PythonDotenv, NODE],
        );
        let bom: Vec<_> = found.iter().filter(|m| m.case == "bom").collect();
        assert_eq!(bom.len(), 1, "{found:?}");
        assert_eq!((bom[0].line, bom[0].key.as_str()), (1, "COLOR"));
        assert_eq!(bom[0].differ, [Reader::PythonDotenv]);
        assert!(mismatches(&doc("COLOR=red\n"), &[Reader::PythonDotenv]).is_empty());
    }

    #[test]
    fn rm04_expansion_depends_on_dotenv_expand() {
        let content = "HOST=db\nURL=${HOST}:5432\n";
        assert_eq!(mismatches(&doc(content), &[NODE]).len(), 1);
        let expanding = Reader::NodeDotenv { expand: Some(true) };
        assert!(mismatches(&doc(content), &[expanding]).is_empty());
        let unknown = Reader::NodeDotenv { expand: None };
        assert!(mismatches(&doc(content), &[unknown]).is_empty());
    }

    #[test]
    fn rm04_readers_are_detected_from_the_project() {
        let d = tempfile::tempdir().unwrap();
        assert!(detect(d.path()).is_empty());
        std::fs::write(d.path().join("compose.yaml"), "services: {}\n").unwrap();
        std::fs::write(d.path().join("requirements.txt"), "python-dotenv==1.2\n").unwrap();
        std::fs::write(
            d.path().join("package.json"),
            r#"{"dependencies":{"x":"1"},"devDependencies":{"dotenv":"^18"}}"#,
        )
        .unwrap();
        assert_eq!(
            detect(d.path()),
            [Reader::Compose, Reader::PythonDotenv, NODE]
        );
    }
}
