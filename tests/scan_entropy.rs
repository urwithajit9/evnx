//! The entropy thresholds `evnx scan` uses, pinned to the corpus they were
//! measured on (C4, RM-53). The numbers, and why these: `docs/scan-thresholds.md`,
//! from `scripts/measure-entropy.py` over `tests/fixtures/entropy/`.
//!
//! A change to `heuristic::ENTROPY_MIN`, `ENTROPY_MIN_LEN` or
//! `looks_like_key_material` that loses recall or gains a false positive on the
//! corpus fails here.

use evnx::core::secretscan::heuristic::detect_heuristic;
use std::path::Path;

fn corpus(kind: &str) -> Vec<(String, Vec<String>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/entropy")
        .join(kind);
    let mut out: Vec<(String, Vec<String>)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .map(|p| {
            let lines = std::fs::read_to_string(&p)
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            (p.file_stem().unwrap().to_string_lossy().into_owned(), lines)
        })
        .collect();
    out.sort();
    out
}

/// The share of `values` the generic fallback reports, with no name to go on.
fn reported(values: &[String]) -> f64 {
    let hits = values
        .iter()
        .filter(|v| detect_heuristic(v, "").is_some())
        .count();
    hits as f64 / values.len() as f64
}

/// Random keys of 32 characters or more, in base64 or alphanumeric form, are
/// reported: 95% at 32 (a random string's entropy is capped by its length),
/// all of them from 40.
#[test]
fn random_keys_of_32_or_more_are_reported() {
    for (class, values) in corpus("positives") {
        let len: usize = class.rsplit('-').next().unwrap().parse().unwrap();
        let share = reported(&values);
        if class.starts_with("hex") || len < 32 {
            // ⚠️ Never, by design: hex tops out near 4 bits per character, the
            // same as SHAs and checksums, and a value under 32 characters is
            // not judged. Provider rules (C3) are what catch these.
            assert_eq!(share, 0.0, "{class}");
        } else if len == 32 {
            assert!(share >= 0.95, "{class}: {share}");
        } else {
            assert_eq!(share, 1.0, "{class}");
        }
    }
}

/// Known non-secrets that look random are not reported: npm SRI hashes, git
/// SHAs, checksums, UUIDs, minified and source identifiers. Wrapped base64
/// lines are the exception, and stay reported: they cannot be told apart from
/// a PEM key body, which is a secret.
#[test]
fn hashes_ids_and_source_tokens_are_not_reported() {
    for (class, values) in corpus("negatives") {
        let share = reported(&values);
        if class == "base64-image" {
            assert!(share > 0.9, "{class}: {share}");
        } else {
            assert_eq!(share, 0.0, "{class}");
        }
    }
}

/// The table in `docs/scan-thresholds.md` is what the script writes from this
/// corpus: a corpus change that is not re-measured fails here.
#[test]
fn the_measured_table_is_current() {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = std::process::Command::new("python3")
        .args(["-I", "scripts/measure-entropy.py", "--check"])
        .current_dir(root)
        .output();
    match out {
        Ok(o) => assert!(
            o.status.success(),
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(_) => {
            // T2: a missing tool fails under CI and says so locally.
            assert!(
                std::env::var_os("CI").is_none(),
                "python3 is not installed, and CI is set: the table would go unchecked"
            );
            eprintln!("skipped: python3 is not installed");
        }
    }
}
