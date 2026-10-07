use crate::core::Parser;
use crate::docs;
use crate::utils::patterns;
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
use std::collections::HashSet; //  Reuse existing sensitive-key detection
                               // use crate::utils::ui; //   Reuse UI helpers if available

// ─────────────────────────────────────────────────────────────
// Data Structures (enhanced with redaction + stats)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffResult {
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub different: Vec<DiffItem>,
    ///  Optional statistics for JSON output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<DiffStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffItem {
    pub key: String,
    /// What may be shown for the template's value — **never the raw secret**.
    ///
    /// `None` means withheld: the value is not sensitive but `--show-values`
    /// was not passed. `Some("***")` with `values_redacted` means the key is
    /// credential-shaped and the value is masked whatever was asked for.
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
    /// `true` when the values above are masked rather than literal.
    pub values_redacted: bool,
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

#[allow(clippy::too_many_arguments)]
pub fn run(
    env: String,
    example: String,
    show_values: bool,
    format: String,
    reverse: bool,
    verbose: bool,
    ignore_keys: Vec<String>,
    with_stats: bool,
    interactive: bool,
) -> Result<i32> {
    //  CHANGED: Return exit code
    if verbose {
        eprintln!("{}", "Running diff in verbose mode".dimmed());
    }

    // Only print header for pretty output.
    //
    // The file names were hardcoded here, so `--env-name production --against
    // staging` announced "Comparing .env ↔ .env.example" and then listed
    // differences between two entirely different files.
    if format == "pretty" {
        // `ui::print_header` gives the same context in one line and keeps `diff`
        // consistent with everything else.
        //
        // ⚠️ This comment used to claim diff held "the last hand-rolled box in
        // the CLI". It did not: `migrate` kept its own, built from three string
        // literals rather than `print_box`, which is exactly why a sweep of the
        // helper missed it. Removed in the same pass as this note.
        ui::print_header("evnx diff", Some(&format!("{env} ↔ {example}")));
    }

    let parser = Parser::default();

    let env_file = parser.parse_file_or_hint(
        &env,
        "Create it with `evnx init`, or point at another file with --env.",
    )?;

    let example_file = parser.parse_file_or_hint(
        &example,
        "Create it from your .env with `evnx sync`, or point at another file \
         with --example. `evnx diff --against staging` compares two real \
         environments instead of a template.",
    )?;

    let (left, right, left_name, right_name) = if reverse {
        (&example_file.vars, &env_file.vars, &example, &env)
    } else {
        (&env_file.vars, &example_file.vars, &env, &example)
    };

    //  Convert ignore list to HashSet for O(1) lookups
    let ignore_set: HashSet<_> = ignore_keys.into_iter().collect();

    // Compute diff with filtering + redaction
    let mut diff_result = compute_diff(left, right, &ignore_set, show_values);

    // Add statistics if requested (JSON mode)
    if with_stats && format == "json" {
        diff_result.stats = Some(compute_stats(left, right, &diff_result));
    }

    // Route to output formatter
    match format.as_str() {
        "json" => output_json(&diff_result)?,
        "patch" => {
            if interactive {
                output_patch_interactive(&diff_result, left, right, right_name)?;
            } else {
                output_patch(&diff_result, left, right)?;
            }
        }
        _ => output_pretty(
            &diff_result,
            left,
            right,
            left_name,
            right_name,
            show_values,
        )?,
    }

    // ✅ Always print — eprintln never pollutes stdout
    ui::print_docs_hint(&docs::DIFF);

    Ok(if diff_result.has_changes() { 1 } else { 0 })
}

// ─────────────────────────────────────────────────────────────
// Core Diff Logic ( uses IndexMap,  adds redaction)
// ─────────────────────────────────────────────────────────────

