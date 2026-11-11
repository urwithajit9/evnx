//! The detection engine's one entry point: [`scan_text`] (C1, RM-45).
//!
//! Text in, detections out, each with its line and, for a `.env`, its
//! variable. No file walking, decoding, git or output: `commands::scan` reads
//! the files and calls this, and anything else that has text (a staged diff,
//! a structured file) can call it the same way.
//!
//! Moved from `commands::scan::runner` unchanged, so every scan golden is
//! byte-identical. What a finding becomes afterwards — the placeholder flag,
//! the public-prefix escalation, the `--severity` floor, masking — is the
//! report's business and stays in the runner.

use super::detector::{Detection, DetectorRegistry};
use crate::core::parser::{strip_export, Document, Parser, ParserConfig};

/// How to read the text: as a `.env`, through the parser, as a structured
/// file (C8), or line by line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dotenv,
    /// YAML, JSON or TOML: names and values from the parsed tree, **and** the
    /// text path's per-line rules, merged to one finding per value.
    Structured(Format),
    Text,
}

/// A structured format [`leaves`] can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Json,
    Toml,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Format::Yaml => "yaml",
            Format::Json => "json",
            Format::Toml => "toml",
        })
    }
}

/// One string value in a structured file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// Dotted, for the location: `db.password`, `servers[0].token`.
    pub path: String,
    /// The key the name detectors judge: the leaf's own (`password`), or for
    /// a list item its list's (`tokens`).
    pub key: String,
    pub value: String,
    /// 1-based, found by searching the raw text: the parsers keep no positions.
    pub line: usize,
}

/// What [`scan_text`] needs besides the text.
pub struct Context<'a> {
    /// The name locations are built from: a path as output shows it, or
    /// `<stdin>`.
    pub path: &'a str,
    pub kind: Kind,
    /// An already-parsed `.env`, so a caller that has one does not parse it
    /// twice. `None` parses `text` (expansion off).
    pub document: Option<&'a Document>,
    /// The same for a structured file: its [`leaves`]. `None` parses `text`,
    /// and a file that does not parse is read as [`Kind::Text`].
    pub leaves: Option<&'a [Leaf]>,
    /// The detectors, with their rules, disabled set and `[vars]` spec.
    pub registry: &'a DetectorRegistry,
}

/// One detection, where it was found.
pub struct Found {
    /// 1-based; for a multiline value, the line it opened on.
    pub line: usize,
    /// The `.env` key, when there is one.
    pub variable: Option<String>,
    pub detection: Detection,
}

/// Scan `text` and return every detection, at most one per value, in the
/// order they appear.
pub fn scan_text(text: &str, ctx: &Context) -> Vec<Found> {
    // ── `.env`: the real parser, not a second implementation ─────────────
    //
    // ⚠️ This used to split every line on `=` itself. A continuation line of
    // a multiline value has no `=`, so it was skipped entirely — and a PEM
    // private key, the most common multiline value there is, scanned clean
    // with exit 0. `scan.mdx` lists "private keys accidentally pasted as
    // values" as something this catches. D21.
    //
    // `core::parser` already handles multiline, quoting and `export`, and
    // eleven other modules use it. Three settings matter here:
    //
    // * `allow_expansion: false` — a scanner wants the literal text. With
    //   expansion on, `$OTHER` would be substituted, which can both hide a
    //   secret written literally and invent one that is not in the file.
    // * `allow_inline_comments: true` — `A=secret # note` yields `secret`,
    //   which is the value that would reach a program.
    // * `strict: false` — a lowercase key is still worth scanning.
    match ctx.kind {
        Kind::Text => return scan_lines(text, ctx),
        Kind::Structured(format) => return scan_structured(text, format, ctx),
        Kind::Dotenv => {}
    }
    let parsed;
    let doc = match ctx.document {
        Some(d) => d,
        None => {
            parsed = Parser::new(ParserConfig {
                allow_expansion: false,
                ..Default::default()
            })
            .parse_document(text);
            &parsed
        }
    };
    let mut out = Vec::new();
    scan_env(doc, ctx, &mut out);
    out
}

