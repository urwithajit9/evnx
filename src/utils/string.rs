//! String utility functions for formatting and display.
//!
//! These helpers are used throughout the CLI for consistent presentation
//! of values in terminal output.

/// Truncate a string to `max_len` characters.
///
/// If the string is longer than `max_len`, it is cut and `...` is appended.
/// `max_len` must be at least 4 — otherwise the ellipsis itself would not fit.
///
/// # Examples
///
/// ```
/// use evnx::utils::string::truncate;
/// assert_eq!(truncate("hello world", 8), "hello...");
/// assert_eq!(truncate("hi", 10), "hi");
/// ```
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Redact a sensitive value for safe display in terminal output.
///
/// - Short values (≤ 8 chars): replaced entirely with `*`.
/// - Longer values: first 4 chars shown, then `...****`.
///
/// The original value is never fully exposed in output; this is purely
/// for debugging context (e.g. showing that a key *has* a value).
///
/// # Examples
///
/// ```
/// use evnx::utils::string::redact;
/// assert_eq!(redact("secret"), "******");
/// assert_eq!(redact("secretkey123"), "secr...****");
/// ```
pub fn redact(s: &str) -> String {
    if s.len() <= 8 {
        "*".repeat(s.len())
    } else {
        format!("{}...{}", &s[..4], "*".repeat(4))
    }
}

/// Return a count-aware string using the correct singular or plural form.
///
/// # Examples
///
/// ```
/// use evnx::utils::string::pluralize;
/// assert_eq!(pluralize(1, "file", "files"), "1 file");
/// assert_eq!(pluralize(3, "file", "files"), "3 files");
/// ```
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{} {}", count, singular)
    } else {
        format!("{} {}", count, plural)
    }
}

/// `1 variable`, `2 variables`: a count with its noun, plural by `s` (I-022).
///
/// Moved from `add` in 0.10.12 so `sync` stops writing "Added 1 variables"
/// (I-126). A noun whose plural is not `+s` uses [`pluralize`].
///
/// ```
/// use evnx::utils::string::count;
/// assert_eq!(count(1, "variable"), "1 variable");
/// assert_eq!(count(0, "variable"), "0 variables");
/// ```
pub fn count(n: usize, noun: &str) -> String {
    pluralize(n, noun, &format!("{noun}s"))
}

/// The names to offer under "Did you mean": ids that start with or contain
/// `input` (up to six, in the order given), else the ids and aliases within a
/// small edit distance ([`closest`]), each alias suggesting its id.
///
/// One rule for every "unknown X" message that has a list to suggest from —
/// components and blueprints (I-235). Prefix and substring cover an
/// abbreviation (`mern` for `mern_v2`) and the wrong half of a two-word name;
/// edit distance covers a transposition, a dropped or a doubled letter. The
/// second is consulted only when the first finds nothing.
pub fn suggest<'a>(input: &str, ids: &[&'a str], aliases: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    let near: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| id.starts_with(input) || id.contains(input))
        .take(6)
        .collect();
    if !near.is_empty() {
        return near;
    }
    closest(
        input,
        ids.iter()
            .map(|id| (*id, *id))
            .chain(aliases.iter().copied()),
    )
}

/// "Did you mean": the candidates within a small edit distance of `input`,
/// closest first, at most three. `candidates` pairs a name to compare with the id to suggest (they
/// differ for an alias).
///
/// The threshold grows with length: one edit for names of up to four letters,
/// two above that. A fixed two would make `pg` look like `go` and `aws`, and a
/// fixed one would miss `postgrsql`.
///
/// One helper for every hint (moved from `schema::component` in 0.10.5):
/// `init`'s component and blueprint ids, `template`'s filter names.
pub fn closest<'a>(
    input: &str,
    candidates: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<&'a str> {
    let limit = if input.chars().count() <= 4 { 1 } else { 2 };
    let mut scored: Vec<(usize, &str)> = Vec::new();
    for (name, id) in candidates {
        let d = edit_distance(input, name);
        if d <= limit && !scored.iter().any(|(_, seen)| *seen == id) {
            scored.push((d, id));
        }
    }
    // Stable sort: equal distances keep catalogue order.
    scored.sort_by_key(|(d, _)| *d);
    scored.into_iter().take(3).map(|(_, id)| id).collect()
}

/// Optimal string alignment distance: insertions, deletions, substitutions and
/// swaps of two adjacent characters each cost one. Inputs here are a few dozen
/// short ids, so the plain O(n·m) table is the right size.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

/// `1024 KiB`, `5120 KiB`, `500 bytes`: a size as `scan`'s skip note says it.
pub fn size_label(bytes: u64) -> String {
    if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        assert_eq!(truncate("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_exact_boundary() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_redact_short() {
        assert_eq!(redact("secret"), "******");
    }

    #[test]
    fn test_redact_long() {
        assert_eq!(redact("secretkey123"), "secr...****");
    }

    #[test]
    fn test_redact_exactly_eight() {
        // 8 chars → all stars
        assert_eq!(redact("12345678"), "********");
    }

    #[test]
    fn test_pluralize_singular() {
        assert_eq!(pluralize(1, "file", "files"), "1 file");
    }

    #[test]
    fn test_pluralize_plural() {
        assert_eq!(pluralize(2, "file", "files"), "2 files");
    }

    #[test]
    fn test_pluralize_zero() {
        assert_eq!(pluralize(0, "file", "files"), "0 files");
    }

    #[test]
    fn edit_distance_counts_a_swap_as_one() {
        assert_eq!(edit_distance("redis", "redis"), 0);
        assert_eq!(edit_distance("rdeis", "redis"), 1);
        assert_eq!(edit_distance("strpe", "stripe"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn closest_keeps_the_candidate_order_and_maps_aliases() {
        let names = [("upper", "upper"), ("lower", "lower"), ("pg", "postgresql")];
        assert_eq!(closest("uper", names.into_iter()), vec!["upper"]);
        assert_eq!(closest("pq", names.into_iter()), vec!["postgresql"]);
        assert!(closest("json", names.into_iter()).is_empty());
    }
}
