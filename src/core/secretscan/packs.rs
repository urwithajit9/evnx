//! Rule packs (C7; S2, S4, S6): rule files added from the catalogue, a URL or
//! a path, recorded in `rules.lock` beside the rule folder with their sha256.
//!
//! ⛔ **A scan refuses a pack whose bytes no longer match the lock** (S4). A
//! file in the rule folder with no lock entry is a hand-written rule file and
//! is not hash-checked. Nothing here touches the network: fetching is the
//! `evnx scan rules add|update` command's, and only theirs.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::core::config::PatternRule;

/// The catalogue's index (S6). One constant, so moving it to rules.evnx.dev
/// is a one-line change. `EVNX_RULES_CATALOGUE` overrides it for tests and
/// mirrors.
pub const CATALOGUE_URL: &str =
    "https://raw.githubusercontent.com/urwithajit9/evnx-rules/main/index.toml";

/// `EVNX_RULES_CATALOGUE`, else [`CATALOGUE_URL`].
pub fn catalogue_url() -> String {
    std::env::var("EVNX_RULES_CATALOGUE")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| CATALOGUE_URL.to_string())
}

/// `rules.lock`, beside the rule folder: `.evnx/rules.lock` for
/// `.evnx/rules`, `~/.config/evnx/rules.lock` for the personal folder.
pub fn lock_path(rules_dir: &Path) -> PathBuf {
    rules_dir.with_file_name("rules.lock")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Lock {
    pub version: u32,
    #[serde(default)]
    pub packs: Vec<Entry>,
}

/// One added pack. The file is `<rules dir>/<name>.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    /// `catalogue`, `url` or `path`: how `update` fetches it again.
    pub kind: String,
    /// The catalogue name, the URL or the path, as given to `add`.
    pub source: String,
    /// `evnx` or `gitleaks`: what the source is written in.
    pub format: String,
    /// Of the stored file, which for `gitleaks` is the converted copy.
    pub sha256: String,
    pub rules: usize,
    /// `YYYY-MM-DD`.
    pub added: String,
}

impl Lock {
    /// A missing lock is an empty one.
    pub fn load(path: &Path) -> Result<Lock> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Lock {
                    version: 1,
                    packs: Vec::new(),
                })
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let lock: Lock = toml::from_str(&text)
            .with_context(|| format!("{} is not an evnx rules.lock", path.display()))?;
        if lock.version != 1 {
            bail!(
                "{} has version {}; this evnx reads 1",
                path.display(),
                lock.version
            );
        }
        Ok(lock)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = format!(
            "# Written by `evnx scan rules add|update|remove`; commit it with the packs.\n\
             # A scan refuses a pack whose sha256 no longer matches.\n{}",
            toml::to_string(self)?
        );
        crate::core::safe_write::write_shared(path, body.as_bytes())?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.packs.iter().find(|e| e.name == name)
    }
}

/// S4: every pack the lock under `rules_dir` lists is present and unchanged.
/// `shown` is how the folder is named in messages.
pub fn verify(rules_dir: &Path, shown: &str) -> Result<()> {
    let lock = Lock::load(&lock_path(rules_dir))?;
    for e in &lock.packs {
        let file = rules_dir.join(format!("{}.toml", e.name));
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => bail!(
                "rules.lock lists the pack '{}', but {shown}/{}.toml is missing; \
                 run `evnx scan rules remove {}` or `evnx scan rules update {}`",
                e.name,
                e.name,
                e.name,
                e.name
            ),
            Err(err) => return Err(err).with_context(|| format!("reading {}", file.display())),
        };
        if sha256_hex(&bytes) != e.sha256 {
            bail!(
                "{shown}/{}.toml changed since it was added; run `evnx scan rules update {}` \
                 or remove it (`evnx scan rules remove {}`)",
                e.name,
                e.name,
                e.name
            );
        }
    }
    Ok(())
}

/// A pack name: lowercase letters, digits and `-`, as a rule id.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── The catalogue index ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub version: u32,
    /// `[[pack]]`, as the evnx-rules README specifies.
    #[serde(default, rename = "pack")]
    pub packs: Vec<IndexEntry>,
}

/// One `[[pack]]`. Other fields the index carries (`licence`, `upstream`,
/// `commit`) are for people and are not read.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    /// Relative to the index's own URL, or absolute.
    pub path: String,
    pub sha256: String,
    pub rules: usize,
    #[serde(default)]
    pub description: String,
}