/// The text path: every non-blank line, as tokens and whole.
fn scan_lines(text: &str, ctx: &Context) -> Vec<Found> {
    let mut out = Vec::new();

    for (line_num, line) in text.lines().enumerate() {
        let line_num = line_num + 1;

        // ⚠️ Blank lines only. A commented-out secret is still a secret
        // that was committed, so `#` no longer skips the line — the
        // detectors below read the raw text either way.
        if line.trim().is_empty() {
            continue;
        }

        // Scan tokens for general text files.
        let location = format!("{}:{}", ctx.path, line_num);
        let mut detections: Vec<Detection> = Vec::new();

        for token in extract_tokens(line) {
            detections.extend(ctx.registry.scan_token(&token, &location));
        }

        // ...and the whole line, for what a token cannot carry.
        //
        // ⚠️ A line is not one value. Two unrelated credentials on one
        // line are two findings, whereas two detectors describing one
        // value are one — so these are reconciled by *value* below, not
        // collapsed into a single answer per line.
        //
        // `extract_tokens` also drops anything 20 characters or shorter,
        // so a format shorter than that is reachable only from here. An
        // AWS access key is `AKIA` plus sixteen characters — exactly 20
        // — and so arrives by this path alone.
        detections.extend(ctx.registry.scan_line(line, &location));

        for detection in one_finding_per_value(detections) {
            out.push(Found {
                line: line_num,
                variable: None,
                detection,
            });
        }
    }
    out
}

/// ⚠️ The line reported for a multiline value is the line it **opened** on,
/// matching `EnvFile::lines` and the convention parse errors already use.
/// Pointing at the fragment that happened to match would send someone to the
/// middle of a PEM block rather than to the assignment they need to edit.
fn scan_env(doc: &Document, ctx: &Context, out: &mut Vec<Found>) {
    let vars = doc.vars();
    let lines = doc.lines();

    for (key, value) in &vars {
        let line_num = lines.get(key).copied().unwrap_or(1);
        let location = format!("{}:{} ({})", ctx.path, line_num, key);
        if let Some(detection) = best(ctx.registry.scan_kv(key, value, &location)) {
            out.push(Found {
                line: line_num,
                variable: Some(key.clone()),
                detection,
            });
        }
    }

    // ⛔ The map is not the whole file. What it drops, a secret on which
    // scanned clean:
    //
    // * **A commented-out assignment.** `# STRIPE_SECRET_KEY=sk_live_…` is a
    //   comment to the parser and a leaked credential to everyone else. A key
    //   pasted into a file and then commented out is still a key that was
    //   committed.
    // * **A shadowed duplicate.** `K=sk_live_…` followed by `K=` leaves only
    //   the empty one in the map, so the real value was never offered to a
    //   detector — and "I blanked it on the line below" does not unpublish it.
    // * **A line the parser refuses.** A malformed `.env` is exactly where a
    //   half-pasted secret lives, so it is read as raw `KEY=value` text.
    //
    // ⚠️ Only what the map missed is scanned here, so a secret is not
    // reported twice. A line inside a multiline value is neither a comment
    // nor an assignment: its value was scanned whole above.
    let mut dropped: Vec<(usize, &str, &str)> = Vec::new();
    for c in &doc.comments {
        // If it looks like an assignment, keep the key so the finding can
        // name it; otherwise scan the text as a bare value.
        let body = c.text.trim();
        let (k, v) = match body.split_once('=') {
            Some((k, v)) => (k.trim().trim_start_matches("export ").trim(), v.trim()),
            None => ("", body),
        };
        dropped.push((c.line, k, v));
    }
    for e in &doc.entries {
        let shadowed = lines.get(&e.key) != Some(&e.span.line);
        if shadowed && vars.get(&e.key) != Some(&e.value) {
            dropped.push((e.span.line, &e.key, &e.value));
        }
    }
    for p in &doc.problems {
        for (i, line) in p.raw.lines().enumerate() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let k = strip_export(k).unwrap_or(k);
                dropped.push((p.span.line + i, k, v.trim()));
            }
        }
    }
    dropped.sort_by_key(|(line, _, _)| *line);

    for (line_num, key, value) in dropped {
        if value.is_empty() {
            continue;
        }
        let location = format!("{}:{} ({})", ctx.path, line_num, key);
        if let Some(detection) = best(ctx.registry.scan_kv(key, value, &location)) {
            let variable = (!key.is_empty()).then(|| key.to_string());
            out.push(Found {
                line: line_num,
                variable,
                detection,
            });
        }
    }
}

