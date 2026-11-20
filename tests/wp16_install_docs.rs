//! WP-16: the install URL, the stability page and the docs examples, checked
//! against the code.
//!
//! * One install URL (`core::build::INSTALL_URL`) in `src`, the README and the
//!   docs: no other spelling of the script's address.
//! * `docs/STABILITY.md` lists every headline of the CHANGELOG's "Breaking"
//!   section for this release, so the two cannot drift apart.
//! * Every `evnx …` command line in a bash block of the README and the guide
//!   names a command, and only flags, that the built binary knows. The check
//!   reads `evnx commands --json` and each command's `--help`, so a flag renamed
//!   or removed fails here until the docs say so.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

// ── One install URL ──────────────────────────────────────────────────────────

#[test]
fn wp16_one_install_url_everywhere() {
    let url = evnx::core::build::INSTALL_URL;
    assert_eq!(url, "https://evnx.dev/install.sh");

    let mut files = Vec::new();
    files_under(&root().join("src"), &mut files);
    files_under(&root().join("docs"), &mut files);
    for f in [
        "README.md",
        "pypi-readme.md",
        ".github/workflows/release.yml",
    ] {
        files.push(root().join(f));
    }

    let mut bad = Vec::new();
    for f in files {
        // Past release notes are history, and this file names the old spellings.
        let rel = f
            .strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("docs/releases/")
            || rel == "docs/HISTORY.md"
            || rel.ends_with("wp16_install_docs.rs")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let other_spelling = line.contains("dotenv.space/install")
                || (line.contains("raw.githubusercontent.com") && line.contains("install.sh"))
                || line.contains("evnx.dev/install\"")
                || line.contains("evnx.dev/install)")
                || line.contains("evnx.dev/install ")
                || line.ends_with("evnx.dev/install");
            // The old host may be named in a redirect remark, and in the release
            // runbook, which checks that dotenv.space still serves the same file.
            if other_spelling
                && !line.to_lowercase().contains("redirect")
                && rel != "docs/RELEASING.md"
            {
                bad.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "another install address:\n{}",
        bad.join("\n")
    );

    // The spellings that say the script's address agree with the constant.
    for rel in ["README.md", "docs/GETTING_STARTED.md", "pypi-readme.md"] {
        assert!(
            read(rel).contains(&format!("curl -fsSL {url} | bash")),
            "{rel} does not show the install command"
        );
    }
}

// ── STABILITY.md equals the CHANGELOG's Breaking headlines ───────────────────

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn wp16_stability_lists_every_breaking_headline() {
    let changelog = read("CHANGELOG.md");
    // The newest released section that has a "Breaking" part. `## [Unreleased]`
    // is skipped: its entries move into STABILITY.md when the release is cut.
    let unreleased = changelog
        .split("\n## [")
        .skip(1)
        .filter(|section| !section.starts_with("Unreleased]"))
        .find(|section| section.contains("### Breaking\n"))
        .expect("a CHANGELOG section with a Breaking part");
    let breaking = unreleased
        .split("### Breaking\n")
        .nth(1)
        .and_then(|s| s.split("\n### ").next())
        .expect("a Breaking section");

    // `- **headline**` at the start of an item, which may span lines.
    let mut headlines = Vec::new();
    let mut rest = breaking;
    while let Some(i) = rest
        .find("\n- **")
        .or_else(|| rest.strip_prefix("- **").map(|_| 0))
    {
        let after = &rest[i..];
        let start = after.find("**").unwrap() + 2;
        let end = after[start..].find("**").unwrap() + start;
        headlines.push(squash(&after[start..end]));
        rest = &after[end + 2..];
    }
    assert!(
        // The 0.15.0 section has 20+; an `[Unreleased]` section starts small.
        !headlines.is_empty(),
        "found only {} headlines",
        headlines.len()
    );

    let stability = squash(&read("docs/STABILITY.md"));
    let missing: Vec<_> = headlines
        .iter()
        .filter(|h| !stability.contains(&format!("**{h}**")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/STABILITY.md lacks these Breaking headlines from the CHANGELOG: {missing:#?}"
    );
}

// ── Doc examples name real commands and flags ────────────────────────────────

fn evnx(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_evnx"))
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run evnx");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The command lines in ```bash / ```sh blocks that start with `evnx`.
fn example_lines(doc: &str) -> Vec<(usize, String)> {
    let mut lines = Vec::new();
    let mut in_block = false;
    let mut shell = false;
    for (n, line) in doc.lines().enumerate() {
        let t = line.trim_start();
        if let Some(info) = t.strip_prefix("```") {
            if in_block {
                in_block = false;
            } else {
                in_block = true;
                shell = matches!(info.trim(), "bash" | "sh" | "shell" | "");
            }
            continue;
        }
        if !(in_block && shell) {
            continue;
        }
        // Every segment of a pipeline or `&&` chain that starts with evnx.
        for seg in t.split(['|', ';']).flat_map(|s| s.split("&&")) {
            let seg = seg.trim().trim_start_matches('$').trim();
            if seg.starts_with("evnx ") {
                lines.push((n + 1, seg.to_string()));
            }
        }
    }
    lines
}

/// Words of a command line: quotes group, `#` and `>` end it.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                has = true;
            }
            (None, '#') if cur.is_empty() && !has => break,
            (None, '>') | (None, '<') => break,
            (None, '\\') if chars.peek().is_none() => break,
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[test]
fn wp16_docs_examples_name_real_commands_and_flags() {
    let json: serde_json::Value = serde_json::from_str(&evnx(&["commands", "--json"])).unwrap();
    // Path (space-joined) -> the path it is an alias of.
    let mut known: BTreeMap<String, String> = BTreeMap::new();
    for c in json["commands"].as_array().unwrap() {
        let path: Vec<String> = c["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let canon = path.join(" ");
        known.insert(canon.clone(), canon.clone());
        for key in ["aliases", "visible_aliases"] {
            for a in c[key].as_array().into_iter().flatten() {
                let mut p = path.clone();
                *p.last_mut().unwrap() = a.as_str().unwrap().to_string();
                known.insert(p.join(" "), canon.clone());
            }
        }
    }
    // A default build lacks these; their examples are checked in --all-features runs.
    let optional: BTreeSet<&str> = [
        "auth", "org", "vault", "cloud", "platform", "migrate", "backup", "restore",
    ]
    .into();
    let global: BTreeSet<&str> = [
        "--help",
        "--version",
        "--no-color",
        "--quiet",
        "--verbose",
        "--json",
    ]
    .into();

    let mut help: BTreeMap<String, String> = BTreeMap::new();
    let mut bad = Vec::new();
    let mut checked = 0;

    for rel in ["README.md", "docs/GETTING_STARTED.md", "docs/STABILITY.md"] {
        let doc = read(rel);
        for (n, line) in example_lines(&doc) {
            let w = words(&line);
            let mut path: Vec<String> = Vec::new();
            let mut skip = false;
            for tok in w.iter().skip(1) {
                if tok.starts_with('-') || tok.starts_with('<') || tok.starts_with('[') {
                    continue;
                }
                let mut next = path.clone();
                next.push(tok.clone());
                if known.contains_key(&next.join(" ")) {
                    path = next;
                } else if path.is_empty() {
                    if optional.contains(tok.as_str()) {
                        skip = true;
                    } else if tok != "help" {
                        bad.push(format!("{rel}:{n}: unknown command `{tok}` in `{line}`"));
                        skip = true;
                    }
                    break;
                } else {
                    break; // an argument
                }
            }
            if skip || path.is_empty() {
                continue;
            }
            checked += 1;
            let canon = known[&path.join(" ")].clone();
            let text = help.entry(canon.clone()).or_insert_with(|| {
                let mut args: Vec<&str> = canon.split(' ').collect();
                args.push("--help");
                evnx(&args)
            });
            for tok in &w {
                let Some(flag) = tok.strip_prefix("--") else {
                    continue;
                };
                let name = flag.split('=').next().unwrap();
                if name.is_empty() || global.contains(format!("--{name}").as_str()) {
                    continue;
                }
                let needle = format!("--{name}");
                let found = text.match_indices(&needle).any(|(i, _)| {
                    !text[i + needle.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
                });
                if !found {
                    bad.push(format!(
                        "{rel}:{n}: `{needle}` is not a flag of `evnx {canon}`: `{line}`"
                    ));
                }
            }
        }
    }
    assert!(checked >= 60, "only {checked} example lines were checked");
    assert!(
        bad.is_empty(),
        "doc examples out of step with the CLI:\n{}",
        bad.join("\n")
    );
}