fn compute_diff(
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    ignore_keys: &HashSet<String>,
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

    let left_keys: HashSet<_> = left_filtered.keys().cloned().collect();
    let right_keys: HashSet<_> = right_filtered.keys().cloned().collect();

    //  Preserve order: iterate through right_filtered for "missing"
    let missing: Vec<String> = right_filtered
        .keys()
        .filter(|k| !left_keys.contains(*k))
        .cloned()
        .collect();

    //  Preserve order: iterate through left_filtered for "extra"
    let extra: Vec<String> = left_filtered
        .keys()
        .filter(|k| !right_keys.contains(*k))
        .cloned()
        .collect();

    let mut different = Vec::new();
    for key in left_filtered.keys() {
        if let (Some(left_val), Some(right_val)) = (left_filtered.get(key), right_filtered.get(key))
        {
            if left_val != right_val {
                let (example_value, ex_masked) = disclose(key, right_val, show_values);
                let (env_value, env_masked) = disclose(key, left_val, show_values);
                different.push(DiffItem {
                    key: key.clone(),
                    example_value,
                    env_value,
                    values_redacted: ex_masked || env_masked,
                });
            }
        }
    }

    DiffResult {
        missing,
        extra,
        different,
        stats: None,
    }
}

/// The mask. A fixed string, with **no prefix of the real value**.
///
/// ⚠️ This was `format!("{}***", &v[..v.len().min(2)])`, which had two
/// problems:
///
/// 1. **It panicked.** Byte index 2 is not a character boundary when the value
///    starts with a 3-byte character, so `DB_PASSWORD=日本語のパスワード` killed
///    `evnx diff` outright:
///
///    ```text
///    thread 'main' panicked at src/commands/diff.rs:232:36:
///    end byte index 2 is not a char boundary; it is inside '日'
///    ```
///
/// 2. **The prefix is a leak.** `sk***` says "this is a Stripe key"; `ey***`
///    says "this is a JWT". The key's own name is already printed beside it, so
///    the prefix adds nothing a reader did not have and tells an attacker which
///    credential is worth pursuing.
const MASK: &str = "***";

