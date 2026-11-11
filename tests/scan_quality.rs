//! The scanner's quality gate (RM-70): precision and recall over a labelled
//! corpus, compared with a committed baseline, on every CI run.
//!
//! "The scanner got better" is a claim until it is a number. This runs the real
//! `evnx scan` binary over a corpus and reports, per family and overall, how
//! many of the secrets it must find it found (recall) and how many of its
//! findings were right (precision).
//!
//! # The corpus
//!
//! `tests/scan_corpus/`:
//!
//! * `corpus.toml`: one `[[case]]` per file. A case with an `expect` list is a
//!   file whose secrets must be found, each labelled with its rule id and line.
//!   A case with none is a look-alike (a documentation key, a hash, a lockfile
//!   digest, an image, a placeholder) and any finding in it is a false positive.
//! * `cases/<id>.tpl`: the file, as a template.
//! * `baseline.json`: the numbers the gate compares with.
//!
//! # No secret is ever committed
//!
//! A template holds `{{...}}` markers, never a value. They are filled here,
//! from parts, so the repository holds nothing that looks like a live secret:
//! GitHub push protection and `evnx scan` on this repository see none, and
//! `templates_hold_no_secret_shaped_values` fails if one is ever pasted in.
//!
//! | marker                     | becomes                                           |
//! |----------------------------|---------------------------------------------------|
//! | `{{rule:ID}}`, `{{rule:ID#N}}` | the Nth positive of `tests/fixtures/rules/ID.toml` (parts joined) |
//! | `{{near:ID}}`, `{{near:ID#N}}` | the Nth near miss of the same file                |
//! | `{{cat:a\|b\|c}}`          | the parts joined                                  |
//! | `{{gen:SET:LEN:SEED}}`     | LEN characters of `alnum`, `hex`, `b64`, `upper`  |
//! | `{{uuid:SEED}}`            | a UUID                                            |
//! | `{{hash:LEN:SEED}}`        | the first LEN hex digits of a SHA-256             |
//! | `{{sri:SEED}}`             | `sha512-<base64>`, a lockfile integrity value     |
//!
//! Every value is derived from its SEED, so a run is the same on every machine.
//!
//! # The gate
//!
//! It fails when **recall** falls below the baseline overall, in any family or
//! in any format, and when **precision** falls more than
//! [`PRECISION_TOLERANCE`] below it. Either way it prints the table and every
//! miss and every false positive.
//!
//! When a change is meant to move the numbers, update the baseline on purpose
//! and commit it with the change:
//!
//! ```text
//! EVNX_UPDATE_SCAN_BASELINE=1 cargo test --test scan_quality -- --nocapture
//! ```
//!
//! ⚠️ Raising recall by lowering precision, or the reverse, is a decision. The
//! baseline diff is where a reviewer sees it.

use assert_cmd::cargo::cargo_bin_cmd;
use base64::Engine as _;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// How far a precision ratio may fall below the baseline before the gate fails.
/// Recall has no tolerance: a missed secret is the failure that matters.
const PRECISION_TOLERANCE: f64 = 0.02;

const UPDATE_VAR: &str = "EVNX_UPDATE_SCAN_BASELINE";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn corpus_dir() -> PathBuf {
    root().join("scan_corpus")
}

// ── The corpus ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Manifest {
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    family: String,
    format: String,
    r#as: String,
    #[serde(default)]
    expect: Vec<Expect>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Expect {
    line: usize,
    rule: String,
    /// Other rule ids that still count as finding it. Used where the secret is
    /// reported but a judgement call decides which rule speaks (a JWT under a
    /// `*_TOKEN` name is the name rule's), so the gate watches that the value
    /// is found rather than which of two right rules says so.
    #[serde(default)]
    also: Vec<String>,
}

