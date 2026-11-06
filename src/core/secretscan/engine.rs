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

/// How to read the text: as a `.env`, through the parser, or line by line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dotenv,
    Text,
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
    if ctx.kind == Kind::Text {
        return scan_lines(text, ctx);
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
    /// Do these two detections describe the same value?
    fn same_value(a: &str, b: &str) -> bool {
        if a.is_empty() || b.is_empty() {
            return a == b;
        }
        a.contains(b) || b.contains(a)
    }

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

    #[test]
    fn test_extract_tokens() {
        let line = "API_KEY=sk_live_1234567890abcdefghijklmnop short";
        let tokens = extract_tokens(line);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].len() > 20);
    }
}