/// One value, one finding.
///
/// Every registered detector sees each key/value pair, and more than one can
/// match — `STRIPE_SECRET_KEY` fires both the pattern matcher and the
/// sensitive-key heuristic. Recording both inflated `secrets_found` (one
/// Stripe key reported as two secrets), printed the same line twice, and
/// emitted two GitHub annotations for it.
///
/// Ranking, in order:
///
/// 1. **Carries a remediation URL.** A finding that can say *where to revoke
///    this* is more useful than one that cannot, and only the named provider
///    patterns can. Confidence alone gets this wrong: the sensitive-key
///    heuristic scores `High` on any long value, so it beat the real
///    `AWS Secret Access Key` match — `Medium` on purpose, since that pattern
///    is just "40 base64-ish characters" — and a genuine AWS key was reported
///    as "Sensitive config key" with no link to IAM.
/// 2. **Higher confidence.** So `MY_PASSWORD=<long value>`, which matches only
///    the key-aware heuristic and the shapeless high-entropy fallback, keeps
///    the key-aware answer.
/// 3. **Earliest detector**, which is `RuleDetector` when the
///    project declared patterns and `PatternDetector` otherwise. A rule the
///    project wrote itself outranks a built-in heuristic that reached the
///    same confidence, because it carries a name someone chose.
///
/// The winner keeps its label and URL but takes the **highest confidence any
/// detector reported**. The two are answering different questions: the
/// pattern's `Medium` on an AWS secret key expresses doubt about *which
/// provider* a shapeless 40-character blob belongs to, not doubt about
/// whether it is a secret — and `AWS_SECRET_ACCESS_KEY` as a key name settles
/// the second question. Two independent detectors agreeing is stronger
/// evidence than either alone.
///
/// Without this, deduplication would have quietly narrowed `--severity high`:
/// a real AWS secret key would drop to `Medium` and slip under a gate that
/// used to catch it through the heuristic's separate `High` finding.
fn best(detections: Vec<Detection>) -> Option<Detection> {
    fn rank(d: &Detection) -> (bool, super::Confidence) {
        (d.action_url.is_some(), d.confidence)
    }

    let strongest = detections.iter().map(|d| d.confidence).max()?;
    let mut winner = detections.into_iter().reduce(|best, next| {
        if rank(&next) > rank(&best) {
            next
        } else {
            best
        }
    })?;
    winner.confidence = strongest;
    Some(winner)
}

/// Reconcile a line's detections by the value each one is about.
///
/// [`best`] settles *one* set of detections about *one* value.
/// That is all a `.env` line needs, because `scan_kv` hands every detector
/// the same value and there is one answer to pick. A line of source is not
/// like that: it is scanned twice, by two paths that see different strings,
/// and neither could see the other's result.
///
/// So one Stripe key in a `.py` file was reported twice:
///
/// ```text
///   ·  High-entropy string (possible secret)       low
///   ✗  Stripe Secret Key (LIVE)                    high
/// ```
///
/// The entropy heuristic reaches a non-`.env` file only through
/// `scan_token`, and the provider patterns reach it only through
/// `scan_line` — `RuleDetector::scan_token` returns `None` on purpose, so
/// that it does not report its own match twice. Each path was internally
/// consistent; nothing reconciled them with each other. One key counted as
/// two secrets, and as one `high` **and** one `low` in the severity totals
/// that `--severity` gates and CI reads.
///
/// # Why containment rather than equality
///
/// The two paths do not agree on where the value ends. `extract_tokens`
/// splits on whitespace, `=`, `:` and quotes — not on `-`, `{`, `)` or `,`
/// — so the token is often the credential plus whatever surrounds it, while
/// the pattern matched only the credential:
///
/// ```text
/// c = "prefix-sk_live_51Habc…1234-suffix"
///     token   prefix-sk_live_51Habc…1234-suffix
///     pattern         sk_live_51Habc…1234
/// ```
///
/// Equality collapses the quoted case and leaves that one duplicated, which
/// is the shape real code is full of. Two detections about the same line
/// where one value contains the other are one finding, and `best` then keeps
/// the specific answer — the named provider with its revocation URL, over
/// "high-entropy string".
///
/// ⚠️ **An empty `matched_value` is compared by equality only.** Every string
/// contains `""`, so a detection that recorded no value would otherwise
/// swallow every other finding on its line.
///
/// Two genuinely different credentials on one line stay two findings:
/// neither value contains the other.
///
/// Order is the order values were first seen, so output stays stable —
/// grouping through a `HashMap` would reorder findings between runs of the
/// same scan.
fn one_finding_per_value(detections: Vec<Detection>) -> Vec<Detection> {
    let mut groups: Vec<Vec<Detection>> = Vec::new();
    for detection in detections {
        match groups
            .iter_mut()
            .find(|g| same_value(&g[0].matched_value, &detection.matched_value))
        {
            Some(group) => group.push(detection),
            None => groups.push(vec![detection]),
        }
    }

    groups.into_iter().filter_map(best).collect()
}