fn manifest() -> Manifest {
    let path = corpus_dir().join("corpus.toml");
    toml::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn template(case: &Case) -> String {
    let path = corpus_dir().join("cases").join(format!("{}.tpl", case.id));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        // A checkout on Windows may have turned LF into CRLF.
        .replace("\r\n", "\n")
}

// ── Filling the templates ────────────────────────────────────────────────

#[derive(Deserialize)]
struct RuleFixture {
    positive: Vec<Vec<String>>,
    near_miss: Vec<Vec<String>>,
}

fn fixture_value(id: &str, index: usize, near: bool) -> String {
    let path = root().join("fixtures/rules").join(format!("{id}.toml"));
    let f: RuleFixture = toml::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let list = if near { &f.near_miss } else { &f.positive };
    list.get(index)
        .unwrap_or_else(|| panic!("{id}: no fixture value #{index}"))
        .concat()
}

/// `len` bytes derived from `seed`: SHA-256 in counter mode.
fn stream(seed: &str, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter = 0u32;
    while out.len() < len {
        let mut h = Sha256::new();
        h.update(seed.as_bytes());
        h.update(counter.to_be_bytes());
        out.extend_from_slice(&h.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn generate(set: &str, len: usize, seed: &str) -> String {
    let alphabet: &[u8] = match set {
        "alnum" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
        "hex" => b"0123456789abcdef",
        "b64" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
        "upper" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        other => panic!("unknown character set {other:?}"),
    };
    stream(seed, len)
        .into_iter()
        .map(|b| alphabet[usize::from(b) % alphabet.len()] as char)
        .collect()
}

fn marker(kind: &str, args: &str) -> String {
    let parts: Vec<&str> = args.split(':').collect();
    match kind {
        "rule" | "near" => {
            let (id, n) = match args.split_once('#') {
                Some((id, n)) => (id, n.parse().expect("#N is a number")),
                None => (args, 0),
            };
            fixture_value(id, n, kind == "near")
        }
        "cat" => args.split('|').collect(),
        "gen" => generate(parts[0], parts[1].parse().unwrap(), parts[2]),
        "uuid" => {
            let h = hex(&stream(args, 16));
            format!(
                "{}-{}-4{}-a{}-{}",
                &h[0..8],
                &h[8..12],
                &h[13..16],
                &h[17..20],
                &h[20..32]
            )
        }
        "hash" => {
            let digest = hex(&Sha256::digest(parts[1].as_bytes()));
            digest[..parts[0].parse().unwrap()].to_string()
        }
        "sri" => format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(args.as_bytes()))
        ),
        other => panic!("unknown marker {other:?}"),
    }
}

fn markers() -> Regex {
    Regex::new(r"\{\{(rule|near|cat|gen|uuid|hash|sri):([^{}]*)\}\}").unwrap()
}

fn fill(template: &str) -> String {
    markers()
        .replace_all(template, |c: &regex::Captures| marker(&c[1], &c[2]))
        .into_owned()
}

// ── Running the scanner ──────────────────────────────────────────────────

/// One finding, as the gate compares it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Found {
    line: usize,
    rule: String,
    variable: Option<String>,
}

/// `evnx scan` over `dir`, run from inside it with no personal rules, as a
/// script would: findings by case id.
fn scan(dir: &Path, cases: &[Case]) -> BTreeMap<String, Vec<Found>> {
    let config = dir.join(".evnx-test-config");
    std::fs::create_dir_all(&config).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("EVNX_CONFIG_DIR", &config)
        .args(["scan", ".", "--format", "json", "--exit-zero"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "scan failed\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("scan output is not JSON: {e}\n{stdout}"));

    let mut by_case: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for f in json["findings"].as_array().unwrap() {
        // "<path>:<line> (<variable>)", with `\` for `/` on Windows.
        let location = f["location"].as_str().unwrap().replace('\\', "/");
        let hit = cases.iter().find_map(|c| {
            let needle = format!("{}/{}:", c.id, c.r#as);
            let at = location.find(&needle)?;
            if at != 0 && !location[..at].ends_with('/') {
                return None;
            }
            let digits: String = location[at + needle.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            Some((c.id.clone(), digits.parse::<usize>().ok()?))
        });
        let Some((id, line)) = hit else {
            panic!("a finding outside every case: {location}");
        };
        by_case.entry(id).or_default().push(Found {
            line,
            rule: f["rule_id"].as_str().unwrap().to_string(),
            variable: f["variable"].as_str().map(str::to_string),
        });
    }
    by_case
}

fn write_corpus(dir: &Path, manifest: &Manifest) {
    for case in &manifest.case {
        let folder = dir.join(&case.id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(&case.r#as), fill(&template(case))).unwrap();
    }
}

// ── Scoring ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Score {
    tp: usize,
    fp: usize,
    #[serde(rename = "fn")]
    fn_: usize,
    precision: f64,
    recall: f64,
}

impl Score {
    fn from_counts(tp: usize, fp: usize, fn_: usize) -> Score {
        let ratio = |num: usize, den: usize| {
            if den == 0 {
                1.0
            } else {
                // Four places, so the committed file has no float noise.
                (num as f64 / den as f64 * 10_000.0).round() / 10_000.0
            }
        };
        Score {
            tp,
            fp,
            fn_,
            precision: ratio(tp, tp + fp),
            recall: ratio(tp, tp + fn_),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Report {
    version: u32,
    overall: Score,
    families: BTreeMap<String, Score>,
    formats: BTreeMap<String, Score>,
}

/// Every miss and every false positive, for the reader of a failure.
struct Detail {
    missed: Vec<String>,
    false_positives: Vec<String>,
}

fn score(manifest: &Manifest, found: &BTreeMap<String, Vec<Found>>) -> (Report, Detail) {
    #[derive(Default, Clone, Copy)]
    struct Counts {
        tp: usize,
        fp: usize,
        fn_: usize,
    }
    let mut overall = Counts::default();
    let mut families: BTreeMap<String, Counts> = BTreeMap::new();
    let mut formats: BTreeMap<String, Counts> = BTreeMap::new();
    let mut detail = Detail {
        missed: Vec::new(),
        false_positives: Vec::new(),
    };

    for case in &manifest.case {
        // A finding is right when an expectation on its line names its rule.
        let accepts = |e: &Expect, line: usize, rule: &str| {
            e.line == line && (e.rule == rule || e.also.iter().any(|a| a == rule))
        };
        let got: BTreeMap<(usize, String), Option<String>> = found
            .get(&case.id)
            .into_iter()
            .flatten()
            .map(|f| ((f.line, f.rule.clone()), f.variable.clone()))
            .collect();

        let mut c = Counts::default();
        for e in &case.expect {
            let (line, rule) = (&e.line, &e.rule);
            if got.keys().any(|(l, r)| accepts(e, *l, r)) {
                c.tp += 1;
            } else {
                c.fn_ += 1;
                let reported: Vec<&str> = got
                    .keys()
                    .filter(|(l, _)| l == line)
                    .map(|(_, r)| r.as_str())
                    .collect();
                detail.missed.push(format!(
                    "{}/{}:{line}  expected {rule}  (reported there: {})",
                    case.id,
                    case.r#as,
                    if reported.is_empty() {
                        "nothing".to_string()
                    } else {
                        reported.join(", ")
                    }
                ));
            }
        }
        for ((line, rule), variable) in &got {
            if !case.expect.iter().any(|e| accepts(e, *line, rule)) {
                c.fp += 1;
                detail.false_positives.push(format!(
                    "{}/{}:{line}  {rule}{}",
                    case.id,
                    case.r#as,
                    variable
                        .as_ref()
                        .map(|v| format!("  ({v})"))
                        .unwrap_or_default()
                ));
            }
        }
        for tally in [
            &mut overall,
            families.entry(case.family.clone()).or_default(),
            formats.entry(case.format.clone()).or_default(),
        ] {
            tally.tp += c.tp;
            tally.fp += c.fp;
            tally.fn_ += c.fn_;
        }
    }

    let to_score = |c: &Counts| Score::from_counts(c.tp, c.fp, c.fn_);
    let report = Report {
        version: 1,
        overall: to_score(&overall),
        families: families
            .iter()
            .map(|(k, c)| (k.clone(), to_score(c)))
            .collect(),
        formats: formats
            .iter()
            .map(|(k, c)| (k.clone(), to_score(c)))
            .collect(),
    };
    (report, detail)
}

// ── The gate ─────────────────────────────────────────────────────────────

/// Why the current numbers fail `baseline`; empty when they pass.
fn regressions(now: &Report, baseline: &Report) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |scope: &str, name: &str, now: Option<&Score>, base: &Score| {
        let label = if name.is_empty() {
            scope.to_string()
        } else {
            format!("{scope} {name}")
        };
        let Some(now) = now else {
            out.push(format!(
                "{label}: in the baseline, not in the corpus any more"
            ));
            return;
        };
        // Compared from the counts, not the stored ratios.
        let base = Score::from_counts(base.tp, base.fp, base.fn_);
        if now.recall + 1e-9 < base.recall {
            out.push(format!(
                "{label}: RECALL fell {:.4} -> {:.4}",
                base.recall, now.recall
            ));
        }
        if now.precision + PRECISION_TOLERANCE < base.precision {
            out.push(format!(
                "{label}: PRECISION fell {:.4} -> {:.4} (tolerance {PRECISION_TOLERANCE})",
                base.precision, now.precision
            ));
        }
    };
    check("overall", "", Some(&now.overall), &baseline.overall);
    for (name, base) in &baseline.families {
        check("family", name, now.families.get(name), base);
    }
    for (name, base) in &baseline.formats {
        check("format", name, now.formats.get(name), base);
    }
    for name in now
        .families
        .keys()
        .filter(|k| !baseline.families.contains_key(*k))
    {
        out.push(format!("family {name}: not in the baseline; update it"));
    }
    for name in now
        .formats
        .keys()
        .filter(|k| !baseline.formats.contains_key(*k))
    {
        out.push(format!("format {name}: not in the baseline; update it"));
    }
    out
}

fn table(now: &Report, baseline: Option<&Report>) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<18} {:>4} {:>4} {:>4}  {:>9} {:>9}   {:>9} {:>9}",
        "", "TP", "FP", "FN", "precision", "recall", "base prec", "base rec"
    );
    let mut row = |name: String, now: &Score, base: Option<&Score>| {
        let (bp, br) = match base {
            Some(b) => {
                let b = Score::from_counts(b.tp, b.fp, b.fn_);
                (format!("{:.4}", b.precision), format!("{:.4}", b.recall))
            }
            None => ("-".into(), "-".into()),
        };
        let _ = writeln!(
            s,
            "{name:<18} {:>4} {:>4} {:>4}  {:>9.4} {:>9.4}   {bp:>9} {br:>9}",
            now.tp, now.fp, now.fn_, now.precision, now.recall
        );
    };
    row("OVERALL".into(), &now.overall, baseline.map(|b| &b.overall));
    for (name, score) in &now.families {
        row(
            format!("family {name}"),
            score,
            baseline.and_then(|b| b.families.get(name)),
        );
    }
    for (name, score) in &now.formats {
        row(
            format!("format {name}"),
            score,
            baseline.and_then(|b| b.formats.get(name)),
        );
    }
    s
}

/// The same table for a GitHub job summary.
fn markdown(now: &Report) -> String {
    let mut s = String::from("### Scan quality (RM-70)\n\n| scope | TP | FP | FN | precision | recall |\n|---|--:|--:|--:|--:|--:|\n");
    let mut row = |name: String, c: &Score| {
        let _ = writeln!(
            s,
            "| {name} | {} | {} | {} | {:.4} | {:.4} |",
            c.tp, c.fp, c.fn_, c.precision, c.recall
        );
    };
    row("**overall**".into(), &now.overall);
    for (name, c) in &now.families {
        row(format!("family {name}"), c);
    }
    for (name, c) in &now.formats {
        row(format!("format {name}"), c);
    }
    s
}

fn baseline_path() -> PathBuf {
    corpus_dir().join("baseline.json")
}

fn read_baseline() -> Option<Report> {
    let text = std::fs::read_to_string(baseline_path()).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("baseline.json: {e}")))
}

/// Run the whole corpus: its report, and every miss and false positive.
fn measure() -> (Report, Detail) {
    let manifest = manifest();
    let dir = tempfile::TempDir::new().unwrap();
    write_corpus(dir.path(), &manifest);
    let found = scan(dir.path(), &manifest.case);
    score(&manifest, &found)
}

#[test]
fn precision_and_recall_hold_against_the_baseline() {
    let (now, detail) = measure();
    let baseline = read_baseline();

    let mut text = table(&now, baseline.as_ref());
    if !detail.missed.is_empty() {
        let _ = write!(
            text,
            "\nMISSED ({}):\n  {}\n",
            detail.missed.len(),
            detail.missed.join("\n  ")
        );
    }
    if !detail.false_positives.is_empty() {
        let _ = write!(
            text,
            "\nFALSE POSITIVES ({}):\n  {}\n",
            detail.false_positives.len(),
            detail.false_positives.join("\n  ")
        );
    }
    println!("\n{text}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            use std::io::Write as _;
            let _ = writeln!(f, "{}", markdown(&now));
        }
    }

    if std::env::var_os(UPDATE_VAR).is_some_and(|v| !v.is_empty() && v != "0") {
        let mut json = serde_json::to_string_pretty(&now).unwrap();
        json.push('\n');
        std::fs::write(baseline_path(), json).unwrap();
        println!("baseline written: {}", baseline_path().display());
        return;
    }

    let baseline = baseline.unwrap_or_else(|| {
        panic!(
            "tests/scan_corpus/baseline.json is missing. Create it with \
             {UPDATE_VAR}=1 cargo test --test scan_quality -- --nocapture\n{text}"
        )
    });
    let failures = regressions(&now, &baseline);
    assert!(
        failures.is_empty(),
        "the scanner got worse on the labelled corpus:\n  {}\n\n{text}\n\
         If this is intended, say why and update the baseline:\n  \
         {UPDATE_VAR}=1 cargo test --test scan_quality -- --nocapture",
        failures.join("\n  ")
    );
    if now.overall.recall > baseline.overall.recall
        || now.overall.precision > baseline.overall.precision
    {
        println!(
            "The numbers are better than the baseline. Raise it so the gain is kept:\n  \
             {UPDATE_VAR}=1 cargo test --test scan_quality -- --nocapture"
        );
    }
}

// ── The corpus itself ────────────────────────────────────────────────────

/// A file name every platform accepts: Windows refuses a trailing dot or space,
/// `<>:"|?*`, control characters and the reserved device names.
fn portable(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()));
    !name.is_empty()
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.contains(|c: char| "<>:\"|?*/\\".contains(c) || c.is_control())
        && !reserved
}

