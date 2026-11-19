//! What the words say (WP-10): the grouped command list, help that reads as
//! help, usage errors that stand alone, and the banner that follows the
//! arguments.

mod support;

use serde_json::Value;
use support::surface::{entries, payload};
use support::{Project, run};

/// Every command path of the build, from `evnx commands --json`.
fn paths() -> Vec<Vec<String>> {
    entries().into_iter().map(|e| e.path).collect()
}

/// `--help` of the root and of every command, as `(name, text)`.
/// The groups this build has a command in: a build without `net` has nothing
/// in "Cloud", and the heading is not printed.
fn groups_present() -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for c in payload()["commands"].as_array().unwrap() {
        if let Some(g) = c["group"].as_str() {
            if !seen.iter().any(|s| s == g) {
                seen.push(g.to_string());
            }
        }
    }
    seen
}

fn every_help() -> Vec<(String, String)> {
    let p = Project::new();
    let mut all = vec![("evnx".to_string(), run(p.path(), &["--help"]).stdout)];
    for path in paths() {
        let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
        args.push("--help");
        let out = run(p.path(), &args);
        out.assert_code(0);
        all.push((format!("evnx {}", path.join(" ")), out.stdout));
    }
    all
}

/// Words that look like a tracker id but are standards or examples.
const NOT_TRACKER_IDS: &[&str] = &["UTF-8", "ML-KEM-768", "KEM-768", "ACME-"];

/// A tracker id: `GEN-4`, `DEC-CL8`, `I-232`, `RM-34`, or `(D5, D6)`.
fn tracker_ids(text: &str) -> Vec<String> {
    let re =
        regex::Regex::new(r"\b[A-Z]{1,6}-[A-Z]*\d+\b|\(\s*[A-Z]{1,3}\d+(,\s*[A-Z]{1,3}\d+)*\s*\)")
            .unwrap();
    re.find_iter(text)
        // `[A-Z0-9]` in a pattern is a character class, not an id.
        .filter(|m| !text[..m.start()].ends_with('['))
        .map(|m| m.as_str().to_string())
        .filter(|m| !NOT_TRACKER_IDS.iter().any(|ok| m.contains(ok)))
        .collect()
}