/// Do these two detections describe the same value?
fn same_value(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    a.contains(b) || b.contains(a)
}

/// A structured file (C8): each string leaf by name and value, then the text
/// path over the raw lines, which also reads comments. A value both found is
/// one finding, keeping the key path the tree gave it.
fn scan_structured(text: &str, format: Format, ctx: &Context) -> Vec<Found> {
    let parsed;
    let leaves = match ctx.leaves {
        Some(l) => l,
        None => match leaves(text, format) {
            Some(l) => {
                parsed = l;
                &parsed
            }
            None => return scan_lines(text, ctx),
        },
    };
    let mut all = Vec::new();
    for leaf in leaves {
        // D-36: a `regex = "…"` value is a rule's own declaration (in
        // `.evnx.toml`, a rule file, a pack). Only the text path can tell, by
        // seeing the `regex =` around it, so the value is left to that path.
        if leaf.key == "regex" {
            continue;
        }
        let location = format!("{}:{} ({})", ctx.path, leaf.line, leaf.path);
        // A sentence or a script is not judged by entropy as a whole:
        // `heuristic::looks_like_key_material` refuses whitespace (C4).
        let detections = ctx.registry.scan_kv(&leaf.key, &leaf.value, &location);
        if let Some(detection) = best(detections) {
            all.push(Found {
                line: leaf.line,
                variable: Some(leaf.path.clone()),
                detection,
            });
        }
    }
    all.extend(scan_lines(text, ctx));

    // One finding per value per line, in first-seen order.
    let mut groups: Vec<Vec<Found>> = Vec::new();
    for f in all {
        match groups.iter_mut().find(|g| {
            g[0].line == f.line
                && same_value(&g[0].detection.matched_value, &f.detection.matched_value)
        }) {
            Some(g) => g.push(f),
            None => groups.push(vec![f]),
        }
    }
    let mut out: Vec<Found> = groups
        .into_iter()
        .filter_map(|g| {
            let line = g[0].line;
            let variable = g.iter().find_map(|f| f.variable.clone());
            // ⚠️ A rule with `names` fired for this leaf because its key
            // matched. The text path reads the same value with no key, so it
            // can only report the unconstrained rule the named one exists to
            // beat (`secret_key: sk_live_…` is the project's, not Stripe's);
            // left in the group, that rule's remediation URL outranked it.
            // `.env` never had the problem: it has no text path for a line
            // that parsed (F14-3).
            let keyed = |f: &Found| {
                f.variable.is_some() && ctx.registry.is_name_constrained(&f.detection.rule_id)
            };
            let detections: Vec<Detection> = if g.iter().any(keyed) {
                g.into_iter().filter(keyed).map(|f| f.detection).collect()
            } else {
                g.into_iter().map(|f| f.detection).collect()
            };
            best(detections).map(|detection| Found {
                line,
                variable,
                detection,
            })
        })
        .collect();
    out.sort_by_key(|f| f.line);
    out
}

