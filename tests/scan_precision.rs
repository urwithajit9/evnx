//! Scan precision fixes (v0.14.0): stateless GitHub tokens, names in
//! Dockerfiles / shell / Python, and the false positives the RM-70 corpus
//! found.
//!
//! ⚠️ Every secret-shaped value is assembled at run time from parts, so the
//! repository holds none.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// One finding: (rule id, line, variable).
type Hit = (String, usize, Option<String>);

/// Scan a project holding `files` (name, text) and return every finding.
fn scan(files: &[(&str, &str)]) -> Vec<Hit> {
    let d = TempDir::new().unwrap();
    for (name, text) in files {
        let p = d.path().join(Path::new(name));
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, text.replace("\r\n", "\n")).unwrap();
    }
    let out = cargo_bin_cmd!("evnx")
        .current_dir(d.path())
        .env("NO_COLOR", "1")
        .args(["scan", "--format", "json", "."])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let loc = f["location"].as_str().unwrap();
            // `./file:LINE (VAR)` or `./file:LINE`
            let after = loc.rsplit_once(':').map_or(loc, |(_, r)| r);
            let line = after
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap()
                .parse()
                .unwrap_or_else(|_| panic!("no line in {loc}"));
            (
                f["rule_id"].as_str().unwrap().to_string(),
                line,
                f["variable"].as_str().map(str::to_string),
            )
        })
        .collect()
}

fn rules(hits: &[Hit]) -> Vec<(&str, usize)> {
    hits.iter().map(|(r, l, _)| (r.as_str(), *l)).collect()
}

// ── SCAN-GHS ─────────────────────────────────────────────────────────────

/// `ghs_<appid>_<header>.<payload>.<signature>`, ~520 characters.
fn stateless() -> String {
    let seg = |n: usize, seed: usize| -> String {
        const A: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        (0..n)
            .map(|i| A[(i * 7 + seed * 13 + i / 5) % A.len()] as char)
            .collect()
    };
    format!(
        "ghs_{}_{}.{}.{}",
        "1234567",
        ["eyJ", &seg(33, 1)].concat(),
        ["eyJ", &seg(330, 2)].concat(),
        seg(140, 3)
    )
}

fn classic() -> String {
    ["ghs_", "r080UUTWjPBOaALo0vrQuGXc22ToCIyNaLQS", "WXYZ"].concat()
}

#[test]
fn stateless_installation_token_is_one_github_token_finding() {
    let t = stateless();
    assert!(t.len() > 500);
    let hits = scan(&[("ci.sh", &format!("echo start\nexport X={t}\n"))]);
    assert_eq!(rules(&hits), vec![("github-token", 2)], "{hits:?}");
}

#[test]
fn stateless_token_in_a_dotenv_is_not_also_a_jwt_or_entropy_hit() {
    let t = stateless();
    let hits = scan(&[(".env", &format!("GITHUB_TOKEN={t}\nNOTE=\"{t}\"\n"))]);
    // One finding per value, never a jwt or high-entropy-string beside it.
    assert_eq!(hits.len(), 2, "{hits:?}");
    for (rule, _, _) in &hits {
        assert!(rule != "jwt" && rule != "high-entropy-string", "{hits:?}");
    }
}

#[test]
fn trailing_punctuation_is_not_part_of_a_stateless_token() {
    let rule = evnx::core::secretscan::rules::builtin_rules()
        .into_iter()
        .find(|r| r.id.as_deref() == Some("github-token"))
        .unwrap();
    let re = regex::Regex::new(&rule.regex).unwrap();
    let t = stateless();
    for tail in [".", ",", ")", ";", "\""] {
        let text = format!("token ({t}{tail} done");
        let m = re
            .find(&text)
            .unwrap_or_else(|| panic!("no match before {tail:?}"));
        assert_eq!(m.as_str(), t, "swallowed {tail:?}");
    }
    // The classic form is untouched by the new branch.
    let c = classic();
    assert_eq!(re.find(&format!("({c}).")).unwrap().as_str(), c);
}

#[test]
fn classic_forty_char_form_still_matches() {
    let c = classic();
    assert_eq!(c.len(), 44);
    let hits = scan(&[(".env", &format!("GH={c}\n"))]);
    assert_eq!(rules(&hits), vec![("github-token", 1)], "{hits:?}");
}
