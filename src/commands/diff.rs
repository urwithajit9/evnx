use crate::core::mask::{Disclosure, ValueState};
use crate::core::Parser;
use crate::docs;
use crate::utils::string::pluralize;
use crate::utils::ui;
use crate::utils::ui::glyph;
/// Diff command - compare .env and .env.example
///
/// Shows missing, extra, and different variables between two env files
///
///  Features: Exit codes for CI/CD, auto-redaction of sensitive values
///  Features: Key order preservation, --ignore-keys filtering
/// Features: JSON statistics, interactive merge mode
use anyhow::Result;
use colored::*;
use indexmap::IndexMap; // Preserve insertion order
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ─────────────────────────────────────────────────────────────
// Data Structures (enhanced with redaction + stats)
// ─────────────────────────────────────────────────────────────

/// What `diff` compares (I-078, owner 2026-10-21).
///
/// ⚠️ Decided by **how the right-hand side was chosen**, never by reading the
/// files: `--against NAME` names an environment, while the default
/// `.env.example`, `[defaults] example` and `--example` name a template.
/// A template holds placeholders, so every filled-in value differs from it by
/// construction — comparing them made `evnx diff` exit 1 on every real
/// project, and the docs called that column noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Compare {
    /// Key names only: the right side is a template.
    Keys,
    /// Key names and values: two environments, or `--values`.
    Values,
}

/// Which differences fail the run (`--fail-on`, I-078). The report is the same
/// whatever this says; only the exit code follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailOn {
    pub missing: bool,
    pub extra: bool,
    pub different: bool,
}

impl FailOn {
    /// From `--fail-on`, already checked by clap. Empty means all three, which
    /// is what `diff` did before the flag existed.
    pub fn from_names(names: &[String]) -> FailOn {
        if names.is_empty() {
            return FailOn {
                missing: true,
                extra: true,
                different: true,
            };
        }
        let has = |n: &str| names.iter().any(|x| x == n);
        FailOn {
            missing: has("missing"),
            extra: has("extra"),
            different: has("different"),
        }
    }

    fn fails(&self, diff: &DiffResult) -> bool {
        (self.missing && !diff.missing.is_empty())
            || (self.extra && !diff.extra.is_empty())
            || (self.different && !diff.different.is_empty())
    }