/// Every string leaf of `text` read as `format`, or `None` when it does not
/// parse — a template, a Helm chart with `{{ }}` — which the caller reads as
/// text instead. Numbers, booleans and nulls are not values a secret is in.
pub fn leaves(text: &str, format: Format) -> Option<Vec<Leaf>> {
    use serde::Deserialize;
    let mut out = Vec::new();
    match format {
        Format::Json => walk_json(&serde_json::from_str(text).ok()?, "", "", &mut out),
        Format::Toml => {
            let table: toml::Table = toml::from_str(text).ok()?;
            walk_toml(&toml::Value::Table(table), "", "", &mut out)
        }
        Format::Yaml => {
            // Every document in a `---`-separated stream.
            for doc in serde_yaml::Deserializer::from_str(text) {
                walk_yaml(&serde_yaml::Value::deserialize(doc).ok()?, "", "", &mut out);
            }
        }
    }
    let mut from = 0;
    for leaf in &mut out {
        leaf.line = line_of(text, &leaf.key, &leaf.value, from);
        from = leaf.line.saturating_sub(1);
    }
    Some(out)
}

/// One piece of text inside a Jupyter notebook: a cell's source or one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookChunk {
    /// Where it sits, for the location: `cells[2].source`,
    /// `cells[2].outputs[0].text`.
    pub path: String,
    /// The text after JSON unescaping, lines joined as the notebook stores them.
    pub text: String,
}

/// The text a notebook holds (F14-6): every cell's `source` (code and
/// markdown) and every output's `text`, `data["text/plain"]` and `traceback`,
/// unescaped. `None` when `text` is not a JSON notebook with a `cells` array
/// (malformed, or nbformat 3), which the caller reads as plain text.
///
/// Binary output (`image/png` and the like) is not text and is not read.
pub fn notebook_chunks(text: &str) -> Option<Vec<NotebookChunk>> {
    use serde_json::Value;
    // A notebook's strings are either one string or a list of lines.
    fn joined(v: &Value, sep: &str) -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(sep),
            ),
            _ => None,
        }
    }
    let root: Value = serde_json::from_str(text).ok()?;
    let cells = root.get("cells")?.as_array()?;
    let mut out = Vec::new();
    let mut add = |path: String, text: Option<String>| {
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            out.push(NotebookChunk { path, text });
        }
    };
    for (i, cell) in cells.iter().enumerate() {
        add(
            format!("cells[{i}].source"),
            cell.get("source").and_then(|v| joined(v, "")),
        );
        let Some(outputs) = cell.get("outputs").and_then(Value::as_array) else {
            continue;
        };
        for (j, output) in outputs.iter().enumerate() {
            let base = format!("cells[{i}].outputs[{j}]");
            add(
                format!("{base}.text"),
                output.get("text").and_then(|v| joined(v, "")),
            );
            add(
                format!("{base}.data.text/plain"),
                output
                    .get("data")
                    .and_then(|d| d.get("text/plain"))
                    .and_then(|v| joined(v, "")),
            );
            // A traceback is a list of whole lines without their newlines.
            add(
                format!("{base}.traceback"),
                output.get("traceback").and_then(|v| joined(v, "\n")),
            );
        }
    }
    Some(out)
}

fn push(path: &str, key: &str, value: &str, out: &mut Vec<Leaf>) {
    out.push(Leaf {
        path: path.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        line: 0,
    });
}

fn child(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn walk_json(v: &serde_json::Value, path: &str, key: &str, out: &mut Vec<Leaf>) {
    match v {
        serde_json::Value::String(s) => push(path, key, s, out),
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_json(item, &format!("{path}[{i}]"), key, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, item) in map {
                walk_json(item, &child(path, k), k, out);
            }
        }
        _ => {}
    }
}

fn walk_toml(v: &toml::Value, path: &str, key: &str, out: &mut Vec<Leaf>) {
    match v {
        toml::Value::String(s) => push(path, key, s, out),
        toml::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_toml(item, &format!("{path}[{i}]"), key, out);
            }
        }
        toml::Value::Table(map) => {
            for (k, item) in map {
                walk_toml(item, &child(path, k), k, out);
            }
        }
        _ => {}
    }
}

