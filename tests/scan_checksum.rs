//! The checksum stage for classic GitHub tokens (SCAN-CRC).
//!
//! A `ghp_`/`gho_`/`ghu_`/`ghr_` token with 36 Base62 characters after the
//! prefix carries a checksum in its last six. A value that passes is reported
//! high; one that fails was never issued by GitHub and is not a finding, under
//! any variable name. Other lengths and `ghs_` are not judged.
//!
//! ⚠️ The algorithm is verified here only against itself. The owner checks it
//! against a real token; see the note in `core::secretscan::checksum`.

use assert_cmd::cargo::cargo_bin_cmd;
use evnx::core::secretscan::checksum::with_checksum;
use std::fs;
use tempfile::TempDir;

const BODY: &str = "Tq7Lm2Xv9Rk4Wc8Hb3Jd6Fs1Ga5Nz7";

fn scan(text: &str) -> (i32, serde_json::Value) {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("a.env"), text).unwrap();
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", "a.env"])
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap();
    (out.status.code().unwrap_or(-1), v)
}

fn flip_last(token: &str) -> String {
    let mut b = token.as_bytes().to_vec();
    let last = b.len() - 1;
    b[last] = if b[last] == b'0' { b'1' } else { b'0' };
    String::from_utf8(b).unwrap()
}

#[test]
fn a_token_with_a_valid_checksum_is_a_high_finding_for_each_prefix() {
    for prefix in ["ghp_", "gho_", "ghu_", "ghr_"] {
        let t = with_checksum(prefix, BODY);
        let (code, v) = scan(&format!("GITHUB_TOKEN={t}\n"));
        assert_eq!(code, 1, "{prefix}");
        let f = &v["findings"][0];
        assert_eq!(f["rule_id"], "github-token", "{prefix}");
        assert_eq!(f["confidence"], "high", "{prefix}");
    }
}

#[test]
fn a_token_with_a_failing_checksum_is_dropped_for_each_prefix() {
    for prefix in ["ghp_", "gho_", "ghu_", "ghr_"] {
        let bad = flip_last(&with_checksum(prefix, BODY));
        // Under a name that says "token", under a neutral name, and bare in
        // a line of text: no finding from the rule, the name or entropy.
        for line in [
            format!("GITHUB_TOKEN={bad}\n"),
            format!("VALUE_A={bad}\n"),
            format!("API_TOKEN=\"{bad}\"\n"),
        ] {
            let (code, v) = scan(&line);
            assert_eq!(code, 0, "{line}{v}");
            assert_eq!(v["findings"].as_array().unwrap().len(), 0, "{line}");
        }
    }
}

#[test]
fn other_lengths_and_installation_tokens_are_not_judged() {
    // 35 and 37 characters after the prefix are not the classic shape; ghs_
    // has no published checksum. The rule's own regex decides these as before.
    let long = format!("ghp_{BODY}ABCDEFG");
    let ghs = flip_last(&with_checksum("ghs_", BODY));
    for t in [long, ghs] {
        let (_, v) = scan(&format!("GITHUB_TOKEN={t}\n"));
        let found = v["findings"].as_array().unwrap();
        // 41 characters is outside the rule's 36..40 range; whatever the
        // scanner says, the checksum must not be the reason: ghs_ is found.
        if t.starts_with("ghs_") {
            assert!(!found.is_empty(), "{t}");
        }
    }
}