#[test]
fn no_help_text_carries_a_tracker_id() {
    let mut found = Vec::new();
    for (name, text) in every_help() {
        for id in tracker_ids(&text) {
            found.push(format!("{name}: {id}"));
        }
    }
    assert!(
        found.is_empty(),
        "tracker ids in --help:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_tracker_id_pattern_catches_what_it_is_for() {
    for bad in ["GEN-4", "DEC-CL8", "I-232", "RM-34", "(RM-34, D5, D6)"] {
        assert!(!tracker_ids(&format!("see {bad} here")).is_empty(), "{bad}");
    }
    assert!(tracker_ids("UTF-8 and ML-KEM-768 and 'ACME-[A-Z]'").is_empty());
}

#[test]
fn no_help_text_has_raw_markdown_or_emoji() {
    let emoji = |c: char| {
        matches!(c, '\u{2190}'..='\u{21ff}' | '\u{2600}'..='\u{27bf}' | '\u{1F000}'..='\u{1FFFF}' | '\u{FE0F}')
            && !matches!(c, '→' | '↔' | '←')
    };
    let mut found = Vec::new();
    for (name, text) in every_help() {
        let plain = text.replace("**/", "").replace("/**", "");
        if plain.contains("**") && !plain.contains("***") {
            found.push(format!("{name}: **bold**"));
        }
        if let Some(c) = text.chars().find(|c| emoji(*c)) {
            found.push(format!("{name}: {c:?}"));
        }
    }
    assert!(
        found.is_empty(),
        "markdown or emoji in --help:\n{}",
        found.join("\n")
    );
}

#[test]
fn help_names_the_current_spelling_of_things() {
    let p = Project::new();
    let history = run(p.path(), &["history", "--help"]).stdout;
    assert!(!history.contains("diff --apply"), "{history}");
    assert!(
        history.contains("diff --format patch --interactive"),
        "{history}"
    );

    let update = run(p.path(), &["update", "--help"]).stdout;
    for channel in [
        "cargo",
        "Homebrew",
        "Scoop",
        "winget",
        "npm",
        "pip",
        "pipx",
        "install script",
    ] {
        assert!(
            update.contains(channel),
            "update --help lacks {channel}:\n{update}"
        );
    }
}

#[test]
fn the_command_list_is_in_four_groups_from_the_one_table() {
    let p = Project::new();
    let help = run(p.path(), &["--help"]).stdout;
    let at = |s: &str| {
        help.find(s)
            .unwrap_or_else(|| panic!("no {s:?} in\n{help}"))
    };
    let present = groups_present();
    let order: Vec<String> = ["Local files", "Convert and run", "Cloud", "This tool"]
        .iter()
        .filter(|g| present.iter().any(|p| p == *g))
        .map(|g| format!("{g}:"))
        .chain(["Start here:".to_string(), "Options:".to_string()])
        .collect();
    for pair in order.windows(2) {
        assert!(
            at(&pair[0]) < at(&pair[1]),
            "{pair:?} out of order:\n{help}"
        );
    }
    // Every top-level command sits under the group `commands --json` names.
    let payload = payload();
    for c in payload["commands"].as_array().unwrap() {
        if c["path"].as_array().unwrap().len() != 1 || c["hidden"] == true {
            continue;
        }
        let name = c["name"].as_str().unwrap();
        let group = c["group"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} is in no group"));
        let section = &help[at(&format!("{group}:"))..];
        let section = section.split("\n\n").next().unwrap();
        assert!(
            section
                .lines()
                .any(|l| l.trim_start().starts_with(&format!("{name} "))),
            "{name} is not listed under {group}:\n{help}"
        );
    }
    // The three-line "Start here" block.
    let start = help[at("Start here:")..].split("\n\n").next().unwrap();
    assert_eq!(start.lines().count(), 4, "{start}");
}

#[test]
fn the_groups_are_the_ones_the_owner_named() {
    let payload = payload();
    // A build without a feature lacks some of these commands; the ones it has
    // are in the group the owner named.
    let group_of = |name: &str| {
        payload["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["path"] == serde_json::json!([name]))
            .map(|c| c["group"].as_str().unwrap_or("").to_string())
    };
    let mut checked = 0;
    for (group, names) in [
        (
            "Local files",
            &[
                "init", "add", "set", "get", "remove", "validate", "sync", "align", "diff", "scan",
                "doctor",
            ][..],
        ),
        (
            "Convert and run",
            &[
                "convert", "template", "run", "migrate", "parse", "spec", "backup", "restore",
                "undo", "history",
            ][..],
        ),
        ("Cloud", &["auth", "org", "vault", "cloud", "platform"][..]),
        ("This tool", &["update", "completions", "commands"][..]),
    ] {
        for n in names {
            if let Some(got) = group_of(n) {
                assert_eq!(got, group, "{n}");
                checked += 1;
            }
        }
    }
    assert!(checked >= 15, "only {checked} commands checked");
}

#[test]
fn commands_json_says_what_needs_the_network() {
    let payload = payload();
    let mut seen = std::collections::BTreeMap::new();
    for c in payload["commands"].as_array().unwrap() {
        let net = c["network"]
            .as_str()
            .unwrap_or_else(|| panic!("{} has no network", c["name"]));
        assert!(
            ["local", "local-with-network", "account"].contains(&net),
            "{net}"
        );
        if c["path"].as_array().unwrap().len() == 1 {
            seen.insert(c["name"].as_str().unwrap().to_string(), net.to_string());
        }
    }
    for (name, want) in [
        ("validate", "local"),
        ("scan", "local"),
        ("auth", "account"),
        ("cloud", "account"),
        ("migrate", "local-with-network"),
        ("update", "local-with-network"),
    ] {
        // A build without the feature lacks the command.
        if let Some(got) = seen.get(name) {
            assert_eq!(got, want, "{name}");
        }
    }
    // A subcommand has its command's value.
    let nested = payload["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == serde_json::json!(["scan", "rules"]))
        .expect("scan rules");
    assert_eq!(nested["network"], Value::from("local"));
}

#[test]
fn the_plain_commands_listing_uses_the_same_groups() {
    let p = Project::new();
    let out = run(p.path(), &["commands"]);
    out.assert_code(0);
    for title in groups_present() {
        assert!(
            out.stdout.contains(&format!("  {title}\n")),
            "{}",
            out.stdout
        );
    }
    assert!(!out.stdout.contains("On this machine"), "{}", out.stdout);
}

// ── Usage errors, footers, and the banner ───────────────────────────────────

#[test]
fn a_usage_error_has_no_footer() {
    let p = Project::new().env("A=1\n").example("A=\n");
    for args in [
        &["convert"][..],
        &["init", "--blueprint", "nosuch", "--yes", "--dry-run"][..],
        &["init", "--with", "nosuch", "--yes", "--dry-run"][..],
        &["backup", "missing.env"][..],
        &["migrate", "--to", "heroku"][..],
        &["convert", "--to", "json", "--prefix", "a-b"][..],
    ] {
        let out = run(p.path(), args);
        out.assert_code(2);
        out.assert_not_contains("No verdict");
        out.assert_not_contains("This is not a clean result");
    }
}

#[test]
fn a_failed_run_says_why_there_is_no_verdict() {
    let p = Project::new().env("A=1\n").example("A=\n");
    let out = run(p.path(), &["validate", "--env", "missing.env"]);
    out.assert_code(2);
    out.assert_contains("No verdict: a file evnx needed was not found.");
    out.assert_not_contains("did not finish");
}

#[test]
fn arguments_are_checked_before_the_banner() {
    let p = Project::new().env("A=1\n").example("A=\n");
    for (args, banner) in [
        (&["validate", "--env", "missing.env"][..], "evnx validate"),
        (
            &["validate", "--example", "missing.example"][..],
            "evnx validate",
        ),
        (
            &["init", "--blueprint", "nosuch", "--yes", "--dry-run"][..],
            "evnx init",
        ),
        (&["backup", "missing.env"][..], "evnx backup"),
        (
            &["backup", ".env", "--password-file", "missing.key"][..],
            "evnx backup",
        ),
        (&["restore", "missing.backup"][..], "evnx restore"),
        (&["scan", "missing-folder"][..], "evnx scan"),
        (
            &["template", "--input", "missing.tpl", "--output", "out"][..],
            "Generate config",
        ),
        (&["migrate", "--to", "github"][..], "evnx migrate"),
    ] {
        let out = run(p.path(), args);
        out.assert_code(2);
        assert!(
            !out.all().contains(banner),
            "{args:?} printed its banner before refusing:\n{}",
            out.all()
        );
    }
}

#[test]
fn convert_without_a_terminal_lists_github_not_github_actions() {
    let p = Project::new().env("A=1\n");
    let out = run(p.path(), &["convert"]);
    out.assert_code(2);
    let available = out
        .stderr
        .lines()
        .find(|l| l.contains("Available:"))
        .expect("an Available line");
    assert!(available.contains("github,"), "{available}");
    assert!(!available.contains("github-actions"), "{available}");
}

#[test]
fn remove_leaves_commands_to_copy_for_what_is_still_present() {
    let p = Project::new().env("A=1\nB=2\n").example("A=\nB=\n");
    let out = run(p.path(), &["remove", "B", "--yes"]);
    out.assert_code(0);
    out.assert_contains("evnx remove B --template");
    out.assert_not_contains("add --template");
}