    fn names(&self) -> String {
        [
            (self.missing, "missing"),
            (self.extra, "extra"),
            (self.different, "different"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// Everything `evnx diff` was asked to do.
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// The environment file (the left side unless `reverse`).
    pub env: String,
    /// The template or the other environment (the right side unless `reverse`).
    pub example: String,
    pub compare: Compare,
    pub show_values: bool,
    pub format: String,
    pub reverse: bool,
    pub verbose: bool,
    /// `--ignore-keys` then `[diff] ignore_keys`.
    pub ignore_keys: Vec<String>,
    pub fail_on: FailOn,
    pub with_stats: bool,
    pub interactive: bool,
    pub skip_without_env: bool,
}

/// Flags that would do nothing in this combination are refused before the run
/// (I-075), rather than silently ignored.
pub fn check_flags(
    format: &str,
    with_stats: bool,
    interactive: bool,
    compare: Compare,
    fail_on: &[String],
) -> Result<()> {
    if with_stats && format != "json" {
        anyhow::bail!(
            "--with-stats adds a `stats` object to JSON output: use it with --format json"
        );
    }
    if interactive && format != "patch" {
        anyhow::bail!(
            "--interactive prompts for the changes in a patch: use it with --format patch"
        );
    }
    if compare == Compare::Keys && fail_on.iter().any(|f| f == "different") {
        anyhow::bail!(
            "--fail-on different needs values to be compared, and against a template diff \
             compares key names only.\n\n\
             Add --values to compare them, or compare two environments with --against NAME."
        );
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffResult {
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub different: Vec<DiffItem>,
    /// Keys left out by `--ignore-keys` or `[diff] ignore_keys` that were in
    /// either file (I-077). Names that matched nothing are listed by `-v`.
    #[serde(default)]
    pub ignored: Vec<String>,
    ///  Optional statistics for JSON output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<DiffStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffItem {
    pub key: String,
    /// What may be shown for the template's value — **never the raw secret**.
    ///
    /// `None` means withheld: `--show-values` was not passed. With it, the
    /// value, or its masked form when it holds a secret (`value_state` says
    /// which).
    ///
    /// ⛔ **This was `example_value: String`, serialised unconditionally.**
    /// `evnx diff --format json` therefore printed every differing value in
    /// full — with or without `--show-values`, sensitive or not — and the
    /// GitHub Action forces `--format json` because the exit code is unusable.
    /// A public repository running `command: diff` published its `.env`.
    ///
    /// The two fields that used to sit beside these, `example_value_redacted`
    /// and `env_value_redacted`, are gone. Holding the raw value and the mask
    /// side by side *was* the bug: every consumer had to know to prefer the
    /// second, and the JSON formatter did not.
    pub example_value: Option<String>,
    pub env_value: Option<String>,
    /// Why the values above are what they are: `withheld` (not requested),
    /// `masked` (a secret part replaced) or `shown` (I-076b). It replaced
    /// `values_redacted`, removed in schema 2 (RM-16): a `null` value with
    /// `values_redacted: false` read as "not redacted" when it meant "not
    /// requested". `value_state == "masked"` is the old test.
    pub value_state: ValueState,
}

///  Statistics for programmatic consumption
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffStats {
    pub total_keys_env: usize,
    pub total_keys_example: usize,
    pub overlap_count: usize,
    pub similarity_percent: f64,
}

// ─────────────────────────────────────────────────────────────
// Main Entry Point ( returns exit code for CI/CD)
// ─────────────────────────────────────────────────────────────

pub fn run(opts: DiffOptions) -> Result<i32> {
    // DEC-U8-5's skip, for the pre-commit hook (D-39): a fresh clone has a
    // template and no `.env`. Only the environment file is skipped — a missing
    // template is still "could not run".
    if opts.skip_without_env && !std::path::Path::new(&opts.env).exists() {
        crate::core::verdict::skipped(&format!("{} not found", opts.env));
        return Ok(0);
    }

    let parser = Parser::default();

    let env_file = parser.parse_file_or_hint(&opts.env, &Parser::missing_env_hint(&opts.env))?;

    let example_file = parser.parse_file_or_hint(
        &opts.example,
        "Create it from your .env with `evnx sync`, or point at another file \
         with --example. `evnx diff --against staging` compares two real \
         environments instead of a template.",
    )?;

    let (left, right, left_name, right_name) = if opts.reverse {
        (&example_file.vars, &env_file.vars, &opts.example, &opts.env)
    } else {
        (&env_file.vars, &example_file.vars, &opts.env, &opts.example)
    };

    // Only print header for pretty output, and only once both files are read:
    // an error used to arrive under a header announcing a comparison that
    // never happened (I-080).
    //
    // The file names were hardcoded here, so `--env-name production --against
    // staging` announced "Comparing .env ↔ .env.example" and then listed
    // differences between two entirely different files.
    if opts.format == "pretty" {
        // `ui::print_header` gives the same context in one line and keeps `diff`
        // consistent with everything else.
        ui::print_header("evnx diff", Some(&format!("{left_name} ↔ {right_name}")));
    }

    // In the order given, flag first; once each.
    let mut ignore_list: Vec<String> = Vec::new();
    for k in &opts.ignore_keys {
        if !ignore_list.contains(k) {
            ignore_list.push(k.clone());
        }
    }
    let ignore_set: HashSet<String> = ignore_list.iter().cloned().collect();

    // Compute diff with filtering + redaction
    let mut diff_result = compute_diff(left, right, &ignore_set, opts.compare, opts.show_values);
    let (ignored, unmatched): (Vec<String>, Vec<String>) = ignore_list
        .into_iter()
        .partition(|k| left.contains_key(k) || right.contains_key(k));
    diff_result.ignored = ignored;

    if opts.verbose {
        say_what_was_compared(
            left_name,
            right_name,
            left.len(),
            right.len(),
            &opts,
            &diff_result.ignored,
            &unmatched,
        );
    }

    // `check_flags` refused --with-stats without json (I-075).
    if opts.with_stats {
        diff_result.stats = Some(compute_stats(left, right, &diff_result));
    }

    // Route to output formatter
    match opts.format.as_str() {
        "json" => output_json(&diff_result, opts.compare)?,
        "patch" => {
            // DEC-G7: values are withheld unless asked for; when they are, the
            // output is exact (a patch must apply) and says it holds secrets.
            if opts.show_values {
                ui::warning(format!(
                    "--show-values: this output holds the real values from {left_name} \
                     and {right_name}; do not paste it where others can read it"
                ));
            }
            if opts.interactive {
                output_patch_interactive(&diff_result, left, right, right_name, opts.show_values)?;
            } else {
                output_patch(&diff_result, left, right, opts.compare, opts.show_values)?;
            }
            say_values_not_compared(opts.compare);
        }
        _ => {
            output_pretty(
                &diff_result,
                left,
                right,
                left_name,
                right_name,
                opts.compare,
                opts.show_values,
            )?;
            say_values_not_compared(opts.compare);
        }
    }

    // ✅ Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::DIFF);

    Ok(if opts.fail_on.fails(&diff_result) {
        1
    } else {
        0
    })
}

/// One line under a key-only report, so the absence of a "Different values"
/// section is not read as "the values agree".
fn say_values_not_compared(compare: Compare) {
    if compare == Compare::Keys && !ui::is_quiet() {
        eprintln!(
            "  {}",
            "values not compared against a template (--values to compare them)".dimmed()
        );
    }
}

/// `-v`: which files, compared how, failing on what, and what was ignored
/// (I-079). It printed one line, "Running diff in verbose mode", which said
/// nothing the user had not typed.
fn say_what_was_compared(
    left_name: &str,
    right_name: &str,
    left_len: usize,
    right_len: usize,
    opts: &DiffOptions,
    ignored: &[String],
    unmatched: &[String],
) {
    ui::verbose_stderr(format!(
        "comparing {left_name} ({}) with {right_name} ({})",
        pluralize(left_len, "key", "keys"),
        pluralize(right_len, "key", "keys")
    ));
    ui::verbose_stderr(match opts.compare {
        Compare::Keys => format!(
            "{} is a template: key names only (--values compares values)",
            opts.example
        ),
        Compare::Values => "key names and values".to_string(),
    });
    ui::verbose_stderr(format!("fails on: {}", opts.fail_on.names()));
    if !ignored.is_empty() {
        ui::verbose_stderr(format!("ignored: {}", ignored.join(", ")));
    }
    if !unmatched.is_empty() {
        ui::verbose_stderr(format!(
            "ignore names that matched no key (case counts): {}",
            unmatched.join(", ")
        ));
    }
}

// ─────────────────────────────────────────────────────────────
// Core Diff Logic ( uses IndexMap,  adds redaction)
// ─────────────────────────────────────────────────────────────

fn compute_diff(
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    ignore_keys: &HashSet<String>,
    compare: Compare,
    // ⚠️ `compute_diff` did not take this, so the JSON path had no way to
    // honour `--show-values` — which is why the flag was inert there in both
    // directions: json always leaked, and passing the flag changed nothing.
    show_values: bool,
) -> DiffResult {
    //  Filter out ignored keys first (preserves order via IndexMap)
    let left_filtered: IndexMap<_, _> = left
        .iter()
        .filter(|(k, _)| !ignore_keys.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let right_filtered: IndexMap<_, _> = right
        .iter()
        .filter(|(k, _)| !ignore_keys.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // In each side's own order (`core::envfile::only_in`, shared with sync).
    let missing = crate::core::envfile::only_in(&right_filtered, &left_filtered);
    let extra = crate::core::envfile::only_in(&left_filtered, &right_filtered);

    // Against a template, values are not compared at all (I-078).
    let mut different = Vec::new();
    for key in left_filtered.keys() {
        if compare == Compare::Keys {
            break;
        }
        if let (Some(left_val), Some(right_val)) = (left_filtered.get(key), right_filtered.get(key))
        {
            if left_val != right_val {
                let example = disclose(key, right_val, show_values);
                let env = disclose(key, left_val, show_values);
                different.push(DiffItem {
                    key: key.clone(),
                    example_value: example.text().map(str::to_string),
                    env_value: env.text().map(str::to_string),
                    value_state: value_state(&example, &env),
                });
            }
        }
    }

    DiffResult {
        missing,
        extra,
        different,
        ignored: Vec::new(),
        stats: None,
    }
}

/// One state for the pair: masked if either side is. Withheld is all or
/// nothing — it follows `--show-values`, not the value.
fn value_state(example: &Disclosure, env: &Disclosure) -> ValueState {
    match (example.state(), env.state()) {
        (ValueState::Masked, _) | (_, ValueState::Masked) => ValueState::Masked,
        (ValueState::Shown, ValueState::Shown) => ValueState::Shown,
        _ => ValueState::Withheld,
    }
}

/// What `diff` is allowed to show for one key's value. The **only** place this
/// is decided, so the JSON report and the human output cannot disagree — which
/// is exactly how they came to.
///
/// ⚠️ **`--show-values` does not reveal a credential-shaped key, and that is
/// deliberate.** It never did in the human output, and a flag whose meaning
/// changes with `--format` is worse than one with a documented limit. The
/// values are in the file; `diff` is not the tool for reading them out.
///
/// ⚠️ Now `core::mask::disclose` (C-05, A-10a): with `--show-values` it reads
/// the **value** too, so `DATABASE_URL=postgres://admin:s3cr3t@db` shows
/// `postgres://admin:***@db` instead of the password (I-076), and a token
/// under a harmless name is masked whole. Without it nothing is shown, not
/// even `***` for a secret name (owner, 2026-10-21).
fn disclose(key: &str, value: &str, show_values: bool) -> Disclosure {
    crate::core::mask::disclose(key, value, show_values)
}

// Compute statistics for JSON output
fn compute_stats(
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    _diff: &DiffResult,
) -> DiffStats {
    let left_keys: HashSet<_> = left.keys().collect();
    let right_keys: HashSet<_> = right.keys().collect();

    // Count keys present in BOTH maps
    let overlap_count = left_keys.intersection(&right_keys).count();

    let total_left = left.len();
    let total_right = right.len();

    // ✅ Sørensen-Dice coefficient: 2*|A∩B| / (|A|+|B|) * 100
    // This gives 50% for 1 overlap out of 2 keys each, which is intuitive
    let similarity = if total_left + total_right > 0 {
        (2.0 * overlap_count as f64 / (total_left + total_right) as f64) * 100.0
    } else {
        100.0
    };

    DiffStats {
        total_keys_env: total_left,
        total_keys_example: total_right,
        overlap_count,
        similarity_percent: (similarity * 100.0).round() / 100.0,
    }
}

// ─────────────────────────────────────────────────────────────
// Output Formatters
// ─────────────────────────────────────────────────────────────

fn output_pretty(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    left_name: &str,
    right_name: &str,
    compare: Compare,
    show_values: bool,
) -> Result<()> {
    let ignored = match diff.ignored.len() {
        0 => String::new(),
        n => format!(" ({})", pluralize(n, "key ignored", "keys ignored")),
    };
    if !diff.has_changes() {
        // `ui::success`: same line, and `-q` hides it (I-079).
        //
        // ⚠️ A key-only run says so: "Files are identical" after comparing no
        // values would be a claim nobody checked (I-078).
        ui::success(match compare {
            Compare::Keys => format!("{left_name} and {right_name} have the same keys{ignored}"),
            Compare::Values => format!("Files are identical{ignored}"),
        });
        return Ok(());
    }

    if !diff.missing.is_empty() {
        println!(
            "{}",
            format!("Missing from {} (present in {}):", left_name, right_name).bold()
        );
        for key in &diff.missing {
            if let Some(val) = right.get(key) {
                // ⛔ Through `disclose`, like everything else.
                //
                // This used to look the key up in `diff.different` for a
                // redacted form and fall back to the raw value — but a *missing*
                // key is by definition not in `different`, so the lookup always
                // missed and the fallback always won. `--show-values` therefore
                // printed missing and extra values **completely unredacted,
                // including credential-shaped ones**, while the `different`
                // section three lines below masked them correctly.
                match disclose(key, val, show_values).text() {
                    Some(d) => println!("  {} {} = {}", "+".green(), key.bold(), d.dimmed()),
                    None => println!("  {} {}", "+".green(), key.bold()),
                }
            }
        }
        println!();
    }

    if !diff.extra.is_empty() {
        println!(
            "{}",
            format!("Extra in {} (not in {}):", left_name, right_name).bold()
        );
        for key in &diff.extra {
            if let Some(val) = left.get(key) {
                match disclose(key, val, show_values).text() {
                    Some(d) => println!("  {} {} = {}", "-".red(), key.bold(), d.dimmed()),
                    None => println!("  {} {}", "-".red(), key.bold()),
                }
            }
        }
        println!();
    }

    if !diff.different.is_empty() {
        println!("{}", "Different values:".bold());
        for item in &diff.different {
            println!("  {} {}", "~".yellow(), item.key.bold());
            // `compute_diff` already decided, so there is nothing to choose
            // between here. A masked key prints its mask; a withheld one prints
            // nothing but the key name.
            if let (Some(ex), Some(env)) = (&item.example_value, &item.env_value) {
                println!("    {}: {}", right_name, ex.dimmed());
                println!("    {}: {}", left_name, env.dimmed());
            }
        }
        println!();
    }

    // One line, only the non-zero counts, matching `scan`, `validate` and
    // `doctor`. The old block always printed all three even when every one was
    // zero — "0 missing (add to .env)" is advice about nothing.
    let mut parts = Vec::new();
    if !diff.missing.is_empty() {
        parts.push(format!("{} missing from {left_name}", diff.missing.len()));
    }
    if !diff.extra.is_empty() {
        parts.push(format!("{} not in {right_name}", diff.extra.len()));
    }
    if !diff.different.is_empty() {
        parts.push(pluralize(
            diff.different.len(),
            "different value",
            "different values",
        ));
    }
    // I-077: an ignored key is not silently absent.
    if !diff.ignored.is_empty() {
        parts.push(pluralize(diff.ignored.len(), "key ignored", "keys ignored"));
    }
    println!("  {}", parts.join("  ·  ").bold());

    // ⓘ A "Statistics" block sat here, unreachable: stats were only computed
    // for json. `--with-stats` without json is now refused (I-075).
    Ok(())
}

/// The counts, alongside the arrays they describe.
///
/// ⚠️ Derived at output time rather than stored on `DiffResult`. A stored count
/// is a second source of truth that can disagree with the array beside it, and
/// the only way it can ever be wrong is if someone forgets to update it.
///
/// `total` is what a CI gate tests — `jq -e '.summary.total == 0'` — and it
/// matches `scan`'s `summary.total` so the two read the same way.
#[derive(Serialize)]
struct DiffSummary {
    missing: usize,
    extra: usize,
    different: usize,
    total: usize,
}

/// `DiffResult` plus its summary.
///
/// A wrapper rather than a field on `DiffResult`, because the struct is also the
/// pretty renderer's input and does not otherwise need to know about JSON.
#[derive(Serialize)]
struct JsonReport<'a> {
    /// The shape of this report (S-D6). Raised when a field changes meaning or
    /// goes away: 2 removed `values_redacted` from `different` (RM-16).
    schema_version: u32,
    /// `keys` against a template, `values` between two environments or with
    /// `--values` (I-078). With `keys`, `different` is always empty.
    compared: Compare,
    #[serde(flatten)]
    diff: &'a DiffResult,
    summary: DiffSummary,
}

/// The JSON report's `schema_version`.
const SCHEMA_VERSION: u32 = 2;

fn output_json(diff: &DiffResult, compared: Compare) -> Result<()> {
    // Additive: `missing`, `extra`, `different` and `stats` keep their places,
    // so a consumer reading them is unaffected.
    let report = JsonReport {
        schema_version: SCHEMA_VERSION,
        compared,
        diff,
        summary: DiffSummary {
            missing: diff.missing.len(),
            extra: diff.extra.len(),
            different: diff.different.len(),
            total: diff.missing.len() + diff.extra.len() + diff.different.len(),
        },
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn output_patch(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    compare: Compare,
    show_values: bool,
) -> Result<()> {
    println!("# Add these to .env:");
    for key in &diff.missing {
        if let Some(v) = right.get(key) {
            println!("+ {}={}", key, shown(v, show_values));
        }
    }

    println!("\n# Remove these from .env:");
    for key in &diff.extra {
        if let Some(v) = left.get(key) {
            println!("- {}={}", key, shown(v, show_values));
        }
    }

    // Against a template there are no updates: replacing real values with
    // placeholders is what this section did (I-078, O-5).
    if compare == Compare::Keys {
        return Ok(());
    }

    println!("\n# Update these in .env:");
    for item in &diff.different {
        // ⚠️ From the maps, not from `DiffItem`, which now carries only what is
        // safe to *display*. A patch exists to be applied, so it needs the real
        // values — masking them would produce a patch that writes `***` into
        // the file.
        //
        // ⚠️ This is therefore the one format that prints secrets, and it is
        // reachable only by typing `--format patch` by hand: the GitHub Action
        // forces `--format json` for diff, and `pretty` is the default.
        if let (Some(env_val), Some(ex_val)) = (left.get(&item.key), right.get(&item.key)) {
            println!("- {}={}", item.key, shown(env_val, show_values));
            println!("+ {}={}", item.key, shown(ex_val, show_values));
        }
    }

    Ok(())
}

/// A value as patch and interactive print it: exact when values were asked
/// for, `<withheld>` otherwise (I-071, DEC-G7).
fn shown(value: &str, show_values: bool) -> &str {
    if show_values {
        value
    } else {
        crate::core::mask::WITHHELD
    }
}

// Interactive merge mode for patch format
fn output_patch_interactive(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    _target_file: &str,
    show_values: bool,
) -> Result<()> {
    use std::io::{self, Write};

    println!("\n{}", "Interactive merge".bold());
    println!("Prompts: (y)es, (n)o, (s)kip rest\n");

    let mut applied = Vec::new();

    // Handle missing keys
    for key in &diff.missing {
        if let Some(v) = right.get(key) {
            print!("Add {}={}? [y/n/s]: ", key, shown(v, show_values));
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_lowercase().as_str() {
                "y" | "yes" => {
                    println!("   ✓ Queued: + {}={}", key, shown(v, show_values));
                    applied.push(('+', key.clone(), shown(v, show_values).to_string()));
                }
                "s" | "skip" => break,
                _ => println!("   ✗ Skipped"),
            }
        }
    }

    // Handle different values
    for item in &diff.different {
        // Same reasoning as `output_patch`: an interactive merge writes real
        // values into a real file, so it reads them from the maps.
        let (Some(env_val), Some(ex_val)) = (left.get(&item.key), right.get(&item.key)) else {
            continue;
        };
        print!(
            "Update {}?\n   - {}\n   + {}? [y/n/s]: ",
            item.key,
            shown(env_val, show_values),
            shown(ex_val, show_values)
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => {
                println!("   ✓ Queued: + {}={}", item.key, shown(ex_val, show_values));
                applied.push((
                    '~',
                    item.key.clone(),
                    shown(ex_val, show_values).to_string(),
                ));
            }
            "s" | "skip" => break,
            _ => println!("   ✗ Skipped"),
        }
    }

    // Summary of applied changes
    if !applied.is_empty() {
        println!("\n{}", "Applied changes:".bold());
        for (op, key, val) in &applied {
            let icon = match op {
                '+' => "+".green(),
                '~' => "~".yellow(),
                _ => "-".red(),
            };
            println!("  {} {}={}", icon, key, val);
        }
        println!(
            "\n  {}  save it and apply with: patch -p1 < changes.patch",
            glyph::ARROW.dimmed()
        );
    } else {
        println!("\n{}", "· No changes applied".dimmed());
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helper Methods
// ─────────────────────────────────────────────────────────────

impl DiffResult {
    /// Check if any differences were found (for exit code logic)
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.missing.is_empty() || !self.extra.is_empty() || !self.different.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────
// Tests (enhanced for new features)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::indexmap;

    #[test]
    fn test_compute_diff_basic() {
        let left = indexmap! {
            "KEY1".to_string() => "value1".to_string(),
            "KEY2".to_string() => "value2".to_string(),
            "EXTRA".to_string() => "extra".to_string(),
        };
        let right = indexmap! {
            "KEY1".to_string() => "value1".to_string(),
            "KEY2".to_string() => "different".to_string(),
            "MISSING".to_string() => "missing".to_string(),
        };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);

        assert_eq!(diff.missing, vec!["MISSING"]);
        assert_eq!(diff.extra, vec!["EXTRA"]);
        assert_eq!(diff.different.len(), 1);
        assert_eq!(diff.different[0].key, "KEY2");
    }

    #[test]
    fn test_ignore_keys_filtering() {
        let left = indexmap! { "IGNORED".to_string() => "a".to_string() };
        let right = indexmap! { "IGNORED".to_string() => "b".to_string() };
        let mut ignore = HashSet::new();
        ignore.insert("IGNORED".to_string());

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);

        assert!(diff.missing.is_empty());
        assert!(diff.extra.is_empty());
        assert!(diff.different.is_empty());
    }

    // ─────────────────────────────────────────────────────────
    // S2 — a value leaves `diff` only when it is safe to
    //
    // ⚠️ `test_redact_sensitive_values` used to live here. It asserted that
    // `redact_if_sensitive` returned a mask containing `***` and not the raw
    // value — and it passed throughout, because the function it tested was
    // correct. The leak was that **`--format json` never consulted it**: the
    // raw value sat in `example_value`/`env_value` beside the mask, and the
    // JSON formatter serialised the struct. A unit test of the redactor could
    // not see that. These test the output instead.
    // ─────────────────────────────────────────────────────────

    /// ⛔ The reported defect. A sensitive value must not reach the JSON,
    /// which is the format the GitHub Action is forced to use.
    #[test]
    fn json_never_carries_a_sensitive_value() {
        let left = indexmap! {
            "DB_PASSWORD".to_string() => "HUNTER2_live".to_string(),
            "API_KEY".to_string()     => "sk_live_abc".to_string(),
        };
        let right = indexmap! {
            "DB_PASSWORD".to_string() => "example_pw".to_string(),
            "API_KEY".to_string()     => "sk_test_xyz".to_string(),
        };
        // Both ways: the flag must not be able to turn the mask off.
        for show_values in [false, true] {
            let diff = compute_diff(&left, &right, &HashSet::new(), Compare::Values, show_values);
            let json = serde_json::to_string(&diff).unwrap();
            for secret in ["HUNTER2_live", "sk_live_abc", "example_pw", "sk_test_xyz"] {
                assert!(
                    !json.contains(secret),
                    "--show-values={show_values} leaked {secret} into json: {json}"
                );
            }
            // Withheld without the flag, masked with it — never shown.
            let expected = if show_values {
                ValueState::Masked
            } else {
                ValueState::Withheld
            };
            assert!(diff.different.iter().all(|d| d.value_state == expected));
        }
    }

    /// `--show-values` must actually do something, and only for values that
    /// are not credential-shaped.
    #[test]
    fn show_values_governs_non_sensitive_values_and_nothing_else() {
        let left = indexmap! { "LOG_LEVEL".to_string() => "debug".to_string() };
        let right = indexmap! { "LOG_LEVEL".to_string() => "info".to_string() };

        let withheld = compute_diff(&left, &right, &HashSet::new(), Compare::Values, false);
        assert_eq!(withheld.different[0].env_value, None, "withheld");
        assert!(!serde_json::to_string(&withheld).unwrap().contains("debug"));

        let shown = compute_diff(&left, &right, &HashSet::new(), Compare::Values, true);
        assert_eq!(shown.different[0].env_value.as_deref(), Some("debug"));
        assert_eq!(shown.different[0].example_value.as_deref(), Some("info"));
    }

    /// ⛔ The one the report missed: `evnx diff` **panicked**. Byte index 2 is
    /// not a character boundary when the value starts with a 3-byte character.
    ///
    /// ```text
    /// thread 'main' panicked at src/commands/diff.rs:232:36:
    /// end byte index 2 is not a char boundary; it is inside '日'
    /// ```
    #[test]
    fn a_non_ascii_secret_does_not_panic() {
        for value in ["日本語のパスワード", "中", "é", "🔑key", ""] {
            let left = indexmap! { "DB_PASSWORD".to_string() => value.to_string() };
            let right = indexmap! { "DB_PASSWORD".to_string() => "x".to_string() };
            let diff = compute_diff(&left, &right, &HashSet::new(), Compare::Values, true);
            if let Some(item) = diff.different.first() {
                assert_eq!(item.env_value.as_deref(), Some(crate::core::mask::MASK));
            }
        }
    }

    /// The mask must not carry a prefix of the real value. `sk***` identifies
    /// the credential's issuer; the key name beside it already said more.
    #[test]
    fn the_mask_reveals_no_part_of_the_value() {
        let left = indexmap! { "STRIPE_SECRET".to_string() => "sk_live_zzz".to_string() };
        let right = indexmap! { "STRIPE_SECRET".to_string() => "sk_test_aaa".to_string() };
        let diff = compute_diff(&left, &right, &HashSet::new(), Compare::Values, true);
        assert_eq!(diff.different[0].env_value.as_deref(), Some("***"));
        assert_eq!(diff.different[0].example_value.as_deref(), Some("***"));
    }

    /// ⛔ The second one the report missed. `disclose` is now the only decision,
    /// so a key that is only on one side is masked exactly like one that
    /// differs — the human output used to print those raw under
    /// `--show-values`, because it looked for a mask in `different`, where a
    /// missing key can never be.
    #[test]
    fn disclosure_does_not_depend_on_which_section_a_key_lands_in() {
        let masked = disclose("DB_PASSWORD", "HUNTER2", true);
        assert_eq!(
            masked.text(),
            Some(crate::core::mask::MASK),
            "sensitive key was not masked"
        );
        assert!(masked.is_masked());
        // Not requested: nothing, whatever the name (owner, 2026-10-21).
        assert_eq!(disclose("DB_PASSWORD", "HUNTER2", false).text(), None);
        assert_eq!(disclose("LOG_LEVEL", "debug", false).text(), None);
        assert_eq!(disclose("LOG_LEVEL", "debug", true).text(), Some("debug"));
    }

    /// I-078: against a template, values are not compared.
    #[test]
    fn t_i078_keys_mode_compares_no_values() {
        let left = indexmap! { "A".to_string() => "real".to_string() };
        let right = indexmap! { "A".to_string() => "placeholder".to_string() };
        let diff = compute_diff(&left, &right, &HashSet::new(), Compare::Keys, true);
        assert!(!diff.has_changes(), "{diff:?}");
        let diff = compute_diff(&left, &right, &HashSet::new(), Compare::Values, true);
        assert_eq!(diff.different.len(), 1);
    }

    /// I-078: `--fail-on` decides the exit code from the sections it names.
    #[test]
    fn t_i078_fail_on_reads_only_the_named_sections() {
        let extra_only = DiffResult {
            missing: vec![],
            extra: vec!["LOCAL".to_string()],
            different: vec![],
            ignored: vec![],
            stats: None,
        };
        assert!(FailOn::from_names(&[]).fails(&extra_only));
        assert!(!FailOn::from_names(&["missing".to_string()]).fails(&extra_only));
        assert!(
            FailOn::from_names(&["missing".to_string(), "extra".to_string()]).fails(&extra_only)
        );
        assert_eq!(FailOn::from_names(&[]).names(), "missing, extra, different");
    }

    #[test]
    fn test_order_preservation() {
        // IndexMap should preserve insertion order
        let left = indexmap! {
            "A".to_string() => "1".to_string(),
            "B".to_string() => "2".to_string(),
            "C".to_string() => "3".to_string(),
        };
        let right = indexmap! {
            "C".to_string() => "3".to_string(),
            "D".to_string() => "4".to_string(),  // Missing in left
            "A".to_string() => "1".to_string(),
        };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);

        // Missing should follow right's order: D appears after C in right
        assert_eq!(diff.missing, vec!["D"]);
        // Extra should follow left's order: B
        assert_eq!(diff.extra, vec!["B"]);
    }

    #[test]
    fn test_has_changes() {
        let diff_empty = DiffResult {
            missing: vec![],
            extra: vec![],
            different: vec![],
            ignored: vec![],
            stats: None,
        };
        assert!(!diff_empty.has_changes());

        let diff_with_missing = DiffResult {
            missing: vec!["NEW_KEY".to_string()],
            extra: vec![],
            different: vec![],
            ignored: vec![],
            stats: None,
        };
        assert!(diff_with_missing.has_changes());
    }

    #[test]
    fn test_compute_stats() {
        let left =
            indexmap! { "A".to_string() => "1".to_string(), "B".to_string() => "2".to_string() };
        let right =
            indexmap! { "A".to_string() => "1".to_string(), "C".to_string() => "3".to_string() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.total_keys_env, 2);
        assert_eq!(stats.total_keys_example, 2);
        assert_eq!(stats.overlap_count, 1); // Only "A" overlaps
        assert!((stats.similarity_percent - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_stats_identical_files() {
        let left = indexmap! { "A".into() => "1".into(), "B".into() => "2".into() };
        let right = indexmap! { "A".into() => "1".into(), "B".into() => "2".into() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 2);
        assert_eq!(stats.similarity_percent, 100.0);
    }

    #[test]
    fn test_compute_stats_no_overlap() {
        let left = indexmap! { "A".into() => "1".into() };
        let right = indexmap! { "B".into() => "2".into() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 0);
        assert_eq!(stats.similarity_percent, 0.0);
    }

    #[test]
    fn test_compute_stats_empty_files() {
        let left = indexmap! {};
        let right = indexmap! {};
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, Compare::Values, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.similarity_percent, 100.0); // Empty files are "identical"
    }
}
