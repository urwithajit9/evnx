//! Every `` `evnx …` `` quoted in a message names a command that exists, with
//! flags it has (T-SHARE-49, DEC-AU4, A-41).
//!
//! Two hints shipped pointing at things that do not run: `evnx auth totp
//! backup-codes` (it is `recovery-codes`, I-202) and, twice, `evnx auth
//! sessions` — a group, which prints help (I-208); a unit test even asserted
//! the wrong one. This reads the real command tree from `evnx commands --json`
//! and every backtick-quoted `evnx …` in `src/` outside comments and tests.
//!
//! Rules: the leading words must be a command path; if that path is a group,
//! a subcommand must follow; each `--flag` must exist on the command (or be a
//! global flag). Words in `<…>` or `{…}` are placeholders.
#![cfg(all(feature = "cloud", feature = "migrate", feature = "backup"))]

mod support;

use std::collections::{HashMap, HashSet};
use std::path::Path;

const GLOBAL_FLAGS: &[&str] = &["quiet", "verbose", "no-color", "help", "version", "server"];

fn is_word(w: &str) -> bool {
    let mut c = w.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase())
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn t_share_49_quoted_commands_exist() {
    let mut flags: HashMap<Vec<String>, HashSet<String>> = HashMap::new();
    for c in support::surface::entries() {
        let longs = c
            .args()
            .iter()
            .filter_map(|a| a["long"].as_str().map(String::from))
            .collect();
        flags.insert(c.path, longs);
    }
    let is_group = |p: &[String]| {
        flags
            .keys()
            .any(|q| q.len() > p.len() && q[..p.len()] == *p)
    };

    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    let mut bad = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        // Tests may quote anything; they sit after the first `#[cfg(test)]`.
        let text = text.split("#[cfg(test)]").next().unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for quoted in line.split('`').skip(1).step_by(2) {
                let Some(rest) = quoted.strip_prefix("evnx ") else {
                    continue;
                };
                let words: Vec<&str> = rest.split_whitespace().collect();
                let mut path: Vec<String> = Vec::new();
                for w in &words {
                    let mut next = path.clone();
                    next.push((*w).to_string());
                    if is_word(w) && flags.contains_key(&next) {
                        path = next;
                    } else {
                        break;
                    }
                }
                let at = format!("{}:{}: `evnx {rest}`", f.display(), n + 1);
                if path.is_empty() {
                    if !words.first().is_some_and(|w| w.starts_with(['<', '{'])) {
                        bad.push(format!("{at} — no such command"));
                    }
                    continue;
                }
                let after = &words[path.len()..];
                if is_group(&path) && after.first().is_none_or(|w| is_word(w)) {
                    bad.push(format!(
                        "{at} — `{}` is a group; name a subcommand",
                        path.join(" ")
                    ));
                }
                for w in after.iter().filter(|w| w.starts_with("--")) {
                    let flag = w[2..].split('=').next().unwrap();
                    let flag = flag.trim_end_matches(['.', ',', ';', ':', ')']);
                    if is_word(flag)
                        && !GLOBAL_FLAGS.contains(&flag)
                        && !flags[&path].contains(flag)
                    {
                        bad.push(format!("{at} — `{}` has no --{flag}", path.join(" ")));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "quoted commands that do not run:\n{}",
        bad.join("\n")
    );
}