/// What `diff` is allowed to show for one key's value. The **only** place this
/// is decided, so the JSON report and the human output cannot disagree — which
/// is exactly how they came to.
///
/// ⚠️ **`--show-values` does not reveal a credential-shaped key, and that is
/// deliberate.** It never did in the human output, and a flag whose meaning
/// changes with `--format` is worse than one with a documented limit. The
/// values are in the file; `diff` is not the tool for reading them out.
fn disclose(key: &str, value: &str, show_values: bool) -> (Option<String>, bool) {
    if patterns::is_sensitive_key(key) {
        (Some(MASK.to_string()), true)
    } else if show_values {
        (Some(value.to_string()), false)
    } else {
        (None, false)
    }
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
    show_values: bool,
) -> Result<()> {
    if !diff.has_changes() {
        println!("{} Files are identical", "✓".green());
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
                let (display, _) = disclose(key, val, show_values);
                match display {
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
                let (display, _) = disclose(key, val, show_values);
                match display {
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
    if parts.is_empty() {
        println!(
            "  {}  {} and {} agree",
            glyph::OK.green(),
            left_name,
            right_name
        );
    } else {
        println!("  {}", parts.join("  ·  ").bold());
    }

    //  Show stats in pretty mode if available
    if let Some(stats) = &diff.stats {
        println!("\n{}", "Statistics:".bold());
        println!("  • Similarity: {:.1}%", stats.similarity_percent);
    }

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
    #[serde(flatten)]
    diff: &'a DiffResult,
    summary: DiffSummary,
}

fn output_json(diff: &DiffResult) -> Result<()> {
    // Additive: `missing`, `extra`, `different` and `stats` keep their places,
    // so a consumer reading them is unaffected.
    let report = JsonReport {
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
) -> Result<()> {
    println!("# Add these to .env:");
    for key in &diff.missing {
        if let Some(val) = right.get(key) {
            println!("+ {}={}", key, val);
        }
    }

    println!("\n# Remove these from .env:");
    for key in &diff.extra {
        if let Some(val) = left.get(key) {
            println!("- {}={}", key, val);
        }
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
            println!("- {}={}", item.key, env_val);
            println!("+ {}={}", item.key, ex_val);
        }
    }

    Ok(())
}

// Interactive merge mode for patch format
fn output_patch_interactive(
    diff: &DiffResult,
    left: &IndexMap<String, String>,
    right: &IndexMap<String, String>,
    _target_file: &str,
) -> Result<()> {
    use std::io::{self, Write};

    println!("\n{}", "Interactive merge".bold());
    println!("Prompts: (y)es, (n)o, (s)kip rest\n");

    let mut applied = Vec::new();

    // Handle missing keys
    for key in &diff.missing {
        if let Some(val) = right.get(key) {
            print!("Add {}={}? [y/n/s]: ", key, val);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_lowercase().as_str() {
                "y" | "yes" => {
                    println!("   ✓ Queued: + {}={}", key, val);
                    applied.push(('+', key.clone(), val.clone()));
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
            item.key, env_val, ex_val
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => {
                println!("   ✓ Queued: + {}={}", item.key, ex_val);
                applied.push(('~', item.key.clone(), ex_val.clone()));
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

        let diff = compute_diff(&left, &right, &ignore, false);

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

        let diff = compute_diff(&left, &right, &ignore, false);

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
            let diff = compute_diff(&left, &right, &HashSet::new(), show_values);
            let json = serde_json::to_string(&diff).unwrap();
            for secret in ["HUNTER2_live", "sk_live_abc", "example_pw", "sk_test_xyz"] {
                assert!(
                    !json.contains(secret),
                    "--show-values={show_values} leaked {secret} into json: {json}"
                );
            }
            assert!(diff.different.iter().all(|d| d.values_redacted));
        }
    }

    /// `--show-values` must actually do something, and only for values that
    /// are not credential-shaped.
    #[test]
    fn show_values_governs_non_sensitive_values_and_nothing_else() {
        let left = indexmap! { "LOG_LEVEL".to_string() => "debug".to_string() };
        let right = indexmap! { "LOG_LEVEL".to_string() => "info".to_string() };

        let withheld = compute_diff(&left, &right, &HashSet::new(), false);
        assert_eq!(withheld.different[0].env_value, None, "withheld");
        assert!(!withheld.different[0].values_redacted);
        assert!(!serde_json::to_string(&withheld).unwrap().contains("debug"));

        let shown = compute_diff(&left, &right, &HashSet::new(), true);
        assert_eq!(shown.different[0].env_value.as_deref(), Some("debug"));
        assert_eq!(shown.different[0].example_value.as_deref(), Some("info"));
        assert!(!shown.different[0].values_redacted);
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
            let diff = compute_diff(&left, &right, &HashSet::new(), true);
            if let Some(item) = diff.different.first() {
                assert_eq!(item.env_value.as_deref(), Some(MASK));
            }
        }
    }

    /// The mask must not carry a prefix of the real value. `sk***` identifies
    /// the credential's issuer; the key name beside it already said more.
    #[test]
    fn the_mask_reveals_no_part_of_the_value() {
        let left = indexmap! { "STRIPE_SECRET".to_string() => "sk_live_zzz".to_string() };
        let right = indexmap! { "STRIPE_SECRET".to_string() => "sk_test_aaa".to_string() };
        let diff = compute_diff(&left, &right, &HashSet::new(), true);
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
        for show_values in [false, true] {
            let (masked, redacted) = disclose("DB_PASSWORD", "HUNTER2", show_values);
            assert_eq!(
                masked.as_deref(),
                Some(MASK),
                "sensitive key was not masked"
            );
            assert!(redacted);
        }
        assert_eq!(disclose("LOG_LEVEL", "debug", false).0, None);
        assert_eq!(
            disclose("LOG_LEVEL", "debug", true).0.as_deref(),
            Some("debug")
        );
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

        let diff = compute_diff(&left, &right, &ignore, false);

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
            stats: None,
        };
        assert!(!diff_empty.has_changes());

        let diff_with_missing = DiffResult {
            missing: vec!["NEW_KEY".to_string()],
            extra: vec![],
            different: vec![],
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

        let diff = compute_diff(&left, &right, &ignore, false);
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

        let diff = compute_diff(&left, &right, &ignore, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 2);
        assert_eq!(stats.similarity_percent, 100.0);
    }

    #[test]
    fn test_compute_stats_no_overlap() {
        let left = indexmap! { "A".into() => "1".into() };
        let right = indexmap! { "B".into() => "2".into() };
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.overlap_count, 0);
        assert_eq!(stats.similarity_percent, 0.0);
    }

    #[test]
    fn test_compute_stats_empty_files() {
        let left = indexmap! {};
        let right = indexmap! {};
        let ignore = HashSet::new();

        let diff = compute_diff(&left, &right, &ignore, false);
        let stats = compute_stats(&left, &right, &diff);

        assert_eq!(stats.similarity_percent, 100.0); // Empty files are "identical"
    }
}
