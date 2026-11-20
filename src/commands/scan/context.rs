//! Usage hints and test paths, without a model (SCAN-CONTEXT).
//!
//! * A value on a line that passes it to an `Authorization`/`Bearer` header, an
//!   HTTP client call or a credentialed connection string is being **used**,
//!   which is more likely a real credential: confidence goes up one level.
//! * A file under a `test`, `tests`, `mock`, `spec` or `fixtures` folder is
//!   more likely holding a stand-in: confidence goes down one level.
//!
//! ⛔ Neither ever removes a finding, and neither changes what fails a build
//! (D9). The `--confidence` floor, the counts and the exit code all use the
//! confidence from **before** the hint; only the level shown, the sort and the
//! JSON `context` move. And only name, entropy and medium findings are
//! adjusted: a fixed-prefix rule that is high stays high in a test folder, so
//! a real key committed under `tests/` still reads as one.

use crate::core::secretscan::Confidence;
use regex::Regex;
use std::sync::LazyLock;

/// A hint's name in the JSON `context` list.
pub const USAGE: &str = "usage";
pub const TEST_PATH: &str = "test-path";

static USAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ix)
        \bauthorization['"]?\s*[:=]
        | \bbearer\s+[\w.~+/=$\-{}]{8,}
        | \bx-api-key\s*[:=]
        | \b(?:curl|wget)\s
        | \b(?:requests|httpx|axios|got|http|https|session|client)\.(?:get|post|put|patch|delete|head|request)\s*\(
        | \bfetch\s*\(
        | \bhttp\.NewRequest\b
        | \bInvoke-(?:RestMethod|WebRequest)\b
        | \b[a-z][a-z0-9+.\-]*://[^\s/:@]+:[^\s/@]+@
        | \b(?:server|host|data\ source)=[^;\n]+;.*\b(?:password|pwd)=
        "#,
    )
    .expect("the usage expression compiles")
});

/// Whether `line` passes a value on: an auth header, an HTTP client call or a
/// connection string with credentials.
#[must_use]
pub fn is_usage(line: &str) -> bool {
    USAGE_RE.is_match(line)
}

/// Whether `path` has a `test`, `tests`, `mock`, `spec` or `fixtures` folder
/// (any case, either separator). The file's own name does not count.
#[must_use]
pub fn is_test_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    let mut parts: Vec<&str> = norm.split('/').collect();
    parts.pop();
    parts.iter().any(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "test" | "tests" | "mock" | "spec" | "fixtures"
        )
    })
}

/// Whether a finding is one the hints may move: judged by name or entropy, or
/// medium. A fixed-prefix rule at high confidence is not.
fn adjustable(rule_id: &str, confidence: Confidence) -> bool {
    confidence == Confidence::Medium
        || rule_id == crate::core::secretscan::detector::SENSITIVE_KEY_ID
        || rule_id == "high-entropy-string"
}

fn step(c: Confidence, by: i8) -> Confidence {
    match (c, by) {
        (Confidence::Low, 1) => Confidence::Medium,
        (Confidence::Medium, 1) => Confidence::High,
        (Confidence::High, -1) => Confidence::Medium,
        (Confidence::Medium, -1) => Confidence::Low,
        (c, _) => c,
    }
}

/// The level to show, and the hints that applied (empty when none did or the
/// finding is not one the hints move).
#[must_use]
pub fn adjust(
    rule_id: &str,
    confidence: Confidence,
    usage: bool,
    test_path: bool,
) -> (Confidence, Vec<&'static str>) {
    if !adjustable(rule_id, confidence) {
        return (confidence, Vec::new());
    }
    let mut hints = Vec::new();
    let mut level = confidence;
    if usage {
        hints.push(USAGE);
        level = step(level, 1);
    }
    if test_path {
        hints.push(TEST_PATH);
        level = step(level, -1);
    }
    (level, hints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_is_an_auth_header_a_client_call_or_a_connection_string() {
        for l in [
            r#"curl -H "Authorization: Bearer abc" https://x"#,
            "headers = {'Authorization': token}",
            "requests.get(url, headers=h)",
            "const r = await fetch(url, { headers })",
            "DATABASE_URL=postgres://u:p@host/db",
            "Server=db;Database=app;User Id=sa;Password=x;",
            "Server=db;Password=x;",
        ] {
            assert!(is_usage(l), "{l}");
        }
        for l in [
            "API_KEY=abc",
            "# the bearer of news",
            "see https://example.com/docs",
        ] {
            assert!(!is_usage(l), "{l}");
        }
    }

    #[test]
    fn a_test_path_has_a_test_folder_not_a_test_file() {
        for p in [
            "tests/a.env",
            "./src/test/x.py",
            "app\\spec\\a.rb",
            "a/Fixtures/b.json",
            "mock/x",
        ] {
            assert!(is_test_path(p), "{p}");
        }
        for p in [
            "src/latest/a.env",
            "test.env",
            "a/contest/b",
            "attest/x",
            "src/app.test.ts",
        ] {
            assert!(!is_test_path(p), "{p}");
        }
    }

    #[test]
    fn only_name_entropy_and_medium_findings_move_one_level() {
        let name = crate::core::secretscan::detector::SENSITIVE_KEY_ID;
        assert_eq!(
            adjust(name, Confidence::High, false, true),
            (Confidence::Medium, vec![TEST_PATH])
        );
        assert_eq!(
            adjust("high-entropy-string", Confidence::Low, true, false),
            (Confidence::Medium, vec![USAGE])
        );
        assert_eq!(
            adjust("jwt", Confidence::Medium, false, true),
            (Confidence::Low, vec![TEST_PATH])
        );
        // Both hints cancel, and both are named.
        assert_eq!(
            adjust(name, Confidence::Medium, true, true),
            (Confidence::Medium, vec![USAGE, TEST_PATH])
        );
        // A high fixed-prefix rule is never lowered; the floor holds at the ends.
        assert_eq!(
            adjust("aws-access-key", Confidence::High, false, true),
            (Confidence::High, vec![])
        );
        assert_eq!(
            adjust("jwt", Confidence::Medium, false, true).0,
            Confidence::Low
        );
        assert_eq!(
            adjust(name, Confidence::Low, false, true).0,
            Confidence::Low
        );
    }
}