impl Index {
    pub fn parse(text: &str) -> Result<Index> {
        let index: Index = toml::from_str(text).context("the catalogue index is not valid")?;
        if index.version != 1 {
            bail!(
                "the catalogue index has version {}; this evnx reads 1",
                index.version
            );
        }
        Ok(index)
    }
}

/// `url` against the index at `base`: absolute stays, relative is joined to
/// the index's folder.
pub fn resolve(base: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_string();
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{}", &base[..i], url.trim_start_matches("./")),
        None => url.to_string(),
    }
}

// ── gitleaks import (S2) ───────────────────────────────────────────────────

/// What [`from_gitleaks`] made of a gitleaks file.
#[derive(Debug, Default)]
pub struct Converted {
    pub rules: Vec<PatternRule>,
    /// `(gitleaks id, why it was left out)`, in file order. Never silent.
    pub skipped: Vec<(String, String)>,
    /// Rules with an allowlist that could not be carried over (one tied to
    /// paths or commits): they may report more than in gitleaks.
    pub narrowed: usize,
    /// Rules whose Betterleaks `filter` held more than an entropy floor, or
    /// that were composites needing `components`: what evnx cannot express
    /// is dropped, so they may report more than upstream.
    pub filtered: usize,
}

/// Convert gitleaks' TOML. `id`, `regex`, `keywords`, `entropy`, the secret
/// group and the allowlists (see [`allowlists`]) carry over; the
/// description becomes the name; severity and confidence are `medium`. Each
/// id gets `<pack>-` in front, so a pack cannot collide with evnx's own ids
/// (S5). A regex Rust's `regex` cannot compile (look-around, back-references)
/// is skipped and listed.
///
/// Betterleaks (the gitleaks fork) writes the same format plus a few fields,
/// read here too: `confidence` carries over; `valueGroup` is its secret
/// group; a rule with `skipReport = true` only exists as part of a composite
/// and is skipped; and from the CEL `filter` the leading
/// `entropy(finding["secret"]) < N` (or `<=`) clause becomes the entropy floor.
/// Anything else in a filter, and a composite's `components`, cannot be
/// expressed: the rule is kept, counted in [`Converted::filtered`].
pub fn from_gitleaks(text: &str, pack: &str) -> Result<Converted> {
    let doc: toml::Value = toml::from_str(text).context("not a valid gitleaks TOML file")?;
    let Some(rules) = doc.get("rules").and_then(|r| r.as_array()) else {
        bail!("no [[rules]] in this gitleaks file");
    };
    let mut out = Converted::default();
    for (i, r) in rules.iter().enumerate() {
        let gid = r
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("rule-{}", i + 1));
        if r.get("skipReport").and_then(|v| v.as_bool()) == Some(true) {
            out.skipped.push((
                gid,
                "reported only as part of a composite rule (skipReport)".into(),
            ));
            continue;
        }
        let Some(regex) = r.get("regex").and_then(|v| v.as_str()) else {
            out.skipped
                .push((gid, "no regex: it matches file paths only".into()));
            continue;
        };
        let regex = &ascii_classes(regex);
        // The same ceiling a scan compiles with, so what converts also loads.
        let compiled = match regex::RegexBuilder::new(regex)
            .size_limit(super::patternset::PATTERN_SIZE_LIMIT)
            .build()
        {
            Ok(re) => re,
            Err(e) => {
                let why = e.to_string();
                let first = why.lines().last().unwrap_or("").trim().to_string();
                out.skipped
                    .push((gid, format!("its regex does not compile in Rust: {first}")));
                continue;
            }
        };
        let (allow, allow_line, dropped) = allowlists(r);
        if dropped {
            out.narrowed += 1;
        }
        let slug: String = gid
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let id = if slug.starts_with(&format!("{pack}-")) {
            slug
        } else {
            format!("{pack}-{slug}")
        };
        let name = r
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|d| !d.trim().is_empty())
            .map(|d| d.trim().to_string())
            .unwrap_or_else(|| gid.clone());
        let mut rule = PatternRule::default();
        rule.name = name;
        rule.regex = regex.to_string();
        rule.id = Some(id);
        rule.confidence = Some(
            r.get("confidence")
                .and_then(|v| v.as_str())
                .filter(|c| matches!(*c, "low" | "medium" | "high"))
                .unwrap_or("medium")
                .into(),
        );
        rule.severity = Some("medium".into());
        rule.keywords = r.get("keywords").and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        });
        let (floor, rest) = r
            .get("filter")
            .and_then(|v| v.as_str())
            .map(entropy_floor)
            .unwrap_or((None, false));
        if rest || r.get("components").is_some() {
            out.filtered += 1;
        }
        rule.entropy = r
            .get("entropy")
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)))
            .or(floor);
        // gitleaks judges the secret: `secretGroup` (Betterleaks: `valueGroup`),
        // else the first group.
        rule.secret_group = r
            .get("secretGroup")
            .or_else(|| r.get("valueGroup"))
            .and_then(|v| v.as_integer())
            .map(|g| g as usize)
            .filter(|&g| g > 0)
            .or((compiled.captures_len() > 1).then_some(1));
        rule.allow = (!allow.is_empty()).then_some(allow);
        rule.allow_line = (!allow_line.is_empty()).then_some(allow_line);
        out.rules.push(rule);
    }
    Ok(out)
}