fn walk_yaml(v: &serde_yaml::Value, path: &str, key: &str, out: &mut Vec<Leaf>) {
    use serde_yaml::Value;
    match v {
        Value::String(s) => push(path, key, s, out),
        Value::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_yaml(item, &format!("{path}[{i}]"), key, out);
            }
        }
        Value::Mapping(map) => {
            for (k, item) in map {
                // A key is usually a string; a number or a bool is named as written.
                let k = match k {
                    Value::String(s) => s.clone(),
                    other => serde_yaml::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                walk_yaml(item, &child(path, &k), &k, out);
            }
        }
        Value::Tagged(t) => walk_yaml(&t.value, path, key, out),
        _ => {}
    }
}

/// The 1-based line a leaf is on. The parsers keep no positions, so this
/// searches the raw text, from `from` on (leaves come in file order): the first
/// line holding the key and the value's first line; else the first holding
/// the value (a list item's key is its parent's, on the line above); else the
/// first holding the key; else line 1.
///
/// shortcut: a guess when the same key and value repeat, or a value is
/// escaped differently in the source (`\u00e9` in JSON); use a span-keeping
/// parser if a wrong line is ever reported.
fn line_of(text: &str, key: &str, value: &str, from: usize) -> usize {
    let first = value.lines().next().unwrap_or("").trim();
    let lines: Vec<&str> = text.lines().collect();
    let find = |pred: &dyn Fn(&str) -> bool| {
        (from..lines.len())
            .chain(0..from.min(lines.len()))
            .find(|&i| pred(lines[i]))
            .map(|i| i + 1)
    };
    find(&|l| l.contains(key) && !first.is_empty() && l.contains(first))
        .or_else(|| find(&|l| !first.is_empty() && l.contains(first)))
        .or_else(|| find(&|l| !key.is_empty() && l.contains(key)))
        .unwrap_or(1)
}

/// Extract tokens from a line for scanning.
///
/// Splits on common separators and filters by minimum length.
///
/// # Arguments
///
/// * `line` - Line of text to extract tokens from
///
/// # Returns
///
/// Vector of token strings (min 20 chars by default)
fn extract_tokens(line: &str) -> Vec<String> {
    line.split(|c: char| c.is_whitespace() || c == '=' || c == ':' || c == '"' || c == '\'')
        .filter(|t| t.len() > 20)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C8: dotted paths through maps and lists, string leaves only, every
    /// document of a YAML stream, and the line each leaf is on.
    #[test]
    fn leaves_walk_paths_and_find_lines() {
        let yaml = "a:\n  b: one\n  n: 3\n  list:\n    - two\n---\nc: three\n";
        let got: Vec<(String, String, String, usize)> = leaves(yaml, Format::Yaml)
            .unwrap()
            .into_iter()
            .map(|l| (l.path, l.key, l.value, l.line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a.b".into(), "b".into(), "one".into(), 2),
                ("a.list[0]".into(), "list".into(), "two".into(), 5),
                ("c".into(), "c".into(), "three".into(), 7),
            ]
        );
        let json = leaves("{\"x\": {\"y\": [\"v\"]}, \"z\": null}", Format::Json).unwrap();
        assert_eq!(
            (json[0].path.as_str(), json[0].key.as_str()),
            ("x.y[0]", "y")
        );
        assert_eq!(json.len(), 1);
        assert!(leaves("a = [", Format::Toml).is_none());
    }

    /// A repeated key is found on its own line, not the first one.
    #[test]
    fn line_of_moves_forward_through_repeats() {
        let yaml = "one:\n  password: aaa\ntwo:\n  password: bbb\n";
        let l = leaves(yaml, Format::Yaml).unwrap();
        assert_eq!((l[0].line, l[1].line), (2, 4));
    }

    #[test]
    fn test_extract_tokens() {
        let line = "API_KEY=sk_live_1234567890abcdefghijklmnop short";
        let tokens = extract_tokens(line);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].len() > 20);
    }
}