#[test]
fn the_manifest_and_the_templates_agree() {
    let manifest = manifest();
    let mut ids = BTreeSet::new();
    let known: BTreeSet<String> = evnx::core::secretscan::rules::catalogue_ids()
        .into_iter()
        .collect();

    let leftover = Regex::new(r"\{\{[a-z]+:").unwrap();
    for case in &manifest.case {
        assert!(
            ids.insert(case.id.to_ascii_lowercase()),
            "{}: a duplicate id (ids are folder names, and Windows ignores case)",
            case.id
        );
        assert!(
            portable(&case.id),
            "{}: not a portable folder name",
            case.id
        );
        assert!(
            portable(&case.r#as),
            "{}: `as` is not a portable file name",
            case.id
        );
        let lines = template(case).lines().count();
        for e in &case.expect {
            assert!(
                (1..=lines).contains(&e.line),
                "{}: expects line {} of {lines}",
                case.id,
                e.line
            );
            assert!(
                known.contains(&e.rule),
                "{}: expects {:?}, which is not a rule or check evnx has",
                case.id,
                e.rule
            );
        }
        // Every marker must be one the filler knows, and none may survive it.
        assert!(
            !leftover.is_match(&fill(&template(case))),
            "{}: a marker was left unfilled (a typo in its name?)",
            case.id
        );
    }

    let on_disk: BTreeSet<String> = std::fs::read_dir(corpus_dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let listed: BTreeSet<String> = manifest
        .case
        .iter()
        .map(|c| format!("{}.tpl", c.id))
        .collect();
    assert_eq!(
        on_disk, listed,
        "cases/ and corpus.toml list different files"
    );
}

/// The corpus is only safe to commit if no template holds a secret. With the
/// markers taken out, no line may match a built-in rule, and none may carry a
/// long unbroken run of key-like characters.
///
/// ⚠️ Not a scan of the templates by `evnx scan`: a name rule reports any
/// value under `*_TOKEN`, a marker included, which says nothing about whether
/// the template holds a secret.
#[test]
fn templates_hold_no_secret_shaped_values() {
    use evnx::core::secretscan::rules::builtin_rules;
    use evnx::core::secretscan::PatternSet;

    let rules = builtin_rules();
    let set = PatternSet::compile(&rules).unwrap();
    let long_run = Regex::new(r"[A-Za-z0-9+/_\-]{40,}").unwrap();
    let mut problems = Vec::new();
    for case in &manifest().case {
        let bare = markers().replace_all(&template(case), "").into_owned();
        for (n, line) in bare.lines().enumerate() {
            let at = format!("{}.tpl:{}", case.id, n + 1);
            for m in set.find_all(line) {
                problems.push(format!("{at}: matches rule {}", m.id));
            }
            if long_run.is_match(line) {
                problems.push(format!("{at}: a run of 40+ key-like characters"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "a template holds a secret-shaped value; build it from markers instead:\n  {}",
        problems.join("\n  ")
    );
}

// ── The harness itself ───────────────────────────────────────────────────

#[test]
fn generated_values_are_stable_and_shaped_as_asked() {
    assert_eq!(fill("{{gen:hex:12:s}}"), fill("{{gen:hex:12:s}}"));
    assert_ne!(fill("{{gen:hex:12:s}}"), fill("{{gen:hex:12:t}}"));
    assert_eq!(fill("{{gen:alnum:50:s}}").len(), 50);
    assert_eq!(fill("{{hash:40:s}}").len(), 40);
    assert_eq!(fill("{{cat:a|b|c}}"), "abc");
    let uuid = fill("{{uuid:s}}");
    assert_eq!(uuid.len(), 36);
    assert_eq!(uuid.matches('-').count(), 4);
    assert!(fill("{{sri:s}}").starts_with("sha512-"));
    // Text that is not a marker is left alone: a Helm template is not ours.
    assert_eq!(fill("{{ .Values.x }}"), "{{ .Values.x }}");
}

#[test]
fn the_gate_fails_on_lower_recall_and_on_a_precision_drop_beyond_tolerance() {
    let base = Report {
        version: 1,
        overall: Score::from_counts(10, 0, 0),
        ..Report::default()
    };
    assert!(regressions(&base, &base).is_empty());

    let lost_one = Report {
        overall: Score::from_counts(9, 0, 1),
        ..base.clone()
    };
    assert!(regressions(&lost_one, &base)[0].contains("RECALL"));

    let noisy = Report {
        overall: Score::from_counts(10, 2, 0),
        ..base.clone()
    };
    assert!(regressions(&noisy, &base)[0].contains("PRECISION"));

    // Within the tolerance: 100 right, 1 wrong is 0.9901.
    let big_base = Report {
        overall: Score::from_counts(100, 0, 0),
        ..Report::default()
    };
    let slightly = Report {
        overall: Score::from_counts(100, 1, 0),
        ..Report::default()
    };
    assert!(regressions(&slightly, &big_base).is_empty());
}

#[test]
fn portable_names() {
    for good in [
        ".env",
        ".env.production",
        "Dockerfile",
        "docker-compose.yml",
        "id_deploy",
    ] {
        assert!(portable(good), "{good}");
    }
    for bad in [
        "trailing.",
        "space ",
        "a:b",
        "CON",
        "nul.txt",
        "COM1",
        "a|b",
        "",
    ] {
        assert!(!portable(bad), "{bad:?}");
    }
}