/// A Betterleaks CEL `filter` as `(entropy floor, anything else in it)`.
///
/// A filter says when to *drop* a finding, so a leading
/// `entropy(finding["secret"]) < N` means "report only from entropy N up",
/// which is what a rule's `entropy` says. `<=` is read the same way; a secret
/// of exactly N is the one value treated differently. Only the leading
/// clause is read: anything after it (`|| tokenRatio(..)`, path matches) is
/// reported as the second value so the caller can count it.
fn entropy_floor(filter: &str) -> (Option<f64>, bool) {
    let re = regex::Regex::new(
        r#"^\s*entropy\(\s*finding\[\s*"secret"\s*\]\s*\)\s*<=?\s*([0-9]+(?:\.[0-9]+)?)"#,
    )
    .expect("a fixed pattern");
    let text = filter.trim();
    match re.captures(text) {
        Some(c) => {
            let floor = c[1].parse::<f64>().ok();
            let end = c.get(0).map_or(0, |m| m.end());
            (floor, !text[end..].trim().is_empty())
        }
        None => (None, !text.is_empty()),
    }
}

/// A gitleaks rule's allowlists as `(allow, allow_line, something dropped)`.
/// Regexes on the secret or the match judge the secret; on the line, the
/// line. Stopwords become one case-insensitive expression: gitleaks allows a
/// secret that contains one. An allowlist that depends on paths or commits
/// cannot be expressed, so it is dropped whole and counted: dropping only its
/// path half of an AND would allow more than gitleaks does.
fn allowlists(r: &toml::Value) -> (Vec<String>, Vec<String>, bool) {
    let mut lists: Vec<&toml::Value> = Vec::new();
    if let Some(a) = r.get("allowlist") {
        lists.push(a);
    }
    if let Some(a) = r.get("allowlists").and_then(|v| v.as_array()) {
        lists.extend(a);
    }
    let (mut allow, mut line, mut dropped) = (Vec::new(), Vec::new(), false);
    let strings = |a: &toml::Value, k: &str| -> Vec<String> {
        a.get(k)
            .and_then(|v| v.as_array())
            .map(|v| {
                v.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    for a in lists {
        if a.get("paths").is_some() || a.get("commits").is_some() {
            dropped = true;
            continue;
        }
        let target = a
            .get("regexTarget")
            .and_then(|v| v.as_str())
            .unwrap_or("secret");
        for re in strings(a, "regexes") {
            let re = ascii_classes(&re);
            if regex::RegexBuilder::new(&re)
                .size_limit(super::patternset::PATTERN_SIZE_LIMIT)
                .build()
                .is_err()
            {
                dropped = true;
                continue;
            }
            if target == "line" {
                line.push(re);
            } else {
                allow.push(re);
            }
        }
        let words = strings(a, "stopwords");
        if !words.is_empty() {
            let alt: Vec<String> = words.iter().map(|w| regex::escape(w)).collect();
            allow.push(format!("(?i)(?:{})", alt.join("|")));
        }
    }
    (allow, line, dropped)
}

/// Go's `\w`, `\d` and `\s` are ASCII; Rust's are Unicode, which under `(?i)`
/// is both a different match and a compiled size past the limit. Each becomes
/// its ASCII class (a nested class inside `[...]`, which Rust accepts), so the
/// rule matches what it matched in gitleaks. An escaped backslash is kept.
fn ascii_classes(re: &str) -> String {
    let mut out = String::with_capacity(re.len());
    let mut chars = re.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('w') => out.push_str("[0-9A-Za-z_]"),
            Some('W') => out.push_str("[^0-9A-Za-z_]"),
            Some('d') => out.push_str("[0-9]"),
            Some('D') => out.push_str("[^0-9]"),
            Some('s') => out.push_str("[\\t\\n\\f\\r ]"),
            Some('S') => out.push_str("[^\\t\\n\\f\\r ]"),
            Some(next) => {
                out.push('\\');
                out.push(next);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `rules` as an evnx rule file (version 1), after `header` comment lines.
pub fn to_rule_file(rules: &[PatternRule], header: &[String]) -> Result<String> {
    use toml::map::Map;
    use toml::Value;
    let mut list = Vec::with_capacity(rules.len());
    for r in rules {
        let mut t = Map::new();
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                t.insert(k.to_string(), v);
            }
        };
        put("id", r.id.clone().map(Value::String));
        put("name", Some(Value::String(r.name.clone())));
        put("regex", Some(Value::String(r.regex.clone())));
        put("severity", r.severity.clone().map(Value::String));
        put("confidence", r.confidence.clone().map(Value::String));
        put("entropy", r.entropy.map(Value::Float));
        put(
            "secret_group",
            r.secret_group.map(|g| Value::Integer(g as i64)),
        );
        let strings = |v: &Option<Vec<String>>| {
            v.clone()
                .map(|k| Value::Array(k.into_iter().map(Value::String).collect()))
        };
        put("allow", strings(&r.allow));
        put("allow_line", strings(&r.allow_line));
        put("names", strings(&r.names));
        put(
            "keywords",
            r.keywords
                .clone()
                .map(|k| Value::Array(k.into_iter().map(Value::String).collect())),
        );
        put("url", r.url.clone().map(Value::String));
        list.push(Value::Table(t));
    }
    let mut doc = Map::new();
    doc.insert("version".into(), Value::Integer(1));
    doc.insert("rules".into(), Value::Array(list));
    let comments: String = header.iter().map(|l| format!("# {l}\n")).collect();
    Ok(format!(
        "{comments}{}",
        toml::to_string(&Value::Table(doc))?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C13: Betterleaks' extra fields: confidence, valueGroup, skipReport,
    /// and the entropy floor of a CEL filter.
    #[test]
    fn betterleaks_filters_confidence_and_composites_convert() {
        let text = r#"
[[rules]]
id = "acme-token"
description = "Acme token"
regex = '''acme_[a-z0-9]{32}'''
confidence = "high"
keywords = ["acme_"]
filter = '''
entropy(finding["secret"]) <= 4.0
'''

[[rules]]
id = "acme-key"
regex = '''key=([a-z0-9]{32})'''
valueGroup = 1
filter = '''
entropy(finding["secret"]) < 3.5 || tokenRatio(finding["secret"]) >= 0.5
'''

[[rules]]
id = "acme-child"
regex = '''child_[a-z]{8}'''
skipReport = true

[[rules]]
id = "acme-parent"
regex = '''parent_[a-z]{8}'''
confidence = "certain"
components = ["acme-child"]
"#;
        let c = from_gitleaks(text, "betterleaks").unwrap();
        let ids: Vec<&str> = c.rules.iter().map(|r| r.id.as_deref().unwrap()).collect();
        assert_eq!(
            ids,
            [
                "betterleaks-acme-token",
                "betterleaks-acme-key",
                "betterleaks-acme-parent"
            ]
        );
        assert_eq!(c.rules[0].entropy, Some(4.0));
        assert_eq!(c.rules[0].confidence.as_deref(), Some("high"));
        assert_eq!(c.rules[1].entropy, Some(3.5));
        assert_eq!(c.rules[1].secret_group, Some(1));
        assert_eq!(
            c.rules[2].confidence.as_deref(),
            Some("medium"),
            "unknown values fall back"
        );
        assert_eq!(
            c.filtered, 2,
            "the tokenRatio clause and the composite are counted"
        );
        assert_eq!(c.skipped.len(), 1);
        assert!(c.skipped[0].1.contains("skipReport"));
    }

    #[test]
    fn an_entropy_floor_is_read_only_from_a_leading_clause() {
        assert_eq!(
            entropy_floor("entropy(finding[\"secret\"]) < 3"),
            (Some(3.0), false)
        );
        assert_eq!(
            entropy_floor("  entropy( finding[ \"secret\" ] ) <= 4.5\n"),
            (Some(4.5), false)
        );
        assert_eq!(
            entropy_floor("matchesAny(finding[\"secret\"], [\"x\"])"),
            (None, true)
        );
        assert_eq!(entropy_floor(""), (None, false));
    }

    #[test]
    fn gitleaks_rules_convert_and_what_cannot_is_listed() {
        let text = r#"
title = "x"
[[rules]]
id = "acme-key"
description = "Acme API key"
regex = '''acme_([a-z0-9]{32})'''
entropy = 3.5
keywords = ["acme_"]
[[rules.allowlists]]
stopwords = ["example"]
[[rules.allowlists]]
regexTarget = "line"
regexes = ['''test\.py''']
[[rules.allowlists]]
condition = "AND"
paths = ['''docs/''']
regexes = ['''x''']

[[rules]]
id = "needs-lookahead"
regex = '''key(?=[0-9])'''

[[rules]]
id = "pkcs12-file"
path = '''\.p12$'''
"#;
        let c = from_gitleaks(text, "gitleaks").unwrap();
        assert_eq!(c.rules.len(), 1);
        let r = &c.rules[0];
        assert_eq!(r.id.as_deref(), Some("gitleaks-acme-key"));
        assert_eq!(r.name, "Acme API key");
        assert_eq!(r.entropy, Some(3.5));
        assert_eq!(r.severity.as_deref(), Some("medium"));
        assert_eq!(c.narrowed, 1, "the paths allowlist is dropped and counted");
        assert_eq!(r.secret_group, Some(1));
        assert_eq!(
            r.allow.as_deref(),
            Some(&["(?i)(?:example)".to_string()][..])
        );
        assert_eq!(
            r.allow_line.as_deref(),
            Some(&[r"test\.py".to_string()][..])
        );
        assert_eq!(r.keywords.as_deref(), Some(&["acme_".to_string()][..]));
        let skipped: Vec<&str> = c.skipped.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(skipped, ["needs-lookahead", "pkcs12-file"]);

        // What it writes reads back as the same rules.
        let file = to_rule_file(&c.rules, &["converted".into()]).unwrap();
        assert!(file.starts_with("# converted\n"));
        #[derive(serde::Deserialize)]
        struct F {
            version: u32,
            rules: Vec<PatternRule>,
        }
        let back: F = toml::from_str(&file).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.rules, c.rules);
    }

    #[test]
    fn go_classes_become_ascii_and_escaped_backslashes_stay() {
        assert_eq!(
            ascii_classes(r"[\w.-]{0,50}\d"),
            r"[[0-9A-Za-z_].-]{0,50}[0-9]"
        );
        assert_eq!(ascii_classes(r"\s\S"), r"[\t\n\f\r ][^\t\n\f\r ]");
        assert_eq!(ascii_classes(r"\\w\.\b"), r"\\w\.\b");
        let re = regex::Regex::new(&ascii_classes(r"(?i)key[\w]{2}")).unwrap();
        assert!(re.is_match("KEY_a"));
        // Unicode letters are not `\w` in Go, so not here either.
        assert!(!re.is_match("keyéé"));
    }

    #[test]
    fn index_urls_resolve_against_the_index() {
        let base = "https://example.com/evnx-rules/main/index.toml";
        assert_eq!(
            resolve(base, "packs/gitleaks.toml"),
            "https://example.com/evnx-rules/main/packs/gitleaks.toml"
        );
        assert_eq!(
            resolve(base, "https://other/x.toml"),
            "https://other/x.toml"
        );
    }

    #[test]
    fn a_changed_or_missing_pack_is_refused_and_a_hand_written_file_is_not() {
        let d = tempfile::TempDir::new().unwrap();
        let rules = d.path().join("rules");
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::write(rules.join("pack.toml"), "version = 1\nrules = []\n").unwrap();
        std::fs::write(rules.join("mine.toml"), "anything").unwrap();
        let lock = Lock {
            version: 1,
            packs: vec![Entry {
                name: "pack".into(),
                kind: "path".into(),
                source: "pack.toml".into(),
                format: "evnx".into(),
                sha256: sha256_hex(b"version = 1\nrules = []\n"),
                rules: 0,
                added: "2026-11-07".into(),
            }],
        };
        lock.save(&lock_path(&rules)).unwrap();
        verify(&rules, ".evnx/rules").unwrap();

        std::fs::write(rules.join("pack.toml"), "version = 1\nrules = []\n# x\n").unwrap();
        let e = verify(&rules, ".evnx/rules").unwrap_err().to_string();
        assert!(
            e.contains(".evnx/rules/pack.toml changed since it was added"),
            "{e}"
        );

        std::fs::remove_file(rules.join("pack.toml")).unwrap();
        let e = verify(&rules, ".evnx/rules").unwrap_err().to_string();
        assert!(e.contains("is missing"), "{e}");
    }
}
